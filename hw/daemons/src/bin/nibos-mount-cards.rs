//! nibos-mount-cards: mounts /mnt/roms (NIBOS-DATA / ROMS) and /mnt/ds (TF2 games card)
//! directly from devtmpfs using sysfs uevent metadata, without waiting ~1.5 s for udev's
//! coldplug and blkid symlinks. Falls back silently if a partition isn't recognised or an
//! automount is configured in /etc/fstab (letting udev / systemd handle it as usual).

use std::{
    ffi::CString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};

fn is_mounted(target: &str) -> bool {
    fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .any(|l| l.split(' ').nth(1) == Some(target))
}

fn fstab_line(target: &str) -> Option<String> {
    fs::read_to_string("/etc/fstab")
        .unwrap_or_default()
        .lines()
        .find(|l| {
            let l = l.trim();
            !l.starts_with('#') && l.split_whitespace().nth(1) == Some(target)
        })
        .map(str::to_owned)
}

/// Extracts mount-option string for libc::mount (uid/gid/fmask/dmask) from an fstab line.
fn mount_data(fstab: Option<&str>) -> String {
    let mut kept = Vec::new();
    if let Some(opts) = fstab.and_then(|l| l.split_whitespace().nth(3)) {
        for opt in opts.split(',') {
            if opt.starts_with("uid=")
                || opt.starts_with("gid=")
                || opt.starts_with("fmask=")
                || opt.starts_with("dmask=")
                || opt.starts_with("iocharset=")
                || opt.starts_with("codepage=")
            {
                kept.push(opt);
            }
        }
    }
    if kept.is_empty() {
        "uid=1000,gid=1000,fmask=0022,dmask=0022".into()
    } else {
        kept.join(",")
    }
}

fn read_uevent_val(dir: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join("uevent")).ok()?;
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(str::to_owned)
}

fn root_disk() -> Option<PathBuf> {
    let dev = fs::metadata("/").ok()?.dev();
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    fs::canonicalize(format!("/sys/dev/block/{major}:{minor}/..")).ok()
}

fn try_mount(dev: &str, target: &str, fstypes: &[&str], data: &str) -> bool {
    let _ = fs::create_dir_all(target);
    let Ok(c_dev) = CString::new(dev) else {
        return false;
    };
    let Ok(c_tgt) = CString::new(target) else {
        return false;
    };
    let Ok(c_data) = CString::new(data) else {
        return false;
    };
    let mut need_modprobe = false;
    for &fs in fstypes {
        let Ok(c_fs) = CString::new(fs) else { continue };
        // SAFETY: valid NUL-terminated strings for source, target, fstype and options.
        let rc = unsafe {
            libc::mount(
                c_dev.as_ptr(),
                c_tgt.as_ptr(),
                c_fs.as_ptr(),
                libc::MS_NOATIME,
                c_data.as_ptr().cast(),
            )
        };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error().raw_os_error();
        if err == Some(libc::ENODEV) {
            need_modprobe = true;
        }
    }
    if need_modprobe {
        // Fallback when filesystem driver is a loadable module (e.g. vfat).
        let opts = format!("noatime,{data}");
        for &fs in fstypes {
            if Command::new("/usr/bin/mount")
                .args(["-t", fs, "-o", &opts, dev, target])
                .status()
                .is_ok_and(|s| s.success())
            {
                return true;
            }
        }
    }
    false
}

fn mount_roms() {
    if is_mounted("/mnt/roms") {
        return;
    }
    let line = fstab_line("/mnt/roms");
    // Never race an fstab x-systemd.automount entry (autofs would stack over a direct mount).
    if line
        .as_deref()
        .is_some_and(|l| l.contains("x-systemd.automount"))
    {
        return;
    }
    let Some(disk) = root_disk() else { return };
    let Some(disk_name) = disk.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let data = mount_data(line.as_deref());
    for entry in fs::read_dir(&disk).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(disk_name) {
            continue;
        }
        let partname = read_uevent_val(&entry.path(), "PARTNAME").unwrap_or_default();
        let fstypes: &[&str] = match partname.as_str() {
            "nibos-data" => &["exfat", "vfat"],
            "ROMS" => &["vfat", "exfat"],
            _ => continue,
        };
        let dev = format!("/dev/{name}");
        if Path::new(&dev).exists() && try_mount(&dev, "/mnt/roms", fstypes, &data) {
            return;
        }
    }
}

fn mount_games_card() {
    if is_mounted("/mnt/ds") {
        return;
    }
    let disk = Path::new("/sys/block/mmcblk1");
    if !disk.exists() {
        return;
    }
    let line = fstab_line("/mnt/ds");
    let data = mount_data(line.as_deref());
    let mut parts: Vec<(u8, String)> = Vec::new();
    for entry in fs::read_dir(disk).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("mmcblk1p") {
            continue;
        }
        let partname = read_uevent_val(&entry.path(), "PARTNAME").unwrap_or_default();
        let prio = match partname.as_str() {
            "DS" => 0,
            "NIBOS-DATA" | "nibos-data" | "ROMS" => 1,
            _ => 2,
        };
        parts.push((prio, name));
    }
    parts.sort();
    let candidate = if parts.len() == 1 || parts.first().is_some_and(|(p, _)| *p < 2) {
        Some(parts[0].1.clone())
    } else if parts.is_empty() {
        Some("mmcblk1".to_owned())
    } else {
        // Multi-partition card with no recognised PARTNAME: let udev's blkid rules pick.
        None
    };
    if let Some(name) = candidate {
        let dev = format!("/dev/{name}");
        if Path::new(&dev).exists() {
            let _ = try_mount(&dev, "/mnt/ds", &["exfat", "vfat"], &data);
        }
    }
}

fn main() {
    mount_roms();
    mount_games_card();
}
