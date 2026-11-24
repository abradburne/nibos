//! nibos-provision: applies `provision.toml` from the card's `NIBOS` partition, at every boot
//! of a downloadable image, before the network, SSH and the console start
//! ([`nibos_hw::provision`]; `docs/design/images.md`).
//!
//! - A valid file is applied (Wi-Fi and SSH keys to the state partition, the password, the time
//!   zone, the data card), then overwritten and deleted.
//! - A file that fails a check is renamed `provision.rejected-N.toml`, with
//!   `provision.error.txt` saying why (never quoting a value), and nothing in it is applied.
//! - Either way, the device's access follows what is set: no password, the console logs the
//!   user in and sudo and polkit ask for none (the console is the way in); a password, a login
//!   prompt and sudo asks for it; SSH runs only once a key is set.

use nibos_hw::provision::{self, DataCard, Provision};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs,
    io::{self, Write},
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{exit, Command, Stdio},
};

/// The FAT partition with provision.toml, mounted here while this runs.
const BOOT_PART: &str = "/dev/disk/by-partlabel/nibos-boot";
const BOOT_MOUNT: &str = "/run/nibos/boot";
/// The state partition (fstab): what provisioning sets, kept across a new root.
const STATE: &str = "/var/lib/nibos/state";
/// The image's one user (uid 1000).
const USER: &str = "nibos";
const ZONEINFO: &str = "/usr/share/zoneinfo";
const LOCALTIME: &str = "/etc/localtime";

fn run(cmd: &str, args: &[&str], input: Option<&str>) -> io::Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()?;
    if let Some(text) = input {
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(text.as_bytes())?;
    }
    match child.wait()? {
        s if s.success() => Ok(()),
        s => Err(io::Error::other(format!("{cmd}: {s}"))),
    }
}

/// Writes a file whole (a temporary file, then a rename), with the mode given.
fn write_file(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("a directory"))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::File::open(&tmp)?.sync_all()?;
    fs::rename(&tmp, path)
}

/// Overwrites the file before deleting it: FAT keeps deleted data otherwise.
fn wipe(path: &Path) -> io::Result<()> {
    let len = fs::metadata(path)?.len() as usize;
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    f.write_all(&vec![0u8; len])?;
    f.sync_all()?;
    fs::remove_file(path)
}

/// Points /etc/localtime at the zone named, if there is one; whether it did.
fn set_timezone(z: &str) -> io::Result<bool> {
    let zone = Path::new(ZONEINFO).join(z);
    if !zone.is_file() {
        return Ok(false);
    }
    let _ = fs::remove_file(LOCALTIME);
    symlink(&zone, LOCALTIME)?;
    Ok(true)
}

/// The zone kept on the state partition (`timezone`, one line: provision.toml's, or what `nib
/// clock zone` set) back on /etc/localtime, which a system update replaces with the image's.
fn restore_timezone(state: &Path) -> io::Result<()> {
    let Ok(text) = fs::read_to_string(state.join("timezone")) else {
        return Ok(());
    };
    let z = text.trim();
    if !provision::valid_timezone(z) {
        eprintln!("nibos-provision: state/timezone isn't a time zone name; ignored");
        return Ok(());
    }
    let now = fs::read_link(LOCALTIME).ok();
    if now.as_deref() == Some(Path::new(ZONEINFO).join(z).as_path()) {
        return Ok(());
    }
    if set_timezone(z)? {
        eprintln!("nibos-provision: time zone {z} (from the state partition)");
    } else {
        eprintln!("nibos-provision: no time zone {z} on this system; kept the image's");
    }
    Ok(())
}

