//! The merged gamepad's layout, following the kernel's gamepad spec
//! (Documentation/input/gamepad.rst):
//!
//! - face, shoulder, trigger, Select/Start and stick-click buttons pass
//!   through (the device tree already names them by position: BTN_SOUTH is
//!   the bottom face button);
//! - the Menu button (KEY_HOME) becomes BTN_MODE;
//! - the D-pad becomes the ABS_HAT0X/Y hat, -1 left/up, +1 right/down;
//! - each stick axis is re-centred on its resting value and scaled to
//!   ±[`AXIS_MAX`], each side of centre on its own, since the ADC is not
//!   symmetric.

use evdev::{AbsoluteAxisType, EventType, InputEvent, Key};

/// The stick range each side of centre.
pub const AXIS_MAX: i32 = 32767;
/// The dead zone reported to readers (`AbsInfo::flat`).
pub const AXIS_FLAT: i32 = 2048;

/// Buttons that keep their code on the merged gamepad.
pub const PASS_THROUGH: &[Key] = &[
    Key::BTN_SOUTH,
    Key::BTN_EAST,
    Key::BTN_NORTH,
    Key::BTN_WEST,
    Key::BTN_TL,
    Key::BTN_TR,
    Key::BTN_TL2,
    Key::BTN_TR2,
    Key::BTN_SELECT,
    Key::BTN_START,
    Key::BTN_THUMBL,
    Key::BTN_THUMBR,
];

/// Stick axes the merged gamepad offers, when the stick device has them (the
/// RG DS+ has one stick: ABS_X and ABS_Y).
pub const STICK_AXES: [AbsoluteAxisType; 4] = [
    AbsoluteAxisType::ABS_X,
    AbsoluteAxisType::ABS_Y,
    AbsoluteAxisType::ABS_RX,
    AbsoluteAxisType::ABS_RY,
];

/// One raw ADC axis: its range and its resting value, read at start-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub min: i32,
    pub centre: i32,
    pub max: i32,
}

impl Axis {
    /// The raw value as a gamepad value: 0 at rest, ±[`AXIS_MAX`] at either
    /// end of the raw range, linear on each side of the centre.
    pub fn scale(&self, raw: i32) -> i32 {
        let raw = raw.clamp(self.min, self.max);
        let span = if raw >= self.centre {
            self.max - self.centre
        } else {
            self.centre - self.min
        };
        if span <= 0 {
            return 0;
        }
        (i64::from(raw - self.centre) * i64::from(AXIS_MAX) / i64::from(span)) as i32
    }
}

/// Which raw device an event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The buttons and the D-pad (`NIBOS_BUTTONS_DEVICE`).
    Buttons,
    /// The stick (`NIBOS_STICK_DEVICE`).
    Sticks,
}

/// The D-pad buttons held, so that opposite directions held together cancel
/// out instead of the last one winning.
#[derive(Clone, Copy, Debug, Default)]
struct Dpad {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

/// -1, 0 or +1 for a pair of opposite directions.
fn hat(negative: bool, positive: bool) -> i32 {
    i32::from(positive) - i32::from(negative)
}

impl Dpad {
    /// Records a D-pad press or release and gives the hat axis and value it
    /// sets; `None` for keys that aren't D-pad directions.
    fn press(&mut self, key: Key, pressed: bool) -> Option<(AbsoluteAxisType, i32)> {
        match key {
            Key::BTN_DPAD_UP => self.up = pressed,
            Key::BTN_DPAD_DOWN => self.down = pressed,
            Key::BTN_DPAD_LEFT => self.left = pressed,
            Key::BTN_DPAD_RIGHT => self.right = pressed,
            _ => return None,
        }
        Some(match key {
            Key::BTN_DPAD_UP | Key::BTN_DPAD_DOWN => {
                (AbsoluteAxisType::ABS_HAT0Y, hat(self.up, self.down))
            }
            _ => (AbsoluteAxisType::ABS_HAT0X, hat(self.left, self.right)),
        })
    }
}

/// Turns the raw devices' events into the merged gamepad's.
#[derive(Debug)]
pub struct Mapper {
    axes: Vec<(AbsoluteAxisType, Axis)>,
    dpad: Dpad,
}

impl Mapper {
    /// A mapper for the stick axes the stick device has, with their ranges.
    pub fn new(axes: Vec<(AbsoluteAxisType, Axis)>) -> Self {
        Mapper {
            axes,
            dpad: Dpad::default(),
        }
    }

    /// The stick axes the merged gamepad offers.
    pub fn stick_axes(&self) -> Vec<AbsoluteAxisType> {
        self.axes.iter().map(|(a, _)| *a).collect()
    }

