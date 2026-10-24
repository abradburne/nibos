//! nibos-update: system updates (`docs/design/updates.md`). A release is a whole slot, written
//! to the slot that isn't running, checked against its signed manifest as it is written and
//! read back after, then tried with the A/B slots' three tries.
//!
//!   nibos-update check [--from FILE|URL]   the newest release for this device on its channel
//!                                          (or the one given): verified, kept as pending
//!   nibos-update download                  write the pending release to the inactive slot
//!                                          (resumable; nibos-update@download.service, low priority)
//!   nibos-update apply                     try the written slot: restart into it
//!   nibos-update rollback                  back to the other slot, if it is good: restart
//!   nibos-update booted                    at boot, and once the slot is confirmed: state
//!                                          migrations, "updated to", "the update failed"
//!   nibos-update ack                       forget "updated to" / "failed" once they were shown
//!   nibos-update channel stable|beta       which releases to follow
//!   nibos-update show                      the status, in words (no root needed)
//!
//! Every command but `show` runs as root, through `nibos-update@<command>.service` (`nib update`
//! starts them; polkit lets the wheel group). Each writes `/run/nibos/update/status.json`, which
//! `show`, `nib update` and DSlot read. What lasts across restarts is on the state partition.

use nibos_hw::update::{
    self, applicable, fetch, fetch_fit, keep_paths, power_ok, running_slot, verify, write_root,
    Error, FileSource, Manifest, Progress, Release, Source, UrlSource,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const RELEASE: &str = "/usr/lib/nibos/release";
const PUBLIC_KEY: &str = "/usr/lib/nibos/update.pub";
const KEEP: &str = "/usr/lib/nibos/update-keep";
const MIGRATE: &str = "/usr/lib/nibos/migrate.d";
/// What lasts: the pending release, progress, what was applied, the channel.
const STATE: &str = "/var/lib/nibos/state/update";
/// What `nib update` leaves for the next command (`from`, `allow-downgrade`), and the status.
const RUN: &str = "/run/nibos/update";
const BATTERY: &str = "/run/nibos/battery";
const CHARGER: &str = "/run/nibos/charger";

/// A path of the system's, or under NIBOS_UPDATE_TEST_ROOT: tools/test-update.sh runs the real
/// program against a card image (with NIBOS_UPDATE_RUNNING, NIBOS_UPDATE_SLOT_DEV,
/// NIBOS_BOOTCTL_DEVICE and NIBOS_UPDATE_NO_REBOOT beside it).
fn sys(path: &str) -> PathBuf {
    match std::env::var_os("NIBOS_UPDATE_TEST_ROOT") {
        Some(root) => Path::new(&root).join(path.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}
const MIN_BATTERY: u32 = 30;

static STOP: AtomicBool = AtomicBool::new(false);
/// The step running, for the status (`step`: what "Try again" runs).
static STEP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn state(name: &str) -> PathBuf {
    sys(STATE).join(name)
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

fn write(path: impl AsRef<Path>, text: &str) -> std::io::Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    fs::rename(tmp, path)
}

fn release() -> Release {
    Release::parse(&fs::read_to_string(sys(RELEASE)).unwrap_or_default())
}

/// A JSON string.
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The status file: what is happening, for `show`, `nib update` and DSlot.
fn status(phase: &str, detail: &str, extra: &[(&str, String)]) {
    let r = release();
    let mut json = format!(
        "{{\"phase\":{},\"detail\":{},\"version\":{},\"edition\":{},\"channel\":{}",
        js(phase),
        js(detail),
        js(&r.version),
        js(&r.edition),
        js(&channel()),
    );
    if let Some(m) = pending() {
        json.push_str(&format!(
            ",\"available\":{},\"notes\":{},\"size\":{}",
            js(&m.version),
            js(&m.notes),
            m.payload.size
        ));
    }
    // The version in the other slot, and whether going back to it is possible (it is good).
    if let Some(prev) = read(state("previous")) {
        json.push_str(&format!(",\"previous\":{}", js(&prev)));
    }
    json.push_str(&format!(",\"rollback_ok\":{}", other_slot_good()));
    if let Some(step) = STEP.get() {
        json.push_str(&format!(",\"step\":{}", js(step)));
    }
    for (k, v) in [("updated_to", "updated-to"), ("failed", "failed")] {
        if let Some(x) = read(state(v)) {
            json.push_str(&format!(",\"{k}\":{}", js(&x)));
        }
    }
    for (k, v) in extra {
        json.push_str(&format!(",\"{k}\":{v}"));
    }
    json.push_str("}\n");
    let _ = fs::create_dir_all(sys(RUN));
    let _ = write(sys(RUN).join("status.json"), &json);
}

/// Whether the slot that isn't running is confirmed good (nibos-bootctl status).
fn other_slot_good() -> bool {
    let Some(running) = std::env::var("NIBOS_UPDATE_RUNNING")
        .ok()
        .and_then(|s| s.chars().next())
        .or_else(|| running_slot(&fs::read_to_string("/proc/cmdline").unwrap_or_default()))
    else {
        return false;
    };
    let other = if running == 'A' { 'B' } else { 'A' };
    Command::new("nibos-bootctl")
        .arg("status")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .is_some_and(|out| {
            out.lines()
                .any(|l| l.starts_with(&format!("slot {other}: good")))
        })
}

fn fail(e: impl std::fmt::Display) -> ! {
    let msg = e.to_string();
    eprintln!("nibos-update: {msg}");
    status("error", &msg, &[]);
    exit(1);
}

fn channel() -> String {
    read(state("channel")).unwrap_or_else(|| release().channel)
}

/// The pending release's manifest, verified again from its saved bytes.
fn pending() -> Option<Manifest> {
    let bytes = fs::read(state("manifest.json")).ok()?;
    let sig = fs::read_to_string(state("manifest.json.minisig")).ok()?;
    let key = fs::read_to_string(sys(PUBLIC_KEY)).ok()?;
    verify(&bytes, &sig, &key).ok()?;
    Manifest::parse(&bytes).ok()
}

/// Where the newest release on the channel is: its manifest's address, from GitHub's list of
/// the repository's releases (beta takes pre-releases too).
fn channel_manifest(r: &Release, channel: &str) -> Result<String, Error> {
    if r.repo.is_empty() || r.image.is_empty() {
        return Err(Error::Refused("this image has no update source".into()));
    }
    let api = format!(
        "https://api.github.com/repos/{}/releases?per_page=20",
        r.repo
    );
    let list = fetch(&api, None)?;
    let releases: serde_json::Value =
        serde_json::from_slice(&list).map_err(|e| Error::Manifest(format!("the releases: {e}")))?;
    let suffix = format!("-{}.manifest.json", r.image);
    for rel in releases.as_array().into_iter().flatten() {
        let draft = rel["draft"].as_bool().unwrap_or(true);
        let pre = rel["prerelease"].as_bool().unwrap_or(false);
        if draft || (pre && channel != "beta") {
            continue;
        }
        for a in rel["assets"].as_array().into_iter().flatten() {
            let name = a["name"].as_str().unwrap_or("");
            if name.ends_with(&suffix) {
                if let Some(url) = a["browser_download_url"].as_str() {
                    return Ok(url.to_owned());
                }
            }
        }
    }
    Err(Error::Refused(format!(
        "no release on the {channel} channel for this device"
    )))
}

/// Reads a manifest or its signature from a file or an address.
fn get(place: &str) -> Result<Vec<u8>, Error> {
    if place.starts_with("https://") || place.starts_with("http://") {
        Ok(fetch(place, None)?)
    } else {
        Ok(fs::read(place)?)
    }
}

fn check(from: Option<String>) {
    let r = release();
    status("checking", "", &[]);
    let place = match from {
        Some(f) => f,
        None => channel_manifest(&r, &channel()).unwrap_or_else(|e| fail(e)),
    };
    let bytes = get(&place).unwrap_or_else(|e| fail(e));
    let sig = get(&format!("{place}.minisig")).unwrap_or_else(|e| fail(e));
    let key = fs::read_to_string(sys(PUBLIC_KEY))
        .unwrap_or_else(|_| fail("this image has no update key: write a newer image"));
    let sig = String::from_utf8_lossy(&sig).into_owned();
    verify(&bytes, &sig, &key).unwrap_or_else(|e| fail(e));
    let m = Manifest::parse(&bytes).unwrap_or_else(|e| fail(e));
    let downgrade = sys(RUN).join("allow-downgrade").exists();
    if let Err(e) = applicable(&m, &r, downgrade) {
        status("current", &e.to_string(), &[]);
        println!("{e}");
        return;
    }
    // A different release than the one written so far starts the slot afresh.
    let same = pending().is_some_and(|p| p.version == m.version && p.build == m.build);
    fs::create_dir_all(sys(STATE)).unwrap_or_else(|e| fail(e));
    write(state("manifest.json.minisig"), &sig).unwrap_or_else(|e| fail(e));
    fs::write(state("manifest.json"), &bytes).unwrap_or_else(|e| fail(e));
    let base = place.rsplit_once('/').map_or(".", |(b, _)| b).to_owned();
    write(state("source"), &base).unwrap_or_else(|e| fail(e));
    if downgrade {
        let _ = write(state("allow-downgrade"), "");
    } else {
        let _ = fs::remove_file(state("allow-downgrade"));
    }
    if !same {
        let _ = fs::remove_file(state("progress"));
        let _ = fs::remove_file(state("ready"));
    }
    println!(
        "{} is available ({} MB)",
        m.version,
        m.payload.size / 1_000_000
    );
    status("available", "", &[]);
}

/// The device the inactive slot's root is on.
fn slot_device(slot: char) -> PathBuf {
    if let Some(dev) = std::env::var_os("NIBOS_UPDATE_SLOT_DEV") {
        return PathBuf::from(dev);
    }
    PathBuf::from(format!(
        "/dev/disk/by-partlabel/nibos-root-{}",
        slot.to_ascii_lowercase()
    ))
}

fn bootctl(args: &[&str]) -> bool {
    Command::new("nibos-bootctl")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn running() -> char {
    if let Some(s) = std::env::var("NIBOS_UPDATE_RUNNING")
        .ok()
        .and_then(|s| s.chars().next())
    {
        return s;
    }
    running_slot(&fs::read_to_string("/proc/cmdline").unwrap_or_default())
        .unwrap_or_else(|| fail("this system isn't on an A/B slot (a development card)"))
}

fn preconditions() {
    power_ok(&sys(BATTERY), &sys(CHARGER), MIN_BATTERY).unwrap_or_else(|e| fail(e));
    if bootctl(&["unconfirmed"]) {
        fail("the running system isn't confirmed yet: update once it is");
    }
}

fn download() {
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
    }
    let r = release();
    let m = pending().unwrap_or_else(|| fail("nothing to download: check first"));
    applicable(&m, &r, state("allow-downgrade").exists()).unwrap_or_else(|e| fail(e));
    preconditions();
    let target_slot = if running() == 'A' { 'B' } else { 'A' };
    let dev = slot_device(target_slot);
    if !dev.exists() {
        fail("this card has no second slot");
    }
    let base = read(state("source")).unwrap_or_else(|| fail("no source: check again"));
    let payload = format!("{base}/{}", m.payload.file);
    let mut source: Box<dyn Source> = if payload.starts_with("http") {
        Box::new(UrlSource(payload))
    } else {
        Box::new(FileSource(
            File::open(&payload).unwrap_or_else(|e| fail(format!("{payload}: {e}"))),
        ))
    };
    let mut target = File::options()
        .read(true)
        .write(true)
        .open(&dev)
        .unwrap_or_else(|e| fail(format!("{}: {e}", dev.display())));
    let manifest_sha = update::sha256(&fs::read(state("manifest.json")).unwrap_or_default());
    let mut progress = Progress::load(&state("progress"), &manifest_sha);
    // The other slot's old contents are being written over: nothing to go back to any more.
    let _ = fs::remove_file(state("previous"));
    let total = m.root.chunks.len();
    let started = Instant::now();
    status(
        "downloading",
        "",
        &[
            ("done", progress.count().to_string()),
            ("total", total.to_string()),
        ],
    );
    let mut report = |done: usize, total: usize| {
        status(
            "downloading",
            "",
            &[("done", done.to_string()), ("total", total.to_string())],
        );
    };
    match write_root(
        &m,
        source.as_mut(),
        &mut target,
        &mut progress,
        &STOP,
        &mut report,
    ) {
        Ok(()) => {}
        Err(Error::Cancelled) => {
            status(
                "paused",
                "",
                &[
                    ("done", progress.count().to_string()),
                    ("total", total.to_string()),
                ],
            );
            return;
        }
        Err(e) => fail(e),
    }
    drop(target);
    status("finishing", "", &[]);
    // The slot's FIT, through nibos-bootctl (which finds the Falcon partition and reads it back).
    let fit = fetch_fit(&m, source.as_mut(), target_slot).unwrap_or_else(|e| fail(e));
    let fit_file = sys(RUN).join("fit");
    fs::write(&fit_file, &fit).unwrap_or_else(|e| fail(e));
    let slot = target_slot.to_string();
    if !bootctl(&["write-fit", &slot, &fit_file.to_string_lossy()]) {
        fail("couldn't write the slot's FIT");
    }
    let _ = fs::remove_file(&fit_file);
    carry_over(&dev);
    write(state("ready"), &format!("{} {target_slot}", m.version)).unwrap_or_else(|e| fail(e));
    println!(
        "{} is written to slot {target_slot} ({} s): restart to update",
        m.version,
        started.elapsed().as_secs()
    );
    status("ready", "", &[]);
}

/// Copies what the running root keeps itself (`update-keep`: the device's identity, the
/// user's settings, the radios, pairings) into the new root.
fn carry_over(dev: &Path) {
    let mnt = sys(RUN).join("slot");
    let _ = Command::new("umount")
        .arg(&mnt)
        .stderr(std::process::Stdio::null())
        .status();
    fs::create_dir_all(&mnt).unwrap_or_else(|e| fail(e));
    let ok = Command::new("mount")
        .args(["-t", "ext4"])
        .arg(dev)
        .arg(&mnt)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        fail("couldn't mount the new slot's root");
    }
    let list = fs::read_to_string(sys(KEEP)).unwrap_or_default();
    let mut result = Ok(());
    let root = sys("/");
    for p in keep_paths(&list, &root) {
        let dest = mnt.join(p.strip_prefix(&root).unwrap_or(&p));
        if dest.is_dir() && !dest.is_symlink() {
            let _ = fs::remove_dir_all(&dest);
        } else {
            let _ = fs::remove_file(&dest);
        }
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // -x: the state partition (under /var/lib/nibos) stays where it is, not copied in.
        let copied = Command::new("cp")
            .arg("-ax")
            .arg(&p)
            .arg(&dest)
            .status()
            .is_ok_and(|s| s.success());
        if !copied {
            result = Err(format!("couldn't keep {}", p.display()));
            break;
        }
    }
    let _ = Command::new("sync").status();
    let _ = Command::new("umount").arg(&mnt).status();
    if let Err(e) = result {
        fail(e);
    }
}

fn reboot() {
    if std::env::var_os("NIBOS_UPDATE_NO_REBOOT").is_none() {
        let _ = Command::new("systemctl").arg("reboot").status();
    }
}

fn apply() {
    let m = pending().unwrap_or_else(|| fail("nothing to apply: check and download first"));
    let ready = read(state("ready")).unwrap_or_else(|| fail("the update isn't written yet"));
    let (version, slot) = ready
        .split_once(' ')
        .unwrap_or_else(|| fail("the update isn't written yet"));
    if version != m.version {
        fail("the written update isn't the pending one: download again");
    }
    preconditions();
    // An image's card has no boot control block until the first update (SPL boots slot A
    // without one): make it now, with the running slot A good.
    let has_block = Command::new("nibos-bootctl")
        .arg("status")
        .output()
        .is_ok_and(|o| !String::from_utf8_lossy(&o.stdout).contains("no boot control block"));
    if !has_block && !bootctl(&["init"]) {
        fail("couldn't make the boot control block");
    }
    write(state("applied"), version).unwrap_or_else(|e| fail(e));
    let _ = write(state("previous"), &release().version);
    if !bootctl(&["try", slot]) {
        let _ = fs::remove_file(state("applied"));
        fail("couldn't set the slot to be tried");
    }
    status("restarting", version, &[]);
    reboot();
}

fn rollback() {
    if !bootctl(&["rollback"]) {
        fail("the other slot isn't good: there is nothing to go back to");
    }
    status("restarting", "back to the previous version", &[]);
    reboot();
}

/// At boot (before the boot UI) and again once the slot is confirmed: the state migrations of
/// a new version, and what became of an applied update.
fn booted() {
    let r = release();
    let _ = fs::create_dir_all(sys(STATE));
    if let Some(last) = read(state("last-version")).filter(|v| *v != r.version) {
        let mut scripts: Vec<PathBuf> = fs::read_dir(sys(MIGRATE))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        scripts.sort();
        for s in scripts {
            match Command::new(&s).arg(&last).arg(&r.version).status() {
                Ok(st) if st.success() => {}
                _ => eprintln!("nibos-update: {} failed", s.display()),
            }
        }
    }
    let _ = write(state("last-version"), &r.version);
    if let Some(applied) = read(state("applied")) {
        if applied == r.version {
            if !bootctl(&["unconfirmed"]) {
                // Confirmed: done. The old slot stays as the way back.
                let _ = write(state("updated-to"), &applied);
                for f in [
                    "applied",
                    "ready",
                    "progress",
                    "manifest.json",
                    "manifest.json.minisig",
                ] {
                    let _ = fs::remove_file(state(f));
                }
            }
        } else {
            // Back on the old version: the new one didn't come up.
            let _ = write(state("failed"), &applied);
            for f in ["applied", "ready", "progress", "previous"] {
                let _ = fs::remove_file(state(f));
            }
        }
    }
    let phase = if read(state("applied")).is_some() {
        "trying"
    } else if read(state("ready")).is_some() {
        "ready"
    } else if pending().is_some() {
        "available"
    } else {
        "idle"
    };
    status(phase, "", &[]);
}

fn show() {
    let text = read(sys(RUN).join("status.json")).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    println!(
        "nibOS {} ({} edition, {} channel)",
        s("version"),
        s("edition"),
        s("channel")
    );
    if !s("updated_to").is_empty() {
        println!("updated to {}", s("updated_to"));
    }
    if !s("failed").is_empty() {
        println!(
            "the update to {} didn't start: back on {}",
            s("failed"),
            s("version")
        );
    }
    let available = s("available");
    match s("phase").as_str() {
        "downloading" => println!(
            "downloading {available}: {} of {} parts",
            v["done"], v["total"]
        ),
        "paused" => println!(
            "downloading {available}: paused at {} of {} parts",
            v["done"], v["total"]
        ),
        "ready" => println!("{available} is ready: nib update apply restarts into it"),
        "available" => println!("{available} is available: nib update download"),
        "trying" => println!("trying the new version until it is confirmed"),
        "error" => println!("the last attempt failed: {}", s("detail")),
        "current" => println!("{}", s("detail")),
        "" => println!("no update checked yet: nib update check"),
        _ => println!("up to date"),
    }
    if !s("notes").is_empty() && !available.is_empty() {
        println!("\n{}", s("notes"));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("show");
    let _ = STEP.set(cmd.to_owned());
    if cmd != "show"
        && unsafe { libc::geteuid() } != 0
        && std::env::var_os("NIBOS_UPDATE_TEST_ROOT").is_none()
    {
        eprintln!("nibos-update: {cmd} runs as root (nib update {cmd})");
        exit(2);
    }
    match cmd {
        "check" => {
            let from = args
                .iter()
                .position(|a| a == "--from")
                .and_then(|i| args.get(i + 1).cloned())
                .or_else(|| read(sys(RUN).join("from")).filter(|s| !s.is_empty()));
            let _ = fs::remove_file(sys(RUN).join("from"));
            check(from);
        }
        "download" => download(),
        "apply" => apply(),
        "rollback" => rollback(),
        "booted" => booted(),
        "ack" => {
            let _ = fs::remove_file(state("updated-to"));
            let _ = fs::remove_file(state("failed"));
            booted();
        }
        "channel-stable" | "channel-beta" => {
            let _ = fs::create_dir_all(sys(STATE));
            write(state("channel"), &cmd["channel-".len()..]).unwrap_or_else(|e| fail(e));
            booted();
        }
        "show" => show(),
        _ => {
            eprintln!("usage: nibos-update check|download|apply|rollback|booted|ack|channel-stable|channel-beta|show");
            exit(2);
        }
    }
}
