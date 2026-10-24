//! What `nibos-input` does with every raw event: the merged gamepad
//! ([`crate::gamepad`]), the volume keys passed on, and one system shortcut,
//! **Menu + Volume up / down**, which changes both screens' brightness
//! wherever you are (the shelf, a game, the console).
//!
//! Menu is also the games' own key (the emulator's menu and hotkey
//! modifier), so it must reach them unchanged when it isn't the shortcut,
//! and not at all when it is:
//!
//! - Menu's press is held back. Released with nothing else pressed, it goes
//!   out as a press and a release together: a tap, delayed by the time it was
//!   held (programs that act on Menu's release see no difference).
//! - Another gamepad button or the D-pad while Menu is held sends the held
//!   press first, then the button: Menu + Start and the like reach the game as
//!   before. Moving the stick doesn't.
//! - A volume key while Menu is held is the shortcut: Menu, its release
//!   included, never reaches the gamepad, and the volume key never reaches
//!   the volume keys device, so neither the game nor the volume changes.
//!
//! No other button is delayed.

use crate::gamepad::{Mapper, Source as PadSource};
use crate::steps::Repeats;
use evdev::{EventType, InputEvent, Key};

/// Where a raw event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Buttons,
    Sticks,
    Volume,
    /// The power key: handled apart ([`crate::power`]), never part of the gamepad.
    Power,
}

/// What to do for a raw event, in order.
#[derive(Clone, Debug)]
pub enum Action {
    /// Write these events to the gamepad, then a SYN report.
    Pad(Vec<InputEvent>),
    /// Write this event to the volume keys device, then a SYN report.
    Volume(InputEvent),
    /// Step both screens' brightness up (1) or down (-1), onto the next multiple of 5%
    /// ([`crate::steps`]).
    Brightness(i32),
}

