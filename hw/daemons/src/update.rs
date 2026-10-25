//! System updates (`nibos-update`, `docs/design/updates.md`): a release's manifest and its
//! minisign signature, which releases a device may take, and writing a release's root file
//! system to the slot that isn't running, chunk by chunk, each checked as it arrives and read
//! back after it is written, resumable at the first chunk not yet done.
//!
//! The device I/O that needs the hardware (the boot control block, restarting) is the binary's;
//! everything here works on any file, so it is tested on a computer.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// The manifest format this code reads.
pub const FORMAT: u32 = 1;

/// A release, as its manifest describes it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub build: String,
    pub board: String,
    pub edition: String,
    pub channel: String,
    /// The oldest version that can take this release directly.
    #[serde(default)]
    pub min_from: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    /// What's new, a few lines, shown on the device.
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub components: BTreeMap<String, String>,
    pub payload: Payload,
    pub root: Root,
    pub fit: Fits,
    /// The state format the release's slot expects (its `migrate.d` scripts bring older
    /// formats up to it).
    #[serde(default)]
    pub migrate: u32,
}

/// The payload file: the root's chunks and both FITs, back to back.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Payload {
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// The root file system image, as chunks of `chunk` bytes (the last one shorter), each
/// compressed with zstd on its own.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Root {
    pub size: u64,
    pub sha256: String,
    pub chunk: u64,
    pub chunks: Vec<Chunk>,
}

/// One chunk: where it is in the payload (compressed), and the hashes of both forms.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Chunk {
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
    pub raw_sha256: String,
}

/// A part of the payload stored as it is (a FIT).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Piece {
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
}

/// Slot A's FIT and slot B's: the same kernel and device tree, each naming its own root.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Fits {
    pub a: Piece,
    pub b: Piece,
}

/// What can go wrong, in words a person can act on, each with a stable code (status.json's
/// `code`, which DSlot words for itself; docs/design/updates.md lists them).
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Manifest(String),
    Signature(String),
    NotForThis(String),
    Refused {
        code: &'static str,
        msg: String,
    },
    /// A chunk or piece whose bytes don't match the manifest.
    Corrupt(String),
    /// A chunk read back from the card differs from what was written.
    CardFailed(String),
    Cancelled,
}

impl Error {
    pub fn refused(code: &'static str, msg: impl Into<String>) -> Error {
        Error::Refused {
            code,
            msg: msg.into(),
        }
    }

    /// The stable code for status.json.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Io(e) => e
                .get_ref()
                .and_then(|e| e.downcast_ref::<NetError>())
                .map_or("io_error", |n| n.code),
            Error::Manifest(_) => "bad_manifest",
            Error::Signature(_) => "bad_signature",
            Error::NotForThis(_) => "not_for_device",
            Error::Refused { code, .. } => code,
            Error::Corrupt(_) => "damaged",
            Error::CardFailed(_) => "card_error",
            Error::Cancelled => "stopped",
        }
    }

    /// curl's own words, for the journal only.
    pub fn raw(&self) -> Option<&str> {
        match self {
            Error::Io(e) => e
                .get_ref()
                .and_then(|e| e.downcast_ref::<NetError>())
                .map(|n| n.raw.as_str()),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Manifest(m) => write!(f, "the release's manifest: {m}"),
            Error::Signature(m) => write!(f, "the release's signature: {m}"),
            Error::NotForThis(m) => write!(f, "not for this device: {m}"),
            Error::Refused { msg, .. } => write!(f, "{msg}"),
            Error::Corrupt(m) => write!(f, "the download is damaged: {m}"),
            Error::CardFailed(m) => write!(f, "{m}"),
            Error::Cancelled => write!(f, "stopped"),
        }
    }
}

