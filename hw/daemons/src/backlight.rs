//! The screens' backlights, as nibOS offers them to programs without root
//! ([`hw/README.md`], "Brightness"; the pattern in
//! `docs/engineering/device-interface.md`):
//!
//! - each backlight is named by the screen it lights, from the board's
//!   description (`top:backlight1 bottom:backlight0`), and linked as
//!   `/run/nibos/backlight/<screen>`, writable by the video group;
//! - levels are kept per backlight in `/var/lib/nibos/backlight/<name>` and
//!   set back at boot;
//! - nothing nibOS sets goes below the board's floor; a program that wants a
//!   screen dark powers it down with `bl_power` instead (also writable by the
//!   video group), and any brightness nibOS sets turns it back on;
//! - programs speak in percent: 0 is the floor, 100 the maximum, and equal
//!   steps look equal. On a backlight whose kernel scale is already
//!   perceptual (`non-linear`, as pwm-backlight's generated table is) that is
//!   linear between floor and maximum; on a `linear` one it is squared.
//!
//! Used by `nibos-backlight` (udev, the save unit, `nib brightness`) and by
//! `nibos-input` (Menu + volume).
//!
//! [`hw/README.md`]: ../../README.md

use crate::{board::Board, steps};
use std::{
    fs, io,
    os::unix::fs::{chown, symlink, PermissionsExt},
    path::{Path, PathBuf},
};

/// How the kernel maps a backlight's raw levels to light (`scale` in sysfs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// Raw levels are already perceptual.
    Perceptual,
    /// Raw levels are proportional to light (or unknown).
    Linear,
}

impl Scale {
    fn from_sysfs(text: &str) -> Scale {
        if text.trim() == "non-linear" {
            Scale::Perceptual
        } else {
            Scale::Linear
        }
    }
}

/// One backlight's range, as the percent mapping needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub max: u32,
    pub floor: u32,
    pub scale: Scale,
}

impl Range {
    /// The range of a backlight with `max` levels and the board's floor in
    /// percent of it; the floor is at least 1, so never off.
    pub fn new(max: u32, min_percent: u32, scale: Scale) -> Range {
        let floor = (u64::from(max) * u64::from(min_percent) / 100) as u32;
        Range {
            max,
            floor: floor.clamp(1, max.max(1)),
            scale,
        }
    }

    /// A raw level kept between the floor and the maximum.
    pub fn clamp(&self, raw: u32) -> u32 {
        raw.clamp(self.floor, self.max)
    }

    /// The raw level for a percent (0-100, more is 100).
    pub fn raw(&self, percent: u32) -> u32 {
        let f = f64::from(percent.min(100)) / 100.0;
        let f = match self.scale {
            Scale::Perceptual => f,
            Scale::Linear => f * f,
        };
        let span = f64::from(self.max - self.floor);
        self.floor + (span * f).round() as u32
    }

    /// The percent a raw level shows as (the inverse of [`Range::raw`]).
    pub fn percent(&self, raw: u32) -> u32 {
        let span = self.max - self.floor;
        if span == 0 {
            return 100;
        }
        let f = f64::from(self.clamp(raw) - self.floor) / f64::from(span);
        let f = match self.scale {
            Scale::Perceptual => f,
            Scale::Linear => f.sqrt(),
        };
        (f * 100.0).round() as u32
    }
}

/// One backlight as found: its screen, its kernel name, its level and range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlight {
    pub screen: String,
    pub name: String,
    pub raw: u32,
    pub range: Range,
}

impl Backlight {
    pub fn percent(&self) -> u32 {
        self.range.percent(self.raw)
    }
}

/// Where the backlights and their state are: the real places, or a test's.
#[derive(Clone, Debug)]
pub struct Backlights {
    /// The kernel's backlights (`/sys/class/backlight`).
    pub sys: PathBuf,
    /// The screens' names for them (`/run/nibos/backlight`).
    pub run: PathBuf,
    /// The saved levels (`/var/lib/nibos/backlight`).
    pub state: PathBuf,
    /// `screen:name` pairs from the board.
    pub screens: Vec<(String, String)>,
    pub min_percent: u32,
}

