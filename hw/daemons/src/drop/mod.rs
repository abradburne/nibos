//! `nibos-drop`: headless LocalSend v2.1 ROM receiver for nibOS (docs/design/rom-drop.md).
//!
//! - [`rom`]: two-stage ROM filter and content verifier for every console DSlot plays;
//! - [`protocol`]: LocalSend v2.1 DTOs, self-signed P-256 TLS identity, HTTP/1.1 framing;
//! - [`server`]: UDP multicast discovery, HTTPS server, session management, atomic placement.

pub mod protocol;
pub mod rom;
pub mod server;

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Persistent configuration and TLS identity directory (kept across A/B updates via `update-keep`).
pub const STATE_DIR: &str = "/var/lib/nibos/drop";
/// Runtime status file read by `nib drop` and DSlot's Network menu.
pub const STATUS_FILE: &str = "/run/nibos/drop/status.json";
/// DSlot's emulator descriptions, for Arcade and Game & Watch `[games]` set lists.
pub const EMULATORS_DIR: &str = "/usr/share/dslot/emulators";
/// Hidden staging folder name inside the target `Roms/` root (ignored by DSlot's scanner).
pub const STAGING_DIR: &str = ".drop-staging";
/// Default LocalSend v2.1 TCP + UDP port.
pub const DEFAULT_PORT: u16 = 53317;
/// Default device name shown in LocalSend when no custom alias is configured.
pub const DEFAULT_ALIAS: &str = "DSlot";
/// Safety headroom left free on the target card when accepting a transfer.
pub const FREE_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
/// Maximum number of recent transfer events kept in `status.json`.
pub const MAX_RECENT: usize = 20;

/// Which SD card dropped ROMs go to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetMode {
    /// TF2 (`/mnt/ds/Roms`) if mounted, else TF1 (`/mnt/roms/Roms`).
    #[default]
    Auto,
    /// Always TF1 (`/mnt/roms/Roms`).
    Tf1,
    /// Always TF2 (`/mnt/ds/Roms`), refusing transfers when TF2 is not mounted.
    Tf2,
}

impl TargetMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "tf1" | "internal" => Some(Self::Tf1),
            "tf2" | "sd" | "card" => Some(Self::Tf2),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tf1 => "tf1",
            Self::Tf2 => "tf2",
        }
    }
}

fn default_true() -> bool {
    true
}

/// Persistent settings in `/var/lib/nibos/drop/config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Device name shown in LocalSend; empty uses `DEFAULT_ALIAS` (`"DSlot"`).
    pub alias: String,
    /// Destination card preference (`auto`, `tf1`, `tf2`).
    pub target: TargetMode,
    /// Optional LocalSend PIN; empty disables PIN protection.
    pub pin: String,
    /// Automatically fetch box and cart artwork in DSlot when a ROM lands.
    #[serde(default = "default_true")]
    pub auto_artwork: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alias: String::new(),
            target: TargetMode::Auto,
            pin: String::new(),
            auto_artwork: true,
        }
    }
}

impl Config {
    pub fn load(state_dir: &Path) -> Self {
        let path = state_dir.join("config.toml");
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, state_dir: &Path) -> io::Result<()> {
        fs::create_dir_all(state_dir)?;
        let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let body = format!(
            "alias = \"{}\"\ntarget = \"{}\"\npin = \"{}\"\nauto_artwork = {}\n",
            escape(&self.alias),
            self.target.as_str(),
            escape(&self.pin),
            self.auto_artwork,
        );
        atomic_write(&state_dir.join("config.toml"), body.as_bytes())
    }
}

/// Active upload progress in `status.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTransfer {
    pub file: String,
    pub received: u64,
    pub total: u64,
}

/// Running totals for this daemon session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub accepted: u64,
    pub rejected: u64,
}

/// One completed or rejected file event in `status.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEvent {
    pub time: String,
    pub file: String,
    /// `"accepted"` or `"rejected"`.
    pub outcome: String,
    /// Target console folder (`"GBA"`, `"SFC"`, ...) when accepted; empty when rejected.
    pub console: String,
    /// Full destination path when accepted; empty when rejected.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Why a file was rejected; empty when accepted.
    pub reason: String,
}

/// Runtime status written atomically to `/run/nibos/drop/status.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub active: bool,
    pub alias: String,
    pub port: u16,
    pub pin_required: bool,
    pub target: String,
    pub target_label: String,
    pub free_bytes: u64,
    pub transfer: Option<ActiveTransfer>,
    pub counts: Counts,
    pub recent: Vec<RecentEvent>,
}

impl Status {
    pub fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        atomic_write(path, &json)
    }

    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// Writes `bytes` to `path` via a sibling temp file and atomic rename.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Where ROMs land for a transfer: the `Roms/` directory and a human-readable label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub roms_dir: PathBuf,
    pub label: String,
}

