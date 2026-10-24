//! nibos-keys' logic, apart from the screen and the devices: the merged gamepad's events as the
//! keyboard's buttons, the Menu + X chord that shows and hides it, and the repeat of a held key.
//!
//! The gamepad is nibos-input's merged one (gamepad.rs): face buttons by position (BTN_EAST is
//! the one printed A, BTN_SOUTH B, BTN_NORTH X, BTN_WEST Y), Menu as BTN_MODE, the D-pad as the
//! ABS_HAT0X/Y hat. nibos-input holds a Menu press back until it is released or another button
//! comes, so Menu + X arrives as Menu, then X.

use nibkit::menu::keys::{code, Button};
use std::time::{Duration, Instant};

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_MODE: u16 = 0x13c;

/// What a gamepad event comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    /// A keyboard button down or up.
    Button(Button, bool),
    /// Menu + X: show or hide the keyboard.
    Toggle,
}

/// The gamepad as the keyboard reads it.
#[derive(Debug, Default)]
pub struct Gamepad {
    hat: (i32, i32),
    menu: bool,
    /// X went down as part of the chord: its release isn't Tab's.
    chord_x: bool,
}

impl Gamepad {
    pub fn new() -> Gamepad {
        Gamepad::default()
    }

    /// One evdev event (type, code, value).
    pub fn event(&mut self, kind: u16, code: u16, value: i32) -> Vec<Pad> {
        match (kind, code) {
            (EV_ABS, ABS_HAT0X) => self.hat(value, true),
            (EV_ABS, ABS_HAT0Y) => self.hat(value, false),
            (EV_KEY, BTN_MODE) => {
                self.menu = value != 0;
                Vec::new()
            }
            (EV_KEY, BTN_NORTH) if value == 1 && self.menu => {
                self.chord_x = true;
                vec![Pad::Toggle]
            }
            (EV_KEY, BTN_NORTH) if value == 0 && self.chord_x => {
                self.chord_x = false;
                Vec::new()
            }
            (EV_KEY, code) => {
                let button = match code {
                    BTN_EAST => Button::A,
                    BTN_SOUTH => Button::B,
                    BTN_NORTH => Button::X,
                    BTN_WEST => Button::Y,
                    BTN_TL => Button::L,
                    BTN_TR => Button::R,
                    _ => return Vec::new(),
                };
                // 2 is the kernel's own repeat of a held button: the keyboard repeats keys itself.
                match value {
                    0 | 1 => vec![Pad::Button(button, value == 1)],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// The hat moves on one axis: the way it left comes up, the way it went goes down.
    fn hat(&mut self, value: i32, across: bool) -> Vec<Pad> {
        let (was, ways) = if across {
            (&mut self.hat.0, [Button::Left, Button::Right])
        } else {
            (&mut self.hat.1, [Button::Up, Button::Down])
        };
        let way = |v: i32| match v {
            v if v < 0 => Some(ways[0]),
            v if v > 0 => Some(ways[1]),
            _ => None,
        };
        let mut out = Vec::new();
        if *was != value {
            out.extend(way(*was).map(|b| Pad::Button(b, false)));
            out.extend(way(value).map(|b| Pad::Button(b, true)));
            *was = value;
        }
        out
    }
}

/// How long a key is held before it repeats, and how often it then does: the console's own
/// defaults for a keyboard (`kbdrate`: 250 ms, 30 a second), a little slower to start, for a
/// finger on glass.
pub const REPEAT_AFTER: Duration = Duration::from_millis(400);
pub const REPEAT_EVERY: Duration = Duration::from_millis(40);

/// The repeat of the last key pressed while it is held (not a modifier's).
#[derive(Debug, Default)]
pub struct Repeat {
    held: Option<(u16, Instant)>,
}

impl Repeat {
    /// A key went down.
    pub fn press(&mut self, key: u16, now: Instant) {
        if ![code::LEFTSHIFT, code::LEFTCTRL, code::LEFTALT].contains(&key) {
            self.held = Some((key, now + REPEAT_AFTER));
        }
    }

    /// A key came up.
    pub fn release(&mut self, key: u16) {
        if self.held.is_some_and(|(k, _)| k == key) {
            self.held = None;
        }
    }

    /// The key to repeat now, if one is due.
    pub fn due(&mut self, now: Instant) -> Option<u16> {
        let (key, next) = self.held.as_mut()?;
        if now < *next {
            return None;
        }
        *next += REPEAT_EVERY;
        Some(*key)
    }

    /// How long until the next repeat: `None` when nothing is held.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.held
            .map(|(_, next)| next.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_and_the_hat() {
        let mut pad = Gamepad::new();
        assert_eq!(
            pad.event(EV_KEY, BTN_EAST, 1),
            [Pad::Button(Button::A, true)]
        );
        assert!(pad.event(EV_KEY, BTN_EAST, 2).is_empty());
        assert_eq!(
            pad.event(EV_KEY, BTN_EAST, 0),
            [Pad::Button(Button::A, false)]
        );
        assert_eq!(
            pad.event(EV_ABS, ABS_HAT0Y, -1),
            [Pad::Button(Button::Up, true)]
        );
        // Straight from up to down: up comes up first.
        assert_eq!(
            pad.event(EV_ABS, ABS_HAT0Y, 1),
            [
                Pad::Button(Button::Up, false),
                Pad::Button(Button::Down, true)
            ]
        );
        assert_eq!(
            pad.event(EV_ABS, ABS_HAT0Y, 0),
            [Pad::Button(Button::Down, false)]
        );
        assert!(pad.event(EV_KEY, BTN_MODE, 1).is_empty());
    }

    /// Menu + X toggles and types no Tab; X alone is Tab; Menu released first is no chord.
    #[test]
    fn chord() {
        let mut pad = Gamepad::new();
        pad.event(EV_KEY, BTN_MODE, 1);
        assert_eq!(pad.event(EV_KEY, BTN_NORTH, 1), [Pad::Toggle]);
        pad.event(EV_KEY, BTN_MODE, 0);
        assert!(pad.event(EV_KEY, BTN_NORTH, 0).is_empty());
        assert_eq!(
            pad.event(EV_KEY, BTN_NORTH, 1),
            [Pad::Button(Button::X, true)]
        );
        assert_eq!(
            pad.event(EV_KEY, BTN_NORTH, 0),
            [Pad::Button(Button::X, false)]
        );
        // Menu tapped on its own, then X: Tab.
        pad.event(EV_KEY, BTN_MODE, 1);
        pad.event(EV_KEY, BTN_MODE, 0);
        assert_eq!(
            pad.event(EV_KEY, BTN_NORTH, 1),
            [Pad::Button(Button::X, true)]
        );
    }

    #[test]
    fn repeat() {
        let t0 = Instant::now();
        let mut r = Repeat::default();
        r.press(code::LEFTSHIFT, t0);
        assert_eq!(r.wait(t0), None);
        r.press(code::BACKSPACE, t0);
        assert_eq!(r.due(t0 + Duration::from_millis(399)), None);
        assert_eq!(r.due(t0 + REPEAT_AFTER), Some(code::BACKSPACE));
        assert_eq!(r.due(t0 + REPEAT_AFTER), None);
        assert_eq!(
            r.due(t0 + REPEAT_AFTER + REPEAT_EVERY),
            Some(code::BACKSPACE)
        );
        r.release(code::ENTER);
        assert!(r.wait(t0).is_some());
        r.release(code::BACKSPACE);
        assert_eq!(r.wait(t0), None);
    }
}