/// What `Backlights::set` and `step` act on.
pub enum Target<'a> {
    All,
    Screen(&'a str),
}

impl Target<'_> {
    pub fn parse(text: &str) -> Target<'_> {
        if text == "all" {
            Target::All
        } else {
            Target::Screen(text)
        }
    }
}

fn read_u32(path: &Path) -> io::Result<u32> {
    fs::read_to_string(path)?.trim().parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

impl Backlights {
    /// The real places, for the board.
    pub fn for_board(board: &Board) -> Backlights {
        Backlights {
            sys: "/sys/class/backlight".into(),
            run: "/run/nibos/backlight".into(),
            state: "/var/lib/nibos/backlight".into(),
            screens: board.backlights.clone(),
            min_percent: board.backlight_min_percent,
        }
    }

    /// The screen a backlight lights: the board's name for it, or its own.
    pub fn screen_of<'a>(&'a self, name: &'a str) -> &'a str {
        self.screens
            .iter()
            .find(|(_, n)| n == name)
            .map_or(name, |(s, _)| s.as_str())
    }

    fn read(&self, name: &str) -> io::Result<Backlight> {
        let dir = self.sys.join(name);
        let scale = fs::read_to_string(dir.join("scale")).unwrap_or_default();
        let range = Range::new(
            read_u32(&dir.join("max_brightness"))?,
            self.min_percent,
            Scale::from_sysfs(&scale),
        );
        Ok(Backlight {
            screen: self.screen_of(name).to_owned(),
            name: name.to_owned(),
            raw: read_u32(&dir.join("brightness"))?,
            range,
        })
    }

    /// Every backlight there is, the board's screens first, in its order.
    pub fn all(&self) -> io::Result<Vec<Backlight>> {
        let mut names: Vec<String> = fs::read_dir(&self.sys)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort_by_key(|n| {
            (
                self.screens
                    .iter()
                    .position(|(_, b)| b == n)
                    .unwrap_or(usize::MAX),
                n.clone(),
            )
        });
        names.iter().map(|n| self.read(n)).collect()
    }

    /// The backlights a target names; an unknown screen is an error.
    pub fn select(&self, target: &Target) -> io::Result<Vec<Backlight>> {
        let all = self.all()?;
        match target {
            Target::All => Ok(all),
            Target::Screen(s) => {
                let one: Vec<_> = all.into_iter().filter(|b| b.screen == *s).collect();
                if one.is_empty() {
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("no screen '{s}'"),
                    ))
                } else {
                    Ok(one)
                }
            }
        }
    }

    /// Set a level, and turn the backlight on if a program powered it down
    /// (`bl_power` 4, the "screen off" knob): a brightness set through nibOS,
    /// Menu + volume included, always lights the screen.
    fn write(&self, b: &Backlight, raw: u32) -> io::Result<()> {
        let dir = self.sys.join(&b.name);
        fs::write(dir.join("brightness"), format!("{}\n", b.range.clamp(raw)))?;
        if read_u32(&dir.join("bl_power")).is_ok_and(|p| p != 0) {
            fs::write(dir.join("bl_power"), "0\n")?;
        }
        Ok(())
    }

    /// Set the target's backlights to a percent, and save them.
    pub fn set(&self, target: &Target, percent: u32) -> io::Result<Vec<Backlight>> {
        let chosen = self.select(target)?;
        for b in &chosen {
            self.write(b, b.range.raw(percent))?;
        }
        self.save()?;
        self.select(target)
    }

    /// Move the target's backlights by `delta` percent each (each from its
    /// own level), and save them.
    pub fn shift(&self, target: &Target, delta: i32) -> io::Result<Vec<Backlight>> {
        self.each(target, |p| (p as i32 + delta).clamp(0, 100) as u32)
    }

    /// Step the target's backlights up (1) or down (-1), each onto the next multiple of 5%
    /// ([`crate::steps`]; Menu + volume), and save them.
    pub fn step(&self, target: &Target, dir: i32) -> io::Result<Vec<Backlight>> {
        self.each(target, |p| steps::next(p, dir))
    }

    fn each(&self, target: &Target, to: impl Fn(u32) -> u32) -> io::Result<Vec<Backlight>> {
        let chosen = self.select(target)?;
        for b in &chosen {
            self.write(b, b.range.raw(to(b.percent())))?;
        }
        self.save()?;
        self.select(target)
    }

    /// Keep every backlight's level, between floor and maximum. The files stay
    /// writable for the state directory's group (video), so `nib brightness`
    /// run by a user can save too.
    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(&self.state)?;
        for b in self.all()? {
            let path = self.state.join(&b.name);
            fs::write(&path, format!("{}\n", b.range.clamp(b.raw)))?;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o664));
        }
        Ok(())
    }

    /// For udev, when a backlight appears: its screen's link, write access for
    /// `group` to `brightness` and `bl_power`, and its saved level back (and
    /// so powered on).
    pub fn add(&self, name: &str, group: Option<u32>) -> io::Result<()> {
        let b = self.read(name)?;
        fs::create_dir_all(&self.run)?;
        let link = self.run.join(&b.screen);
        let _ = fs::remove_file(&link);
        symlink(self.sys.join(name), &link)?;
        if let Some(gid) = group {
            for file in ["brightness", "bl_power"] {
                let path = self.sys.join(name).join(file);
                if path.exists() {
                    chown(&path, None, Some(gid))?;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o664))?;
                }
            }
        }
        if let Ok(saved) = read_u32(&self.state.join(name)) {
            self.write(&b, saved)?;
        }
        Ok(())
    }
}