/// Resolves the destination `Roms/` directory. When `override_dir` is set (desktop testing), uses
/// it directly; otherwise checks `/proc/mounts` for `/mnt/ds` (TF2) and `/mnt/roms` (TF1).
pub fn resolve_target(
    mode: TargetMode,
    override_dir: Option<&Path>,
    mounts_text: &str,
) -> Result<ResolvedTarget, String> {
    if let Some(dir) = override_dir {
        return Ok(ResolvedTarget {
            roms_dir: dir.to_path_buf(),
            label: dir.display().to_string(),
        });
    }
    let mounted = |point: &str| {
        mounts_text
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(point))
    };
    let tf2 = mounted("/mnt/ds");
    let tf1 = mounted("/mnt/roms");
    match mode {
        TargetMode::Auto => {
            if tf2 {
                Ok(ResolvedTarget {
                    roms_dir: PathBuf::from("/mnt/ds/Roms"),
                    label: "TF2 card".into(),
                })
            } else if tf1 {
                Ok(ResolvedTarget {
                    roms_dir: PathBuf::from("/mnt/roms/Roms"),
                    label: "Internal card".into(),
                })
            } else {
                Err("no storage card mounted".into())
            }
        }
        TargetMode::Tf2 => {
            if tf2 {
                Ok(ResolvedTarget {
                    roms_dir: PathBuf::from("/mnt/ds/Roms"),
                    label: "TF2 card".into(),
                })
            } else {
                Err("TF2 card is not mounted".into())
            }
        }
        TargetMode::Tf1 => {
            if tf1 {
                Ok(ResolvedTarget {
                    roms_dir: PathBuf::from("/mnt/roms/Roms"),
                    label: "Internal card".into(),
                })
            } else {
                Err("internal data partition is not mounted".into())
            }
        }
    }
}

/// Available bytes for unprivileged writes on the filesystem containing `path`.
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(probe.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `stat` is a writable `statvfs` struct.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// Reads the system hostname (falling back to `"nibOS"`).
pub fn system_hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nibOS".into())
}

/// Reads the board's display name from `board.conf`, falling back to `"nibOS"`.
pub fn board_model() -> String {
    crate::board::Board::load()
        .ok()
        .map(|b| b.name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nibOS".into())
}

/// Sanitises a single filename component for FAT32/exFAT safety. Returns `None` if the name is
/// empty, hidden (leading `.`), traversal (`..`), or contains control characters.
pub fn sanitize_component(name: &str) -> Option<String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.contains('\0')
        || name.contains('/')
        || name.contains('\\')
    {
        return None;
    }
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_control() {
            return None;
        }
        match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            _ => out.push(c),
        }
    }
    let trimmed = out.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() || trimmed.starts_with('.') {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Validates a LocalSend relative file name: either a single filename (`foo.gba`) or one subfolder
/// (`Game/EBOOT.PBP`, `DiscSet/game.cue`). Rejects absolute paths, `..`, hidden segments, and
/// deeper nesting.
pub fn sanitize_relpath(raw: &str) -> Option<(Option<String>, String)> {
    let normalized = raw.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    match parts.as_slice() {
        [file] => Some((None, sanitize_component(file)?)),
        [dir, file] => Some((Some(sanitize_component(dir)?), sanitize_component(file)?)),
        _ => None,
    }
}

/// Formats the current wall-clock time as `HH:MM:SS` (local time via `libc::localtime_r`).
pub fn now_hms() -> String {
    // SAFETY: `time` and `localtime_r` operate on stack locals with valid pointers.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "00:00:00".into();
        }
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// Removes any leftover `.drop-staging` directories under `roms_dir`.
pub fn cleanup_staging(roms_dir: &Path) {
    let staging = roms_dir.join(STAGING_DIR);
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_resolution_prefers_tf2_in_auto_mode() {
        let both = "/dev/mmcblk0p6 /mnt/roms exfat rw 0 0\n/dev/mmcblk1p1 /mnt/ds exfat rw 0 0\n";
        let only_tf1 = "/dev/mmcblk0p6 /mnt/roms exfat rw 0 0\n";
        let none = "/dev/mmcblk0p3 / ext4 rw 0 0\n";

        assert_eq!(
            resolve_target(TargetMode::Auto, None, both)
                .unwrap()
                .roms_dir,
            PathBuf::from("/mnt/ds/Roms")
        );
        assert_eq!(
            resolve_target(TargetMode::Auto, None, only_tf1)
                .unwrap()
                .roms_dir,
            PathBuf::from("/mnt/roms/Roms")
        );
        assert!(resolve_target(TargetMode::Auto, None, none).is_err());
        assert_eq!(
            resolve_target(TargetMode::Tf1, None, both)
                .unwrap()
                .roms_dir,
            PathBuf::from("/mnt/roms/Roms")
        );
        assert!(resolve_target(TargetMode::Tf2, None, only_tf1).is_err());
    }

    #[test]
    fn sanitize_relpath_blocks_traversal_and_hidden_files() {
        assert_eq!(
            sanitize_relpath("Advance Wars (USA).gba"),
            Some((None, "Advance Wars (USA).gba".into()))
        );
        assert_eq!(
            sanitize_relpath("Castlevania/EBOOT.PBP"),
            Some((Some("Castlevania".into()), "EBOOT.PBP".into()))
        );
        assert_eq!(
            sanitize_relpath("Foo:Bar?.nes"),
            Some((None, "Foo_Bar_.nes".into()))
        );
        assert_eq!(sanitize_relpath("../evil.gba"), None);
        assert_eq!(sanitize_relpath("/etc/passwd"), None);
        assert_eq!(sanitize_relpath(".DS_Store"), None);
        assert_eq!(sanitize_relpath("a/b/c.gba"), None);
    }

    #[test]
    fn config_round_trips() {
        let dir = std::env::temp_dir().join(format!("nibos-drop-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cfg = Config {
            alias: "My RG DS+".into(),
            target: TargetMode::Tf2,
            pin: "1234".into(),
            auto_artwork: false,
        };
        cfg.save(&dir).unwrap();
        assert_eq!(Config::load(&dir), cfg);
        let _ = fs::remove_dir_all(&dir);
    }
}