// evdev's InputEvent has no equality; events are equal when they say the same.
impl PartialEq for Action {
    fn eq(&self, other: &Action) -> bool {
        let same = |a: &InputEvent, b: &InputEvent| {
            (a.event_type(), a.code(), a.value()) == (b.event_type(), b.code(), b.value())
        };
        match (self, other) {
            (Action::Pad(a), Action::Pad(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
            }
            (Action::Volume(a), Action::Volume(b)) => same(a, b),
            (Action::Brightness(a), Action::Brightness(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Up,
    /// Pressed, not sent yet.
    HeldBack,
    /// Pressed and sent, because another button came while it was held.
    Sent,
    /// Pressed and used for the shortcut: never sent.
    Shortcut,
}

/// The state between raw events.
#[derive(Debug)]
pub struct Controls {
    mapper: Mapper,
    menu: Menu,
    /// Volume keys whose press went to the volume keys device, so their
    /// release goes there too, whatever Menu does in between.
    volume_down: Vec<u16>,
    /// Which of the shortcut's key repeats are steps.
    repeats: Repeats,
}

fn mode(value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY, Key::BTN_MODE.code(), value)
}

impl Controls {
    pub fn new(mapper: Mapper) -> Controls {
        Controls {
            mapper,
            menu: Menu::Up,
            volume_down: Vec::new(),
            repeats: Repeats::default(),
        }
    }

    pub fn mapper(&self) -> &Mapper {
        &self.mapper
    }

    /// The actions for one raw event (SYN reports left out).
    pub fn handle(&mut self, source: Source, ev: InputEvent) -> Vec<Action> {
        match source {
            Source::Sticks => self.pad(PadSource::Sticks, ev),
            Source::Buttons
                if ev.event_type() == EventType::KEY && ev.code() == Key::KEY_HOME.code() =>
            {
                self.menu_key(ev.value())
            }
            Source::Buttons => {
                let mut out = Vec::new();
                if ev.event_type() == EventType::KEY
                    && ev.value() == 1
                    && self.menu == Menu::HeldBack
                {
                    out.push(Action::Pad(vec![mode(1)]));
                    self.menu = Menu::Sent;
                }
                out.extend(self.pad(PadSource::Buttons, ev));
                out
            }
            Source::Volume => self.volume_key(ev),
            Source::Power => vec![],
        }
    }

    fn pad(&mut self, source: PadSource, ev: InputEvent) -> Vec<Action> {
        self.mapper
            .map(source, ev)
            .map(|e| vec![Action::Pad(vec![e])])
            .unwrap_or_default()
    }

    fn menu_key(&mut self, value: i32) -> Vec<Action> {
        match (value, self.menu) {
            (1, _) => {
                self.menu = Menu::HeldBack;
                vec![]
            }
            (0, Menu::HeldBack) => {
                self.menu = Menu::Up;
                // Two reports, so a reader sees the press before the release.
                vec![Action::Pad(vec![mode(1)]), Action::Pad(vec![mode(0)])]
            }
            (0, Menu::Sent) => {
                self.menu = Menu::Up;
                vec![Action::Pad(vec![mode(0)])]
            }
            (0, _) => {
                self.menu = Menu::Up;
                vec![]
            }
            // Repeats: the button device doesn't send them, and a held-back
            // Menu has nothing to repeat.
            _ => vec![],
        }
    }

    fn volume_key(&mut self, ev: InputEvent) -> Vec<Action> {
        if ev.event_type() != EventType::KEY {
            return vec![];
        }
        let code = ev.code();
        let direction = match Key::new(code) {
            Key::KEY_VOLUMEUP => 1,
            Key::KEY_VOLUMEDOWN => -1,
            _ => return vec![Action::Volume(ev)],
        };
        let passed = self.volume_down.contains(&code);
        if self.menu == Menu::Up || passed {
            // Not the shortcut, or a key that started before Menu: the volume.
            match ev.value() {
                1 => self.volume_down.push(code),
                0 => self.volume_down.retain(|&c| c != code),
                _ => {}
            }
            return vec![Action::Volume(ev)];
        }
        if self.menu == Menu::HeldBack {
            self.menu = Menu::Shortcut;
        }
        if self.repeats.step(ev.value()) {
            vec![Action::Brightness(direction)]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::AbsoluteAxisType;

    fn key(k: Key, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY, k.code(), value)
    }

    fn controls() -> Controls {
        Controls::new(Mapper::new(vec![(
            AbsoluteAxisType::ABS_X,
            crate::gamepad::Axis {
                min: 0,
                centre: 512,
                max: 1023,
            },
        )]))
    }

    fn pad(events: &[InputEvent]) -> Action {
        Action::Pad(events.to_vec())
    }

    #[test]
    fn other_buttons_are_not_delayed() {
        let mut c = controls();
        assert_eq!(
            c.handle(Source::Buttons, key(Key::BTN_SOUTH, 1)),
            [pad(&[key(Key::BTN_SOUTH, 1)])]
        );
        assert_eq!(
            c.handle(Source::Buttons, key(Key::BTN_SOUTH, 0)),
            [pad(&[key(Key::BTN_SOUTH, 0)])]
        );
    }

    #[test]
    fn a_menu_tap_goes_out_on_release() {
        let mut c = controls();
        assert_eq!(c.handle(Source::Buttons, key(Key::KEY_HOME, 1)), []);
        assert_eq!(
            c.handle(Source::Buttons, key(Key::KEY_HOME, 0)),
            [pad(&[mode(1)]), pad(&[mode(0)])]
        );
    }

    #[test]
    fn menu_with_a_button_is_sent_before_it() {
        let mut c = controls();
        c.handle(Source::Buttons, key(Key::KEY_HOME, 1));
        assert_eq!(
            c.handle(Source::Buttons, key(Key::BTN_START, 1)),
            [pad(&[mode(1)]), pad(&[key(Key::BTN_START, 1)])]
        );
        assert_eq!(
            c.handle(Source::Buttons, key(Key::BTN_START, 0)),
            [pad(&[key(Key::BTN_START, 0)])]
        );
        assert_eq!(
            c.handle(Source::Buttons, key(Key::KEY_HOME, 0)),
            [pad(&[mode(0)])]
        );
    }

    #[test]
    fn the_d_pad_counts_as_a_button_and_the_stick_does_not() {
        let mut c = controls();
        c.handle(Source::Buttons, key(Key::KEY_HOME, 1));
        let stick = InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, 1023);
        assert_eq!(
            c.handle(Source::Sticks, stick).len(),
            1,
            "stick passes, Menu still held back"
        );
        let out = c.handle(Source::Buttons, key(Key::BTN_DPAD_RIGHT, 1));
        assert_eq!(out[0], pad(&[mode(1)]));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn menu_with_volume_is_the_shortcut_and_nothing_else() {
        let mut c = controls();
        c.handle(Source::Buttons, key(Key::KEY_HOME, 1));
        assert_eq!(
            c.handle(Source::Volume, key(Key::KEY_VOLUMEUP, 1)),
            [Action::Brightness(1)]
        );
        // Held: every fifth repeat is a step.
        let repeats: Vec<_> = (0..5)
            .map(|_| c.handle(Source::Volume, key(Key::KEY_VOLUMEUP, 2)))
            .collect();
        assert_eq!(repeats[..4], [vec![], vec![], vec![], vec![]]);
        assert_eq!(repeats[4], [Action::Brightness(1)]);
        assert_eq!(c.handle(Source::Volume, key(Key::KEY_VOLUMEUP, 0)), []);
        assert_eq!(
            c.handle(Source::Volume, key(Key::KEY_VOLUMEDOWN, 1)),
            [Action::Brightness(-1)]
        );
        assert_eq!(c.handle(Source::Volume, key(Key::KEY_VOLUMEDOWN, 0)), []);
        assert_eq!(
            c.handle(Source::Buttons, key(Key::KEY_HOME, 0)),
            [],
            "Menu swallowed, release too"
        );
        // Afterwards the volume keys are the volume again.
        assert_eq!(
            c.handle(Source::Volume, key(Key::KEY_VOLUMEUP, 1)),
            [Action::Volume(key(Key::KEY_VOLUMEUP, 1))]
        );
    }

    #[test]
    fn a_volume_key_held_before_menu_stays_the_volume() {
        let mut c = controls();
        let up = |v| key(Key::KEY_VOLUMEUP, v);
        assert_eq!(c.handle(Source::Volume, up(1)), [Action::Volume(up(1))]);
        c.handle(Source::Buttons, key(Key::KEY_HOME, 1));
        assert_eq!(c.handle(Source::Volume, up(2)), [Action::Volume(up(2))]);
        assert_eq!(c.handle(Source::Volume, up(0)), [Action::Volume(up(0))]);
        // Menu was never used: still a tap.
        assert_eq!(c.handle(Source::Buttons, key(Key::KEY_HOME, 0)).len(), 2);
    }

    #[test]
    fn volume_after_menu_was_sent_still_changes_brightness() {
        let mut c = controls();
        c.handle(Source::Buttons, key(Key::KEY_HOME, 1));
        c.handle(Source::Buttons, key(Key::BTN_TR, 1));
        assert_eq!(
            c.handle(Source::Volume, key(Key::KEY_VOLUMEDOWN, 1)),
            [Action::Brightness(-1)]
        );
        assert_eq!(
            c.handle(Source::Buttons, key(Key::KEY_HOME, 0)),
            [pad(&[mode(0)])],
            "sent, so released"
        );
    }
}