/// A download that failed: a stable code and a sentence for a person; curl's own text is kept
/// for the journal ([`Error::raw`]).
#[derive(Debug)]
pub struct NetError {
    pub code: &'static str,
    pub detail: String,
    pub raw: String,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for NetError {}

/// What a failed curl run means, from its exit status and the HTTP status it saw (0: none).
/// The HTTP status decides when there is one: under HTTP/2 a 404 can end in exit 56, not 22.
pub fn classify(exit: Option<i32>, http: u16, url: &str) -> (&'static str, String) {
    match http {
        404 | 410 => (
            "not_found",
            "nothing is published at the update address".into(),
        ),
        429 => (
            "rate_limited",
            "the update server is busy: try again later".into(),
        ),
        // GitHub's API answers a spent rate limit with 403.
        403 if url.contains("api.github.com") => (
            "rate_limited",
            "the update server is busy: try again later".into(),
        ),
        500..=599 => (
            "server_error",
            format!("the update server had a problem (HTTP {http}): try again later"),
        ),
        400..=599 => (
            "http_error",
            format!("the update server refused the request (HTTP {http})"),
        ),
        _ => match exit {
            // Couldn't resolve, couldn't connect, timed out, the connection dropped or no reply.
            Some(5 | 6 | 7 | 28 | 52 | 55 | 56) => (
                "no_network",
                "no connection to the update server: check Wi-Fi".into(),
            ),
            // A certificate it can't check: often the clock (no network time yet).
            Some(35 | 58 | 59 | 60 | 77 | 83 | 90 | 91) => (
                "tls_error",
                "couldn't make a secure connection to the update server: check the date and time"
                    .into(),
            ),
            Some(n) => ("network_error", format!("the download failed (curl {n})")),
            None => ("network_error", "the download failed".into()),
        },
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The SHA-256 of `data`, in lowercase hex.
pub fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Manifest {
    /// Reads a manifest; only after [`verify`] has checked the bytes it came from.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        let m: Manifest =
            serde_json::from_slice(bytes).map_err(|e| Error::Manifest(e.to_string()))?;
        if m.format != FORMAT {
            return Err(Error::Manifest(format!(
                "format {} (this nibOS reads {FORMAT}): write a newer image",
                m.format
            )));
        }
        if m.root.chunk == 0 || m.root.chunks.len() as u64 != m.root.size.div_ceil(m.root.chunk) {
            return Err(Error::Manifest("its chunks don't cover the root".into()));
        }
        Ok(m)
    }

    /// The raw size of chunk `i`.
    pub fn chunk_len(&self, i: usize) -> u64 {
        let start = i as u64 * self.root.chunk;
        self.root.chunk.min(self.root.size - start)
    }

    /// The FIT for `slot` ('A' or 'B').
    pub fn fit(&self, slot: char) -> &Piece {
        if slot == 'B' {
            &self.fit.b
        } else {
            &self.fit.a
        }
    }
}

/// Checks `manifest` (the bytes) against its minisign signature and the public key (the
/// `.pub` file's contents, or just its base64 line).
pub fn verify(manifest: &[u8], signature: &str, public_key: &str) -> Result<()> {
    let key_line = public_key
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(|| Error::Signature("no public key".into()))?;
    let key = minisign_verify::PublicKey::from_base64(key_line)
        .map_err(|e| Error::Signature(format!("the public key: {e}")))?;
    let sig = minisign_verify::Signature::decode(signature)
        .map_err(|e| Error::Signature(format!("unreadable: {e}")))?;
    key.verify(manifest, &sig, false)
        .map_err(|_| Error::Signature("doesn't match: not signed by nibOS's key".into()))
}

/// Compares two versions: `X.Y.Z` numerically, a pre-release (`-alpha2`) before its release,
/// pre-releases by their letters and numbers in order (`alpha1` < `alpha2` < `beta1` < `rc1`).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        (nums, pre.to_owned())
    };
    let (an, ap) = split(a);
    let (bn, bp) = split(b);
    for i in 0..an.len().max(bn.len()) {
        let o = an.get(i).unwrap_or(&0).cmp(bn.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => natural(&ap, &bp),
    }
}

/// Letters compared as text, digit runs as numbers.
fn natural(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, run)) if *d == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    };
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(&pb) {
        let o = if x.0 && y.0 {
            x.1.parse::<u64>()
                .unwrap_or(0)
                .cmp(&y.1.parse::<u64>().unwrap_or(0))
        } else {
            x.1.cmp(&y.1)
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    pa.len().cmp(&pb.len())
}

/// What the running system is (`/usr/lib/nibos/release`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub version: String,
    pub build: String,
    pub board: String,
    pub edition: String,
    pub channel: String,
    /// The image's name after the version: `rgds` or `rgds-dslot`.
    pub image: String,
    /// The GitHub repository releases come from (`owner/name`).
    pub repo: String,
}