/// A group's id from /etc/group.
pub fn group_id(group_file: &str, name: &str) -> Option<u32> {
    group_file.lines().find_map(|l| {
        let mut f = l.split(':');
        (f.next()? == name).then(|| f.nth(1)?.parse().ok())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERCEPTUAL: Range = Range {
        max: 1666,
        floor: 33,
        scale: Scale::Perceptual,
    };

    #[test]
    fn floor_is_a_share_of_the_maximum_and_never_zero() {
        assert_eq!(Range::new(1666, 2, Scale::Perceptual).floor, 33);
        assert_eq!(Range::new(10, 2, Scale::Perceptual).floor, 1);
        assert_eq!(Range::new(1666, 0, Scale::Perceptual).floor, 1);
    }

    #[test]
    fn perceptual_scale_maps_percent_linearly() {
        assert_eq!(PERCEPTUAL.raw(0), 33);
        assert_eq!(PERCEPTUAL.raw(100), 1666);
        assert_eq!(PERCEPTUAL.raw(50), 33 + 817);
        assert_eq!(PERCEPTUAL.raw(250), 1666);
        for p in 0..=100 {
            assert_eq!(PERCEPTUAL.percent(PERCEPTUAL.raw(p)), p);
        }
    }

    #[test]
    fn linear_scale_maps_percent_squared() {
        let r = Range::new(1000, 0, Scale::Linear);
        assert_eq!(r.raw(50), 1 + (999.0f64 * 0.25).round() as u32);
        assert_eq!(r.raw(100), 1000);
        // Squaring leaves few raw levels at the dark end (1% to 3% are all the
        // floor here), so round trips hold from 5% up; the levels never go
        // down as the percent goes up.
        for p in 5..=100 {
            assert!(r.percent(r.raw(p)).abs_diff(p) <= 1, "{p}");
        }
        assert!((0..100).all(|p| r.raw(p) <= r.raw(p + 1)));
    }

    #[test]
    fn levels_below_the_floor_read_as_zero_and_are_raised() {
        assert_eq!(PERCEPTUAL.percent(0), 0);
        assert_eq!(PERCEPTUAL.clamp(0), 33);
        assert_eq!(PERCEPTUAL.clamp(5000), 1666);
    }

    #[test]
    fn group_ids() {
        let g = "root:x:0:\nwheel:x:998:alice\nvideo:x:983:alice,bob\n";
        assert_eq!(group_id(g, "video"), Some(983));
        assert_eq!(group_id(g, "audio"), None);
    }

    /// A fake /sys/class/backlight, /run and /var/lib in a temporary directory.
    fn fake(tag: &str) -> Backlights {
        let root =
            std::env::temp_dir().join(format!("nibos-backlight-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (name, level) in [("backlight0", 833), ("backlight1", 833), ("other", 5)] {
            let d = root.join("sys").join(name);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("brightness"), format!("{level}\n")).unwrap();
            let max = if name == "other" { 10 } else { 1666 };
            fs::write(d.join("max_brightness"), format!("{max}\n")).unwrap();
            fs::write(d.join("scale"), "non-linear\n").unwrap();
            fs::write(d.join("bl_power"), "0\n").unwrap();
        }
        Backlights {
            sys: root.join("sys"),
            run: root.join("run"),
            state: root.join("state"),
            screens: vec![
                ("top".into(), "backlight1".into()),
                ("bottom".into(), "backlight0".into()),
            ],
            min_percent: 2,
        }
    }

    #[test]
    fn screens_come_first_and_by_name() {
        let b = fake("order");
        let names: Vec<_> = b.all().unwrap().into_iter().map(|b| b.screen).collect();
        assert_eq!(names, ["top", "bottom", "other"]);
        assert!(b.select(&Target::Screen("side")).is_err());
    }

    #[test]
    fn set_step_save_and_add() {
        let b = fake("set");
        let top = b.set(&Target::Screen("top"), 0).unwrap();
        assert_eq!(top[0].raw, 33, "0 is the floor, not off");
        assert_eq!(read_u32(&b.state.join("backlight1")).unwrap(), 33);
        assert_eq!(read_u32(&b.state.join("backlight0")).unwrap(), 833);

        let both = b.shift(&Target::All, 10).unwrap();
        assert_eq!(both[0].percent(), 10);
        // The bottom moves from its own level: 833 of 33..1666 is 49%.
        assert_eq!(both[1].percent(), 59);
        // A step (Menu + volume) lands each on the next multiple of 5.
        let both = b.step(&Target::All, 1).unwrap();
        assert_eq!((both[0].percent(), both[1].percent()), (15, 60));
        let both = b.step(&Target::All, -1).unwrap();
        assert_eq!((both[0].percent(), both[1].percent()), (10, 55));

        // A dark level written past nibOS is raised to the floor when saved.
        fs::write(b.sys.join("backlight0/brightness"), "0\n").unwrap();
        b.save().unwrap();
        assert_eq!(read_u32(&b.state.join("backlight0")).unwrap(), 33);

        // A screen a program turned off comes back on with any change.
        fs::write(b.sys.join("backlight1/bl_power"), "4\n").unwrap();
        b.step(&Target::All, 0).unwrap();
        assert_eq!(read_u32(&b.sys.join("backlight1/bl_power")).unwrap(), 0);

        // A restart: the kernel's default, then udev's add sets the saved level.
        fs::write(b.sys.join("backlight1/brightness"), "833\n").unwrap();
        b.add("backlight1", None).unwrap();
        assert_eq!(
            read_u32(&b.sys.join("backlight1/brightness")).unwrap(),
            read_u32(&b.state.join("backlight1")).unwrap()
        );
        assert_eq!(
            fs::read_link(b.run.join("top")).unwrap(),
            b.sys.join("backlight1")
        );
    }
}