fn apply(p: &Provision, state: &Path) -> io::Result<()> {
    if let Some(z) = &p.timezone {
        if set_timezone(z)? {
            // Kept with the other provisioned settings, where `nib clock zone` keeps its too.
            write_file(&state.join("timezone"), &format!("{z}\n"), 0o644)?;
        } else {
            eprintln!("nibos-provision: no time zone {z}; kept the old one");
        }
    }
    if let Some(card) = p.data_card {
        let name = match card {
            DataCard::Auto => "auto",
            DataCard::Tf1 => "tf1",
            DataCard::Tf2 => "tf2",
        };
        write_file(&state.join("data_card"), &format!("{name}\n"), 0o644)?;
    }
    if let Some(w) = &p.wifi {
        let dir = state.join("iwd");
        write_file(
            &dir.join(provision::iwd_file_name(w)),
            &provision::iwd_file(w),
            0o600,
        )?;
    }
    if let Some(password) = &p.user_password {
        match password {
            Some(pw) => run("chpasswd", &[], Some(&format!("{USER}:{pw}\n")))?,
            // No password: locked, so nothing can log in with one; the console logs in.
            None => run("passwd", &["-q", "-l", USER], None)?,
        }
        write_file(
            &state.join("password"),
            if password.is_some() {
                "set\n"
            } else {
                "none\n"
            },
            0o644,
        )?;
    }
    if let Some(keys) = &p.ssh_keys {
        let file = state.join("ssh").join("authorized_keys");
        let text: String = keys.iter().map(|k| format!("{k}\n")).collect();
        write_file(&file, &text, 0o644)?;
    }
    Ok(())
}

/// Writes `path` only if it does not already hold `text` with `mode`; returns whether it changed.
fn sync_file(path: &Path, text: &str, mode: u32) -> io::Result<bool> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == text {
            if let Ok(meta) = fs::metadata(path) {
                if meta.permissions().mode() & 0o777 == mode {
                    return Ok(false);
                }
            }
        }
    }
    write_file(path, text, mode)?;
    Ok(true)
}