impl Release {
    pub fn parse(text: &str) -> Release {
        let env = crate::board::parse_env(text);
        let get = |k: &str| env.get(k).cloned().unwrap_or_default();
        Release {
            version: get("NIBOS_VERSION"),
            build: get("NIBOS_BUILD"),
            board: get("NIBOS_BOARD"),
            edition: get("NIBOS_EDITION"),
            channel: env
                .get("NIBOS_CHANNEL")
                .cloned()
                .unwrap_or_else(|| "stable".into()),
            image: get("NIBOS_IMAGE"),
            repo: get("NIBOS_UPDATE_REPO"),
        }
    }
}

/// Whether this device may take `m`: its board and edition, newer than what runs (unless a
/// downgrade is asked for), and new enough to update from directly.
pub fn applicable(m: &Manifest, running: &Release, allow_downgrade: bool) -> Result<()> {
    if m.board != running.board {
        return Err(Error::NotForThis(format!(
            "it is for {}, this is {}",
            m.board, running.board
        )));
    }
    if m.edition != running.edition {
        return Err(Error::NotForThis(format!(
            "it is the {} edition, this is the {} edition",
            m.edition, running.edition
        )));
    }
    match compare_versions(&m.version, &running.version) {
        Ordering::Greater => {}
        Ordering::Equal if allow_downgrade => {}
        Ordering::Less if allow_downgrade => {}
        Ordering::Equal => {
            return Err(Error::refused(
                "up_to_date",
                format!("{} is running already", m.version),
            ))
        }
        Ordering::Less => {
            return Err(Error::refused(
                "older",
                format!(
                    "{} is older than {} (allow a downgrade to take it)",
                    m.version, running.version
                ),
            ))
        }
    }
    if let Some(min) = &m.min_from {
        if compare_versions(&running.version, min) == Ordering::Less {
            return Err(Error::refused(
                "needs_intermediate",
                format!(
                    "{} updates from {min} or later: update to {min} first, or write the image",
                    m.version
                ),
            ));
        }
    }
    Ok(())
}

/// Where a release's bytes come from: a file, or an address (read with curl's ranges).
pub trait Source {
    fn read(&mut self, offset: u64, len: u64) -> io::Result<Vec<u8>>;
}

/// A payload in a file.
pub struct FileSource(pub File);

impl Source for FileSource {
    fn read(&mut self, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let mut buf = vec![0; len as usize];
        self.0.seek(SeekFrom::Start(offset))?;
        self.0.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// A payload at an address, a range at a time.
pub struct UrlSource(pub String);

impl Source for UrlSource {
    fn read(&mut self, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        fetch(&self.0, Some((offset, len)))
    }
}

/// The line curl writes the HTTP status on, at the end of its stderr.
const HTTP_MARK: &str = "nibos-http=";

/// Reads an address (or a range of it) with curl, which follows redirects (GitHub's release
/// downloads) and retries a dropped connection. A failure is a [`NetError`].
pub fn fetch(url: &str, range: Option<(u64, u64)>) -> io::Result<Vec<u8>> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsSL",
        "--retry",
        "5",
        "--retry-delay",
        "2",
        "--connect-timeout",
        "20",
        "-w",
        &format!("%{{stderr}}\n{HTTP_MARK}%{{http_code}}\n"),
    ]);
    if let Some((offset, len)) = range {
        cmd.args(["-r", &format!("{offset}-{}", offset + len - 1)]);
    }
    let out = cmd.arg(url).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let http = stderr
            .lines()
            .find_map(|l| l.strip_prefix(HTTP_MARK))
            .and_then(|c| c.trim().parse().ok())
            .unwrap_or(0);
        let raw: Vec<&str> = stderr
            .lines()
            .filter(|l| !l.starts_with(HTTP_MARK) && !l.trim().is_empty())
            .collect();
        let (code, detail) = classify(out.status.code(), http, url);
        return Err(io::Error::other(NetError {
            code,
            detail,
            raw: format!("{url}: {}", raw.join(" ")),
        }));
    }
    if let Some((_, len)) = range {
        if out.stdout.len() as u64 != len {
            return Err(io::Error::other(format!(
                "{url}: {} bytes where {len} were asked for",
                out.stdout.len()
            )));
        }
    }
    Ok(out.stdout)
}

