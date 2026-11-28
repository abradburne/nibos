//! USB mass storage: the data cards as drives on a computer, while Menu is held as the cable
//! goes in (`nibos-usb-storage`). The logic, kept apart from the device I/O so it can be tested
//! anywhere:
//!
//! - [`plugged`]: whether a charger event is the cable going in;
//! - [`Card`] and [`cards`]: which cards are exported, from `/etc/fstab`;
//! - [`Gadget`]: the configfs gadget, as the files to write;
//! - [`watch`]: when to give the cards back;
//! - [`exfat_dirty`]: whether a card was left mid-write and wants a check.

use std::path::{Path, PathBuf};

/// Where the configfs gadget is made.
pub const GADGET: &str = "/sys/kernel/config/usb_gadget/nibos";
/// The charger's last state (`1` or `0`), kept by `--event` so a change is told from a plug.
pub const LAST_ONLINE: &str = "/run/nibos/usb-storage-charger";

/// Whether the charger's event is a plug: online now and not at the last event. With no last
/// state (the first event since boot recorded none), online counts as a plug.
pub fn plugged(last: Option<bool>, online: bool) -> bool {
    online && last != Some(true)
}

/// A data card: an `/etc/fstab` entry under `/mnt` with a FAT or exFAT file system (or `auto`,
/// which the card's own file system then decides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The fstab entry's first field (`LABEL=DS`, `UUID=…`, `/dev/…`).
    pub spec: String,
    /// Where it is mounted (`/mnt/ds`).
    pub mount_point: String,
    /// Its file system type (`exfat`, `vfat`).
    pub fs: String,
}

impl Card {
    /// The block device the spec names, as udev links it (the path may not exist: no card).
    pub fn device(&self) -> PathBuf {
        let by = |dir: &str, v: &str| PathBuf::from(format!("/dev/disk/{dir}/{v}"));
        match self.spec.split_once('=') {
            Some(("LABEL", v)) => by("by-label", v),
            Some(("UUID", v)) => by("by-uuid", v),
            Some(("PARTLABEL", v)) => by("by-partlabel", v),
            Some(("PARTUUID", v)) => by("by-partuuid", v),
            _ => PathBuf::from(&self.spec),
        }
    }

    /// Its mount unit and automount unit, as systemd names them.
    pub fn units(&self) -> (String, String) {
        let name = unit_name(&self.mount_point);
        (format!("{name}.mount"), format!("{name}.automount"))
    }

    /// Its name as the computer shows it: the label, else the mount point's last part.
    pub fn name(&self) -> String {
        match self.spec.split_once('=') {
            Some(("LABEL", v)) => v.to_string(),
            _ => Path::new(&self.mount_point)
                .file_name()
                .map(|n| n.to_string_lossy().to_uppercase())
                .unwrap_or_default(),
        }
    }

    /// What the computer is told the drive is (SCSI INQUIRY: 8 vendor, 16 product, 4 revision).
    pub fn inquiry(&self) -> String {
        format!("{:<8}{:<16.16}{:<4}", "nibOS", self.name(), "1")
    }
}

/// A path as systemd names its mount unit (`/mnt/ds` → `mnt-ds`): `/` as `-`, anything but
/// ASCII letters, digits, `_` and a dot not first as `\xNN`.
pub fn unit_name(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return "-".into();
    }
    let mut out = String::new();
    for (i, b) in trimmed.bytes().enumerate() {
        match b {
            b'/' => out.push('-'),
            b'.' if i > 0 => out.push('.'),
            b if b.is_ascii_alphanumeric() || b == b'_' => out.push(b as char),
            b => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

/// The data cards in an fstab, in its order: entries mounted under `/mnt` whose type is
/// `exfat`, `vfat` or `auto` (the games card: whatever the card in TF2 holds). The system's own partitions (the root, the state partition) never are.
pub fn cards(fstab: &str) -> Vec<Card> {
    fstab
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 3 && f[1].starts_with("/mnt/") && matches!(f[2], "exfat" | "vfat" | "auto"))
                .then(|| Card {
                    spec: f[0].into(),
                    mount_point: f[1].into(),
                    fs: f[2].into(),
                })
        })
        .collect()
}

/// The gadget: one mass storage function with a LUN per card, removable (the computer can
/// eject it, and a card can be given to it after it enumerates), not a CD-ROM.
pub struct Gadget {
    /// The device's name for the computer (`Anbernic RG DS+`).
    pub product: String,
    /// Its serial number: the same every time, so the computer knows it again.
    pub serial: String,
    /// One per LUN, in order: what INQUIRY says each one is.
    pub luns: Vec<String>,
}