/// Removes `path` if it exists; returns whether a file was removed.
fn remove_if_exists(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Mounts the FAT boot partition directly via syscall (no `mount` subprocess on every boot).
fn mount_boot() -> bool {
    let Ok(src) = CString::new(BOOT_PART) else {
        return false;
    };
    let Ok(dst) = CString::new(BOOT_MOUNT) else {
        return false;
    };
    let flags = libc::MS_NOEXEC | libc::MS_NOSUID | libc::MS_NODEV;
    // SAFETY: NUL-terminated C strings passed to mount(2).
    unsafe {
        libc::mount(
            src.as_ptr(),
            dst.as_ptr(),
            c"vfat".as_ptr(),
            flags,
            c"umask=077".as_ptr().cast(),
        ) == 0
    }
}

fn umount_boot() {
    if let Ok(dst) = CString::new(BOOT_MOUNT) {
        // SAFETY: NUL-terminated mount point path.
        unsafe { libc::umount2(dst.as_ptr(), 0) };
    }
}

/// The console, sudo, polkit and SSH as the state says.
/// Only writes files or calls systemctl/daemon-reload when something actually changed: on every
/// normal boot after the first, this does zero writes and spawns zero subprocesses.
fn access(state: &Path) -> io::Result<()> {
    let password = fs::read_to_string(state.join("password")).is_ok_and(|s| s.trim() == "set");
    let autologin = Path::new("/etc/systemd/system/getty@tty1.service.d/autologin.conf");
    let sudoers = Path::new("/etc/sudoers.d/wheel");
    let polkit = Path::new("/etc/polkit-1/rules.d/49-nibos-wheel.rules");
    let autologin_changed = if password {
        let changed = remove_if_exists(autologin)?;
        sync_file(sudoers, "%wheel ALL=(ALL:ALL) ALL\n", 0o440)?;
        let _ = remove_if_exists(polkit);
        changed
    } else {
        let changed = sync_file(
            autologin,
            &format!(
                "# nibos-provision: no password is set, so tty1 logs {USER} in.\n[Service]\n\
                 ExecStart=\nExecStart=-/usr/bin/agetty --autologin {USER} --noclear %I $TERM\n"
            ),
            0o644,
        )?;
        sync_file(sudoers, "%wheel ALL=(ALL:ALL) NOPASSWD: ALL\n", 0o440)?;
        sync_file(
            polkit,
            "// nibos-provision: no password is set, so wheel needs none for polkit.\n\
             polkit.addRule(function(action, subject) {\n    \
             if (subject.isInGroup(\"wheel\")) return polkit.Result.YES;\n});\n",
            0o644,
        )?;
        changed
    };
    // sshd reads authorized_keys as the user logging in, so the way to it must be open to
    // everyone (images up to db9a3c7 made the state partition's top 0700, and every key was
    // refused).
    for dir in [state.to_path_buf(), state.join("ssh")] {
        if let Ok(meta) = fs::metadata(&dir) {
            if meta.is_dir() && meta.permissions().mode() & 0o777 != 0o755 {
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    let keys = fs::read_to_string(state.join("ssh").join("authorized_keys"))
        .is_ok_and(|s| !s.trim().is_empty());
    let sshd_enabled = Path::new("/etc/systemd/system/sockets.target.wants/sshd.socket").exists();
    let sshd_changed = keys != sshd_enabled;
    if sshd_changed {
        let verb = if keys { "enable" } else { "disable" };
        run("systemctl", &[verb, "--quiet", "sshd.socket"], None)?;
    }
    if autologin_changed || sshd_changed {
        run("systemctl", &["daemon-reload"], None)?;
    }
    if sshd_changed {
        // Enabling only changes later boots: this boot's jobs were decided before this ran, so the
        // socket is started (or stopped) now too, or SSH would wait for a restart after the first
        // boot. --no-block: this unit is ordered before sshd.socket, so waiting would deadlock.
        let now = if keys { "start" } else { "stop" };
        run("systemctl", &[now, "--no-block", "sshd.socket"], None)?;
    }
    Ok(())
}

/// Applies `provision.toml` if present; returns whether anything on `boot` was written or removed.
fn provision_file(boot: &Path, state: &Path) -> io::Result<bool> {
    let file = boot.join("provision.toml");
    // A Mac leaves "._provision.toml" (AppleDouble: the file's extended attributes) beside it
    // on a FAT volume. nibOS never reads it; wiped with the file it belongs to.
    let apple = boot.join("._provision.toml");
    let mut dirty = false;
    if apple.exists() && !file.exists() {
        let _ = wipe(&apple);
        dirty = true;
    }
    if !file.exists() {
        return Ok(dirty);
    }
    let text = String::from_utf8_lossy(&fs::read(&file)?).into_owned();
    match provision::parse(&text) {
        Ok(p) => {
            apply(&p, state)?;
            wipe(&file)?;
            if apple.exists() {
                let _ = wipe(&apple);
            }
            let _ = fs::remove_file(boot.join("provision.error.txt"));
            eprintln!("nibos-provision: applied provision.toml");
        }
        Err(why) => {
            let existing: BTreeMap<String, ()> = fs::read_dir(boot)?
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), ()))
                .collect();
            let kept = boot.join(provision::rejected_name(&existing));
            fs::rename(&file, &kept)?;
            fs::write(
                boot.join("provision.error.txt"),
                format!(
                    "provision.toml was not applied: {why}.\r\nIt is kept as {}. Fix it, save it \
                     as provision.toml again and restart.\r\n",
                    kept.file_name().expect("a name").to_string_lossy()
                ),
            )?;
            eprintln!("nibos-provision: provision.toml refused: {why}");
        }
    }
    Ok(true)
}

fn main() {
    let boot = PathBuf::from(BOOT_MOUNT);
    let state = PathBuf::from(STATE);
    let _ = fs::create_dir_all(&boot);
    let mounted = mount_boot();
    let result = if mounted {
        let r = provision_file(&boot, &state);
        if matches!(r, Ok(true)) {
            let _ = Command::new("sync").status();
        }
        umount_boot();
        r.map(|_| ())
    } else {
        eprintln!("nibos-provision: no {BOOT_PART}; nothing to read");
        Ok(())
    };
    let result = result
        .and_then(|()| restore_timezone(&state))
        .and_then(|()| access(&state));
    if let Err(e) = result {
        eprintln!("nibos-provision: {e}");
        exit(1);
    }
}