/// The chunks already written and read back, kept on the state partition, so that a restart,
/// a sleep or a power cut resumes at the first chunk not done. Tied to one manifest: another
/// release starts afresh.
pub struct Progress {
    path: PathBuf,
    manifest: String,
    done: BTreeSet<usize>,
}

impl Progress {
    pub fn load(path: &Path, manifest_sha256: &str) -> Progress {
        let text = fs::read_to_string(path).unwrap_or_default();
        let mut lines = text.lines();
        let done = if lines.next() == Some(manifest_sha256) {
            lines.filter_map(|l| l.parse().ok()).collect()
        } else {
            BTreeSet::new()
        };
        Progress {
            path: path.to_owned(),
            manifest: manifest_sha256.to_owned(),
            done,
        }
    }

    pub fn is_done(&self, i: usize) -> bool {
        self.done.contains(&i)
    }

    pub fn count(&self) -> usize {
        self.done.len()
    }

    /// Records chunk `i`, synced, written whole by a rename.
    pub fn mark(&mut self, i: usize) -> io::Result<()> {
        self.done.insert(i);
        let mut text = format!("{}\n", self.manifest);
        for d in &self.done {
            text.push_str(&format!("{d}\n"));
        }
        let tmp = self.path.with_extension("tmp");
        let mut f = File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &self.path)
    }
}

/// Reads a range of `target` from the device itself, not the page cache (the read-back).
fn read_back(target: &File, offset: u64, len: u64) -> io::Result<Vec<u8>> {
    // SAFETY: plain syscalls on an open descriptor.
    unsafe {
        libc::posix_fadvise(
            target.as_raw_fd(),
            offset as libc::off_t,
            len as libc::off_t,
            libc::POSIX_FADV_DONTNEED,
        );
    }
    let mut buf = vec![0; len as usize];
    let mut f = target;
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Writes the release's root file system into `target` (the inactive slot's root partition),
/// every chunk not yet in `progress`: fetched, its hash checked, decompressed, written,
/// synced, read back and its hash checked again, then recorded. `cancel` stops it between
/// chunks; `on_chunk(done, total)` reports progress.
pub fn write_root(
    m: &Manifest,
    source: &mut dyn Source,
    target: &mut File,
    progress: &mut Progress,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(usize, usize),
) -> Result<()> {
    let total = m.root.chunks.len();
    if target.seek(SeekFrom::End(0))? < m.root.size {
        return Err(Error::refused(
            "no_space",
            "the root doesn't fit the slot's partition",
        ));
    }
    for (i, c) in m.root.chunks.iter().enumerate() {
        if progress.is_done(i) {
            continue;
        }
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let packed = source.read(c.offset, c.size)?;
        if sha256(&packed) != c.sha256 {
            return Err(Error::Corrupt(format!("chunk {i} as it arrived")));
        }
        let mut raw = Vec::with_capacity(m.chunk_len(i) as usize);
        ruzstd::decoding::StreamingDecoder::new(&packed[..])
            .map_err(|e| Error::Corrupt(format!("chunk {i}: {e}")))?
            .read_to_end(&mut raw)
            .map_err(|e| Error::Corrupt(format!("chunk {i}: {e}")))?;
        if raw.len() as u64 != m.chunk_len(i) || sha256(&raw) != c.raw_sha256 {
            return Err(Error::Corrupt(format!("chunk {i} unpacked")));
        }
        let offset = i as u64 * m.root.chunk;
        target.seek(SeekFrom::Start(offset))?;
        target.write_all(&raw)?;
        target.sync_data()?;
        if sha256(&read_back(target, offset, raw.len() as u64)?) != c.raw_sha256 {
            return Err(Error::CardFailed(format!(
                "chunk {i} read back from the card differs: the card may be failing"
            )));
        }
        progress.mark(i)?;
        on_chunk(progress.count(), total);
    }
    Ok(())
}

/// The FIT for `slot`, fetched and checked against the manifest.
pub fn fetch_fit(m: &Manifest, source: &mut dyn Source, slot: char) -> Result<Vec<u8>> {
    let p = m.fit(slot);
    let bytes = source.read(p.offset, p.size)?;
    if sha256(&bytes) != p.sha256 {
        return Err(Error::Corrupt(format!("slot {slot}'s FIT")));
    }
    Ok(bytes)
}