impl Gadget {
    /// The directories to make, in order, under [`GADGET`].
    pub fn dirs(&self) -> Vec<String> {
        let mut d = vec![
            "strings/0x409".into(),
            "configs/c.1".into(),
            "configs/c.1/strings/0x409".into(),
            "functions/mass_storage.0".into(),
        ];
        // lun.0 comes with the function.
        d.extend((1..self.luns.len()).map(|i| format!("functions/mass_storage.0/lun.{i}")));
        d
    }

    /// The attributes to write, in order, under [`GADGET`]. The LUNs start with no file: the
    /// cards go in once a computer has enumerated the device.
    pub fn files(&self) -> Vec<(String, String)> {
        let mut f: Vec<(String, String)> = [
            // The Linux Foundation's multifunction composite gadget, as other gadgets use.
            ("idVendor", "0x1d6b"),
            ("idProduct", "0x0104"),
            ("bcdDevice", "0x0100"),
            ("bcdUSB", "0x0200"),
            ("strings/0x409/manufacturer", "nibOS"),
            ("strings/0x409/product", &self.product),
            ("strings/0x409/serialnumber", &self.serial),
            ("configs/c.1/strings/0x409/configuration", "Mass storage"),
            // Asks the computer for the most USB 2 allows; charging doesn't depend on it.
            ("configs/c.1/MaxPower", "500"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        for (i, inquiry) in self.luns.iter().enumerate() {
            let lun = format!("functions/mass_storage.0/lun.{i}");
            for (attr, v) in [
                ("removable", "1"),
                ("cdrom", "0"),
                ("ro", "0"),
                // Honour the computer's Force Unit Access: its writes reach the card.
                ("nofua", "0"),
                ("inquiry_string", inquiry.as_str()),
            ] {
                f.push((format!("{lun}/{attr}"), v.into()));
            }
        }
        f
    }

    /// The directories to remove when it is taken down, in order (after UDC is emptied and
    /// the configuration's link removed). lun.0 goes with the function.
    pub fn teardown_dirs(&self) -> Vec<String> {
        let mut d: Vec<String> = (1..self.luns.len())
            .rev()
            .map(|i| format!("functions/mass_storage.0/lun.{i}"))
            .collect();
        d.extend(
            [
                "configs/c.1/strings/0x409",
                "configs/c.1",
                "functions/mass_storage.0",
                "strings/0x409",
            ]
            .map(String::from),
        );
        d
    }
}

/// What the cable and the computer are doing, read every round while the cards are out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    /// The charger is online (the cable is in).
    pub online: bool,
    /// How many LUNs still have their card (the computer ejects one by emptying it).
    pub luns_with_card: usize,
    /// A stop was asked for (SIGTERM: shutdown).
    pub term: bool,
}

/// Why the cards come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    /// The cable came out.
    Unplugged,
    /// The computer ejected every drive.
    Ejected,
    /// The system is shutting down.
    Stopped,
}

/// Whether to give the cards back now, and why.
pub fn watch(s: Seen) -> Option<Over> {
    if s.term {
        Some(Over::Stopped)
    } else if !s.online {
        Some(Over::Unplugged)
    } else if s.luns_with_card == 0 {
        Some(Over::Ejected)
    } else {
        None
    }
}

/// Whether an exFAT boot sector has VolumeDirty set (VolumeFlags, offset 106, bit 1): the
/// computer had it mounted and didn't finish (unplugged without ejecting, or mid-copy). None
/// if it isn't an exFAT boot sector.
pub fn exfat_dirty(boot: &[u8]) -> Option<bool> {
    if boot.len() < 108 || &boot[3..11] != b"EXFAT   " {
        return None;
    }
    let flags = u16::from_le_bytes([boot[106], boot[107]]);
    Some(flags & 0b10 != 0)
}

