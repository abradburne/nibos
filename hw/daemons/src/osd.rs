//! What the volume keys just set, announced for the indicator the program on the screens draws
//! over itself (nibkit's `levels` reads it, and its `menu::level` draws it): after each step,
//! `nibos-input` (Menu + volume, the brightness) and `nibos-audio` (the volume keys) write the
//! new level to a file in [`DIR`] named for what changed, `brightness` or `volume`, holding the
//! percent and a newline. Written aside and renamed in, so a reader never sees half of one, and
//! a watch on the folder (inotify `IN_MOVED_TO`) sees each step. Only the keys announce: an
//! app's slider or a screensaver changes a level without a word.
//!
//! The folder is made at boot (tmpfiles' `nibos-osd.conf`), the `video` group's to write, as
//! `nibos-audio` runs as the user.

use std::io;
use std::path::Path;

/// Where the levels are announced.
pub const DIR: &str = "/run/nibos/osd";

/// What a level is of, as its file is named.
pub const BRIGHTNESS: &str = "brightness";
pub const VOLUME: &str = "volume";

/// Announces `what` (a file name above) at `percent` in [`DIR`].
pub fn announce(what: &str, percent: u32) -> io::Result<()> {
    announce_in(Path::new(DIR), what, percent)
}

/// Announces `what` at `percent` (held at 100) in `dir`: written to a hidden file of this
/// process's own, then renamed over `what`'s.
pub fn announce_in(dir: &Path, what: &str, percent: u32) -> io::Result<()> {
    let aside = dir.join(format!(".{what}.{}", std::process::id()));
    std::fs::write(&aside, format!("{}\n", percent.min(100)))?;
    std::fs::rename(&aside, dir.join(what)).inspect_err(|_| {
        let _ = std::fs::remove_file(&aside);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_whole_and_nothing_left_aside() {
        let dir = std::env::temp_dir().join(format!("nibos-osd-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        announce_in(&dir, VOLUME, 40).unwrap();
        announce_in(&dir, BRIGHTNESS, 250).unwrap();
        announce_in(&dir, VOLUME, 45).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(VOLUME)).unwrap(), "45\n");
        assert_eq!(
            std::fs::read_to_string(dir.join(BRIGHTNESS)).unwrap(),
            "100\n"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert!(announce_in(&dir.join("missing"), VOLUME, 1).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
