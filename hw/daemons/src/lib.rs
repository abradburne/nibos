//! The nibOS hardware daemons' logic, kept apart from their device I/O so it
//! can be tested anywhere:
//!
//! - [`ab`]: the A/B slots' boot control block and what SPL decides from it (`nibos-bootctl`);
//! - [`board`]: the board's description, which names the devices and sinks
//!   the daemons work with;
//! - [`gamepad`]: how `nibos-input` turns the raw buttons and stick into one
//!   standard gamepad;
//! - [`keys`]: `nibos-keys`, the console's on-screen keyboard: the gamepad as its buttons,
//!   the Menu + X chord, and the repeat of a held key;
//! - [`controls`]: what `nibos-input` does with every raw event: the gamepad,
//!   the volume keys, and the Menu + volume brightness shortcut;
//! - [`backlight`]: the screens' backlights by name, in percent, with a floor
//!   and saved levels (`nibos-backlight`, `nibos-input`);
//! - [`power`]: the power key: a tap sleeps, a hold asks for the power menu;
//! - [`charge`]: charging asleep when the charger switched the device on (`nibos-charge-boot`);
//! - [`audio`]: how `nibos-audio` picks sinks and reads PipeWire's tools;
//! - [`provision`]: checking a downloadable image's `provision.toml`;
//! - [`steps`]: the volume keys' steps, for brightness and volume alike;
//! - [`update`]: system updates: a release's manifest and signature, and writing it to the
//!   slot that isn't running (`nibos-update`).
//!
//! The binaries (`src/bin/`) open the devices, run the loops and call these.

pub mod ab;
pub mod audio;
pub mod backlight;
pub mod board;
pub mod charge;
pub mod controls;
pub mod gamepad;
pub mod keys;
pub mod power;
pub mod provision;
pub mod steps;
pub mod update;

/// The device `nibos-input` passes the volume keys on through (the raw
/// device is grabbed, so the Menu + volume shortcut doesn't change the volume
/// too). `nibos-audio` and desktops read the volume keys from it.
pub const VOLUME_KEYS_NAME: &str = "nibOS volume keys";