    /// The merged gamepad's event for one raw event, if it has one. SYN
    /// reports are left out: the caller emits a batch at a time, and uinput
    /// adds its own.
    pub fn map(&mut self, source: Source, ev: InputEvent) -> Option<InputEvent> {
        match (source, ev.event_type()) {
            (Source::Buttons, EventType::KEY) => {
                let key = Key::new(ev.code());
                if let Some((axis, value)) = self.dpad.press(key, ev.value() != 0) {
                    return Some(InputEvent::new(EventType::ABSOLUTE, axis.0, value));
                }
                match key {
                    Key::KEY_HOME => Some(InputEvent::new(
                        EventType::KEY,
                        Key::BTN_MODE.code(),
                        ev.value(),
                    )),
                    k if PASS_THROUGH.contains(&k) => Some(ev),
                    _ => None,
                }
            }
            (Source::Sticks, EventType::ABSOLUTE) => self
                .axes
                .iter()
                .find(|(a, _)| a.0 == ev.code())
                .map(|(a, axis)| InputEvent::new(EventType::ABSOLUTE, a.0, axis.scale(ev.value()))),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STICK: Axis = Axis {
        min: 0,
        centre: 1800,
        max: 4095,
    };

    fn key(k: Key, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY, k.code(), value)
    }

    fn parts(ev: Option<InputEvent>) -> Option<(EventType, u16, i32)> {
        ev.map(|e| (e.event_type(), e.code(), e.value()))
    }

    fn mapper() -> Mapper {
        Mapper::new(vec![
            (AbsoluteAxisType::ABS_X, STICK),
            (AbsoluteAxisType::ABS_Y, STICK),
        ])
    }

    #[test]
    fn axis_rest_and_ends() {
        assert_eq!(STICK.scale(1800), 0);
        assert_eq!(STICK.scale(4095), AXIS_MAX);
        assert_eq!(STICK.scale(0), -AXIS_MAX);
    }

    #[test]
    fn axis_scales_each_side_on_its_own() {
        // Half way to either end is half the range, though the spans differ
        // (1800 below the centre, 2295 above).
        assert_eq!(STICK.scale(900), -(AXIS_MAX / 2));
        assert_eq!(STICK.scale(1800 + 2295 / 2), AXIS_MAX * 1147 / 2295);
    }

    #[test]
    fn axis_clamps_outside_the_range() {
        assert_eq!(STICK.scale(-50), -AXIS_MAX);
        assert_eq!(STICK.scale(9999), AXIS_MAX);
    }

    #[test]
    fn axis_without_a_span_is_centred() {
        let stuck = Axis {
            min: 100,
            centre: 100,
            max: 100,
        };
        assert_eq!(stuck.scale(100), 0);
        let at_end = Axis {
            min: 0,
            centre: 4095,
            max: 4095,
        };
        assert_eq!(at_end.scale(4095), 0);
        assert_eq!(at_end.scale(0), -AXIS_MAX);
    }

    #[test]
    fn face_buttons_pass_through() {
        let mut m = mapper();
        for &k in PASS_THROUGH {
            assert_eq!(
                parts(m.map(Source::Buttons, key(k, 1))),
                Some((EventType::KEY, k.code(), 1))
            );
        }
    }

    #[test]
    fn menu_becomes_mode_with_its_value() {
        let mut m = mapper();
        for value in [1, 2, 0] {
            assert_eq!(
                parts(m.map(Source::Buttons, key(Key::KEY_HOME, value))),
                Some((EventType::KEY, Key::BTN_MODE.code(), value))
            );
        }
    }

    #[test]
    fn other_keys_are_dropped() {
        let mut m = mapper();
        assert_eq!(
            parts(m.map(Source::Buttons, key(Key::KEY_VOLUMEUP, 1))),
            None
        );
        assert_eq!(parts(m.map(Source::Buttons, key(Key::KEY_BACK, 1))), None);
    }

    #[test]
    fn dpad_is_a_hat() {
        let mut m = mapper();
        let y = AbsoluteAxisType::ABS_HAT0Y.0;
        let x = AbsoluteAxisType::ABS_HAT0X.0;
        let mut press = |k, v| parts(m.map(Source::Buttons, key(k, v)));
        assert_eq!(
            press(Key::BTN_DPAD_UP, 1),
            Some((EventType::ABSOLUTE, y, -1))
        );
        // Opposite directions together cancel; letting one go leaves the other.
        assert_eq!(
            press(Key::BTN_DPAD_DOWN, 1),
            Some((EventType::ABSOLUTE, y, 0))
        );
        assert_eq!(
            press(Key::BTN_DPAD_UP, 0),
            Some((EventType::ABSOLUTE, y, 1))
        );
        assert_eq!(
            press(Key::BTN_DPAD_DOWN, 0),
            Some((EventType::ABSOLUTE, y, 0))
        );
        assert_eq!(
            press(Key::BTN_DPAD_RIGHT, 1),
            Some((EventType::ABSOLUTE, x, 1))
        );
        assert_eq!(
            press(Key::BTN_DPAD_LEFT, 1),
            Some((EventType::ABSOLUTE, x, 0))
        );
        // A key repeat counts as held.
        assert_eq!(
            press(Key::BTN_DPAD_RIGHT, 2),
            Some((EventType::ABSOLUTE, x, 0))
        );
    }

    #[test]
    fn sticks_are_scaled_and_unknown_axes_dropped() {
        let mut m = mapper();
        let ev = |axis: AbsoluteAxisType, v| InputEvent::new(EventType::ABSOLUTE, axis.0, v);
        assert_eq!(
            parts(m.map(Source::Sticks, ev(AbsoluteAxisType::ABS_X, 4095))),
            Some((EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, AXIS_MAX))
        );
        assert_eq!(
            parts(m.map(Source::Sticks, ev(AbsoluteAxisType::ABS_RX, 4095))),
            None
        );
    }

    #[test]
    fn events_from_the_wrong_device_are_dropped() {
        let mut m = mapper();
        assert_eq!(parts(m.map(Source::Sticks, key(Key::BTN_SOUTH, 1))), None);
        let abs = InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, 10);
        assert_eq!(parts(m.map(Source::Buttons, abs)), None);
        let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0);
        assert_eq!(parts(m.map(Source::Buttons, syn)), None);
    }

    #[test]
    fn stick_axes_in_order() {
        assert_eq!(
            mapper().stick_axes(),
            vec![AbsoluteAxisType::ABS_X, AbsoluteAxisType::ABS_Y]
        );
    }
}