/// The battery's charge and whether the charger is in, from nibos-hw's links.
pub fn power_ok(battery: &Path, charger: &Path, min_percent: u32) -> Result<()> {
    let read = |p: PathBuf| fs::read_to_string(p).map(|s| s.trim().to_owned());
    let plugged = read(charger.join("online")).is_ok_and(|s| s == "1");
    let level: Option<u32> = read(battery.join("capacity"))
        .ok()
        .and_then(|s| s.parse().ok());
    match level {
        _ if plugged => Ok(()),
        Some(l) if l >= min_percent => Ok(()),
        Some(l) => Err(Error::refused(
            "low_battery",
            format!("the battery is at {l}%: charge it to {min_percent}% or plug the charger in"),
        )),
        None => Ok(()),
    }
}

/// `new_shadow` (an /etc/shadow) with `user`'s password, and when it was last changed, taken
/// from `old_shadow`: what an update carries over from the running root for the one account a
/// person sets a password on. Every other line, and the rest of the user's, stays the new
/// root's, so a release's own system accounts are its own. None if either lacks the user, or
/// nothing would change.
pub fn carry_password(old_shadow: &str, new_shadow: &str, user: &str) -> Option<String> {
    let fields = |text: &str| {
        text.lines()
            .map(|l| l.split(':').map(str::to_owned).collect::<Vec<_>>())
            .find(|f| f.first().is_some_and(|n| n == user) && f.len() >= 3)
    };
    let old = fields(old_shadow)?;
    let mut new = fields(new_shadow)?;
    if new[1] == old[1] {
        return None;
    }
    new[1] = old[1].clone();
    new[2] = old[2].clone();
    let line = new.join(":");
    let mut out: String = new_shadow
        .lines()
        .map(|l| {
            if l.split(':').next() == Some(user) {
                line.clone()
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    Some(out)
}

/// The slot the running root is on, from the kernel's command line (`root=PARTLABEL=…-a|b`).
pub fn running_slot(cmdline: &str) -> Option<char> {
    cmdline
        .split_whitespace()
        .find_map(|w| w.strip_prefix("root=PARTLABEL="))
        .and_then(|l| l.rsplit('-').next())
        .and_then(|s| match s {
            "a" => Some('A'),
            "b" => Some('B'),
            _ => None,
        })
}

/// The paths to copy from the running root into the new one (`update-keep`): one per line,
/// `#` comments, a trailing `*` matching the names that start with what's before it.
pub fn keep_paths(list: &str, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in list.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let rel = line.trim_start_matches('/');
        match rel.strip_suffix('*') {
            Some(prefix) => {
                let path = root.join(prefix);
                let (dir, start) = match path.file_name() {
                    Some(n) if !prefix.ends_with('/') => (
                        path.parent().unwrap_or(root).to_owned(),
                        n.to_string_lossy().into_owned(),
                    ),
                    _ => (path.clone(), String::new()),
                };
                let mut found: Vec<PathBuf> = fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&start))
                    .map(|e| e.path())
                    .collect();
                found.sort();
                out.extend(found);
            }
            None => {
                let p = root.join(rel);
                if p.exists() {
                    out.push(p);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn versions() {
        use Ordering::*;
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Equal);
        assert_eq!(compare_versions("0.1.0-alpha2", "0.1.0-alpha1"), Greater);
        assert_eq!(compare_versions("0.1.0-alpha10", "0.1.0-alpha9"), Greater);
        assert_eq!(compare_versions("0.1.0-beta1", "0.1.0-alpha9"), Greater);
        assert_eq!(compare_versions("0.1.0-rc1", "0.1.0-beta2"), Greater);
        assert_eq!(compare_versions("0.1.0", "0.1.0-rc1"), Greater);
        assert_eq!(compare_versions("0.2.0-alpha1", "0.1.9"), Greater);
        assert_eq!(compare_versions("v0.1.1", "0.1.0"), Greater);
        assert_eq!(compare_versions("0.1", "0.1.0"), Equal);
    }

    fn release() -> Release {
        Release::parse(
            "NIBOS_VERSION=0.1.0-alpha1\nNIBOS_BOARD=rg-ds-plus\nNIBOS_EDITION=dslot\nNIBOS_IMAGE=rgds-dslot\n",
        )
    }

    /// A small release: `chunk`-byte chunks of a made-up root, and two FITs.
    fn make(root: &[u8], chunk: u64) -> (Manifest, Vec<u8>) {
        let mut payload = Vec::new();
        let mut chunks = String::new();
        for (i, part) in root.chunks(chunk as usize).enumerate() {
            let packed = zstd_stored(part);
            if i > 0 {
                chunks.push(',');
            }
            chunks.push_str(&format!(
                r#"{{"offset":{},"size":{},"sha256":"{}","raw_sha256":"{}"}}"#,
                payload.len(),
                packed.len(),
                sha256(&packed),
                sha256(part)
            ));
            payload.extend_from_slice(&packed);
        }
        let fit_a = b"fit for slot A".to_vec();
        let fit_b = b"fit for slot B!".to_vec();
        let a_off = payload.len();
        payload.extend_from_slice(&fit_a);
        let b_off = payload.len();
        payload.extend_from_slice(&fit_b);
        let json = format!(
            r#"{{"format":1,"version":"0.1.0-alpha2","build":"abc","board":"rg-ds-plus","edition":"dslot",
            "channel":"beta","min_from":"0.1.0-alpha1","notes":"x",
            "payload":{{"file":"p.slot","size":{},"sha256":"{}"}},
            "root":{{"size":{},"sha256":"{}","chunk":{chunk},"chunks":[{chunks}]}},
            "fit":{{"a":{{"offset":{a_off},"size":{},"sha256":"{}"}},"b":{{"offset":{b_off},"size":{},"sha256":"{}"}}}},
            "migrate":1}}"#,
            payload.len(),
            sha256(&payload),
            root.len(),
            sha256(root),
            fit_a.len(),
            sha256(&fit_a),
            fit_b.len(),
            sha256(&fit_b),
        );
        (Manifest::parse(json.as_bytes()).unwrap(), payload)
    }

    /// A zstd frame holding `data` in raw blocks (no compressor needed in the tests).
    fn zstd_stored(data: &[u8]) -> Vec<u8> {
        let mut f = vec![0x28, 0xb5, 0x2f, 0xfd];
        // Frame header: a single segment, the content size in 4 bytes.
        f.push(0b1010_0000);
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        let blocks: Vec<&[u8]> = if data.is_empty() {
            vec![&[][..]]
        } else {
            data.chunks(128 * 1024).collect()
        };
        for (i, b) in blocks.iter().enumerate() {
            let last = (i == blocks.len() - 1) as u32;
            let header = last | (b.len() as u32) << 3; // raw block
            f.extend_from_slice(&header.to_le_bytes()[..3]);
            f.extend_from_slice(b);
        }
        f
    }

    fn root_bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("nibos-update-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn applicability() {
        let (m, _) = make(&root_bytes(100), 64);
        assert!(applicable(&m, &release(), false).is_ok());
        let mut base = release();
        base.edition = "base".into();
        assert!(matches!(
            applicable(&m, &base, false),
            Err(Error::NotForThis(_))
        ));
        let mut other = release();
        other.board = "rg-ds".into();
        assert!(matches!(
            applicable(&m, &other, false),
            Err(Error::NotForThis(_))
        ));
        let mut newer = release();
        newer.version = "0.1.0-alpha3".into();
        assert_eq!(applicable(&m, &newer, false).unwrap_err().code(), "older");
        assert!(applicable(&m, &newer, true).is_ok());
        let mut old = release();
        old.version = "0.0.9".into();
        assert_eq!(
            applicable(&m, &old, false).unwrap_err().code(),
            "needs_intermediate"
        );
    }

    #[test]
    fn writes_verifies_and_resumes() {
        let d = temp("write");
        let root = root_bytes(1000);
        let (m, payload) = make(&root, 64);
        fs::write(d.join("p.slot"), &payload).unwrap();
        let mut target = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(d.join("slot"))
            .unwrap();
        target.set_len(2048).unwrap();
        let cancel = AtomicBool::new(false);

        // Stopped after 5 chunks...
        let mut progress = Progress::load(&d.join("progress"), "m1");
        let mut src = FileSource(File::open(d.join("p.slot")).unwrap());
        let mut n = 0;
        let mut stop = |done: usize, _| {
            n = done;
            if done == 5 {
                cancel.store(true, AtomicOrdering::Relaxed);
            }
        };
        let r = write_root(&m, &mut src, &mut target, &mut progress, &cancel, &mut stop);
        assert!(matches!(r, Err(Error::Cancelled)));
        assert_eq!(n, 5);

        // ...resumes at the sixth, after a restart (the progress read back from its file).
        cancel.store(false, AtomicOrdering::Relaxed);
        let mut progress = Progress::load(&d.join("progress"), "m1");
        assert_eq!(progress.count(), 5);
        let mut written = Vec::new();
        let mut log = |done: usize, _| written.push(done);
        write_root(&m, &mut src, &mut target, &mut progress, &cancel, &mut log).unwrap();
        assert_eq!(written.first(), Some(&6));
        assert_eq!(progress.count(), m.root.chunks.len());
        let mut back = vec![0; root.len()];
        target.seek(SeekFrom::Start(0)).unwrap();
        target.read_exact(&mut back).unwrap();
        assert_eq!(back, root);

        // Another manifest starts afresh.
        assert_eq!(Progress::load(&d.join("progress"), "m2").count(), 0);
        assert_eq!(fetch_fit(&m, &mut src, 'B').unwrap(), b"fit for slot B!");
    }

    #[test]
    fn refuses_a_damaged_chunk() {
        let d = temp("damaged");
        let (m, mut payload) = make(&root_bytes(300), 64);
        payload[m.root.chunks[2].offset as usize + 12] ^= 1;
        let mut target = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(d.join("slot"))
            .unwrap();
        target.set_len(1024).unwrap();
        fs::write(d.join("p.slot"), &payload).unwrap();
        let mut src = FileSource(File::open(d.join("p.slot")).unwrap());
        let mut progress = Progress::load(&d.join("progress"), "m");
        let r = write_root(
            &m,
            &mut src,
            &mut target,
            &mut progress,
            &AtomicBool::new(false),
            &mut |_, _| {},
        );
        assert!(matches!(r, Err(Error::Corrupt(_))));
        assert_eq!(
            progress.count(),
            2,
            "the two good chunks before it stay done"
        );
        let mut fit = payload.clone();
        fit[m.fit.a.offset as usize] ^= 1;
        fs::write(d.join("p2.slot"), &fit).unwrap();
        let mut src = FileSource(File::open(d.join("p2.slot")).unwrap());
        assert!(matches!(
            fetch_fit(&m, &mut src, 'A'),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn a_root_too_big_for_the_slot() {
        let d = temp("big");
        let (m, payload) = make(&root_bytes(300), 64);
        fs::write(d.join("p.slot"), &payload).unwrap();
        let mut target = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(d.join("slot"))
            .unwrap();
        target.set_len(100).unwrap();
        let mut src = FileSource(File::open(d.join("p.slot")).unwrap());
        let mut progress = Progress::load(&d.join("progress"), "m");
        let r = write_root(
            &m,
            &mut src,
            &mut target,
            &mut progress,
            &AtomicBool::new(false),
            &mut |_, _| {},
        );
        assert_eq!(r.unwrap_err().code(), "no_space");
    }

    // A test key and signature made with `minisign -G -W` and `minisign -S` (not nibOS's key).
    const TEST_PUB: &str = include_str!("update/test.pub");
    const TEST_MANIFEST: &[u8] = include_bytes!("update/test.manifest.json");
    const TEST_SIG: &str = include_str!("update/test.manifest.json.minisig");

    #[test]
    fn signatures() {
        verify(TEST_MANIFEST, TEST_SIG, TEST_PUB).unwrap();
        let mut tampered = TEST_MANIFEST.to_vec();
        tampered[10] ^= 1;
        assert!(matches!(
            verify(&tampered, TEST_SIG, TEST_PUB),
            Err(Error::Signature(_))
        ));
        assert!(matches!(
            verify(TEST_MANIFEST, "nonsense", TEST_PUB),
            Err(Error::Signature(_))
        ));
    }

    #[test]
    fn power() {
        let d = temp("power");
        let (bat, chg) = (d.join("battery"), d.join("charger"));
        fs::create_dir_all(&bat).unwrap();
        fs::create_dir_all(&chg).unwrap();
        fs::write(bat.join("capacity"), "25\n").unwrap();
        fs::write(chg.join("online"), "0\n").unwrap();
        assert_eq!(power_ok(&bat, &chg, 30).unwrap_err().code(), "low_battery");
        fs::write(chg.join("online"), "1\n").unwrap();
        assert!(power_ok(&bat, &chg, 30).is_ok());
        fs::write(chg.join("online"), "0\n").unwrap();
        fs::write(bat.join("capacity"), "30\n").unwrap();
        assert!(power_ok(&bat, &chg, 30).is_ok());
    }

    #[test]
    fn slots_and_keep_list() {
        assert_eq!(
            running_slot("quiet root=PARTLABEL=nibos-root-b rw"),
            Some('B')
        );
        assert_eq!(running_slot("root=PARTLABEL=nibos-root-a"), Some('A'));
        assert_eq!(running_slot("root=/dev/mmcblk0p2"), None);
        let d = temp("keep");
        fs::create_dir_all(d.join("etc/ssh")).unwrap();
        fs::write(d.join("etc/machine-id"), "x").unwrap();
        fs::write(d.join("etc/ssh/ssh_host_ed25519_key"), "k").unwrap();
        fs::write(d.join("etc/ssh/ssh_host_ed25519_key.pub"), "k").unwrap();
        fs::write(d.join("etc/ssh/sshd_config"), "c").unwrap();
        let got = keep_paths(
            "# comment\n/etc/machine-id\n/etc/ssh/ssh_host_*\n/var/lib/missing\n",
            &d,
        );
        let names: Vec<String> = got
            .iter()
            .map(|p| p.strip_prefix(&d).unwrap().display().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "etc/machine-id",
                "etc/ssh/ssh_host_ed25519_key",
                "etc/ssh/ssh_host_ed25519_key.pub"
            ]
        );
    }

    #[test]
    fn download_failures_have_stable_codes() {
        let api = "https://api.github.com/repos/o/n/releases";
        let dl = "https://github.com/o/n/releases/download/v1/x.slot";
        // A private or missing repository: 404, whatever curl's exit (56 under HTTP/2).
        assert_eq!(classify(Some(56), 404, api).0, "not_found");
        assert_eq!(classify(Some(22), 404, dl).0, "not_found");
        assert_eq!(classify(Some(22), 403, api).0, "rate_limited");
        assert_eq!(classify(Some(22), 429, dl).0, "rate_limited");
        assert_eq!(classify(Some(22), 403, dl).0, "http_error");
        assert_eq!(classify(Some(22), 503, dl).0, "server_error");
        assert_eq!(classify(Some(6), 0, dl).0, "no_network");
        assert_eq!(classify(Some(28), 0, dl).0, "no_network");
        assert_eq!(classify(Some(60), 0, dl).0, "tls_error");
        assert_eq!(classify(Some(3), 0, dl).0, "network_error");
        // The sentence for a person never carries curl's text.
        assert!(!classify(Some(56), 404, api).1.contains("curl"));
        let e = Error::Io(io::Error::other(NetError {
            code: "not_found",
            detail: "nothing is published at the update address".into(),
            raw: "curl: (56) The requested URL returned error: 404".into(),
        }));
        assert_eq!(e.code(), "not_found");
        assert_eq!(e.to_string(), "nothing is published at the update address");
        assert!(e.raw().unwrap().contains("404"));
        assert_eq!(Error::Io(io::Error::other("disk full")).code(), "io_error");
    }

    #[test]
    fn password_carries_over_alone() {
        let old = "root:!*:20000::::::\nnibos:$y$j9T$abc$def:20380:0:99999:7:::\n";
        let new = "root:!*:20385::::::\nnibos::20385:0:99999:7:::\nnewsvc:!*:20385::::::\n";
        let out = carry_password(old, new, "nibos").unwrap();
        assert_eq!(
            out,
            "root:!*:20385::::::\nnibos:$y$j9T$abc$def:20380:0:99999:7:::\nnewsvc:!*:20385::::::\n"
        );
        // Already the same, or the user missing on either side: nothing to do.
        assert!(carry_password(&out, &out, "nibos").is_none());
        assert!(carry_password("root:x:1::::::\n", new, "nibos").is_none());
        assert!(carry_password(old, "root:x:1::::::\n", "nibos").is_none());
        // A locked account (no password set) carries over as locked.
        let locked = "nibos:!:20380:0:99999:7:::\n";
        assert!(carry_password(locked, new, "nibos")
            .unwrap()
            .contains("nibos:!:20380:"));
    }
}