/// What the screens say while the cards are out: a title for the top screen and lines for the
/// bottom one.
pub fn message(drives: &[String]) -> (&'static str, Vec<String>) {
    let mut lines = vec!["Unplug to return".to_string()];
    lines.push(match drives.len() {
        0 => String::new(),
        1 => format!("{} is on the computer", drives[0]),
        _ => format!("{} are on the computer", drives.join(" and ")),
    });
    lines.push("Eject before unplugging, so nothing is lost".into());
    ("Connected to a computer", lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plug_is_a_change_to_online() {
        assert!(plugged(Some(false), true));
        assert!(plugged(None, true));
        assert!(
            !plugged(Some(true), true),
            "a charging-state change, still in"
        );
        assert!(!plugged(Some(true), false));
        assert!(!plugged(None, false));
    }

    const FSTAB: &str = "\
PARTLABEL=nibos-state  /var/lib/nibos/state  ext4  rw,noatime  0 0
LABEL=NIBOS-DATA  /mnt/roms  exfat  rw,noatime,nofail,x-systemd.automount  0 0
LABEL=DS    /mnt/ds    exfat rw,noatime,nofail,noauto,x-systemd.device-bound  0 0
/dev/nibos-games-card  /mnt/games  auto  rw,noauto 0 0
# LABEL=OLD /mnt/old vfat rw 0 0
/dev/mmcblk0p9 /mnt/scratch ext4 rw 0 0
";

    #[test]
    fn cards_are_fat_under_mnt() {
        let c = cards(FSTAB);
        assert_eq!(c.len(), 3);
        assert_eq!(c[2].fs, "auto");
        assert_eq!(c[2].name(), "GAMES");
        assert_eq!(c[0].mount_point, "/mnt/roms");
        assert_eq!(
            c[0].device(),
            PathBuf::from("/dev/disk/by-label/NIBOS-DATA")
        );
        assert_eq!(c[1].spec, "LABEL=DS");
        assert_eq!(
            c[1].units(),
            ("mnt-ds.mount".to_string(), "mnt-ds.automount".to_string())
        );
        assert_eq!(c[1].inquiry(), "nibOS   DS              1   ");
        assert_eq!(c[1].inquiry().len(), 28);
        assert_eq!(c[0].name(), "NIBOS-DATA");
        let dev = Card {
            spec: "/dev/mmcblk1p1".into(),
            mount_point: "/mnt/ds".into(),
            fs: "exfat".into(),
        };
        assert_eq!(dev.name(), "DS");
    }

    #[test]
    fn unit_names_escape() {
        assert_eq!(unit_name("/mnt/roms"), "mnt-roms");
        assert_eq!(unit_name("/mnt/my-card/"), "mnt-my\\x2dcard");
        assert_eq!(unit_name("/"), "-");
    }

    #[test]
    fn gadget_has_a_lun_per_card() {
        let g = Gadget {
            product: "Anbernic RG DS+".into(),
            serial: "abc".into(),
            luns: vec!["a".into(), "b".into()],
        };
        let dirs = g.dirs();
        assert!(dirs.contains(&"functions/mass_storage.0/lun.1".to_string()));
        assert!(!dirs.iter().any(|d| d.ends_with("lun.0")));
        let files = g.files();
        for lun in ["lun.0", "lun.1"] {
            let get = |a: &str| {
                files
                    .iter()
                    .find(|(p, _)| p == &format!("functions/mass_storage.0/{lun}/{a}"))
                    .map(|(_, v)| v.as_str())
            };
            assert_eq!(get("removable"), Some("1"));
            assert_eq!(get("cdrom"), Some("0"));
            assert_eq!(get("ro"), Some("0"));
        }
        assert!(!files.iter().any(|(p, _)| p.ends_with("/file")));
        let down = g.teardown_dirs();
        assert_eq!(down[0], "functions/mass_storage.0/lun.1");
        assert_eq!(down.last().unwrap(), "strings/0x409");
    }

    #[test]
    fn watch_gives_back_on_unplug_eject_and_stop() {
        let s = Seen {
            online: true,
            luns_with_card: 2,
            term: false,
        };
        assert_eq!(watch(s), None);
        assert_eq!(
            watch(Seen {
                luns_with_card: 1,
                ..s
            }),
            None
        );
        assert_eq!(watch(Seen { online: false, ..s }), Some(Over::Unplugged));
        assert_eq!(
            watch(Seen {
                luns_with_card: 0,
                ..s
            }),
            Some(Over::Ejected)
        );
        assert_eq!(
            watch(Seen {
                term: true,
                online: false,
                ..s
            }),
            Some(Over::Stopped)
        );
    }

    #[test]
    fn exfat_dirty_flag() {
        let mut b = vec![0u8; 512];
        assert_eq!(exfat_dirty(&b), None);
        b[3..11].copy_from_slice(b"EXFAT   ");
        assert_eq!(exfat_dirty(&b), Some(false));
        b[106] = 0b10;
        assert_eq!(exfat_dirty(&b), Some(true));
        b[106] = 0b100; // MediaFailure alone
        assert_eq!(exfat_dirty(&b), Some(false));
    }

    #[test]
    fn message_names_the_drives() {
        let (title, lines) = message(&["DS".into(), "ROMS".into()]);
        assert_eq!(title, "Connected to a computer");
        assert_eq!(lines[0], "Unplug to return");
        assert!(lines[1].contains("DS and ROMS"));
    }
}
