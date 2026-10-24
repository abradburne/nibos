//! The board's description as the daemons read it: which input devices are
//! the controls, what the merged gamepad is called, and which PipeWire sinks
//! are the headphones and the speakers.
//!
//! It comes from [`PATH`], which the package installs from
//! `boards/<board>/hw/files/`. The file is `NAME=value` lines, as a POSIX
//! shell and systemd's `EnvironmentFile=` read them too: `#` starts a comment
//! line, and a value may be in single or double quotes. A name the board
//! doesn't need (a stick on a board without one) is left out or empty.

use std::collections::HashMap;

/// Where the package installs the board's description.
pub const PATH: &str = "/usr/lib/nibos/board.conf";
/// Where an administrator overrides single names in it (`NAME=value` lines,
/// the later winning), e.g. `NIBOS_POWER_LONG_PRESS_MS=1500`.
pub const LOCAL_PATH: &str = "/etc/nibos/board.conf";

/// What the daemons need to know about the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    /// `NIBOS_BOARD_NAME`: the board's name, for messages.
    pub name: String,
    /// `NIBOS_PAD_NAME`: the merged gamepad's name, which udev rules and SDL
    /// mappings match.
    pub pad_name: String,
    /// `NIBOS_BUTTONS_DEVICE`: the input device with the face, shoulder and
    /// D-pad buttons and Menu.
    pub buttons_device: String,
    /// `NIBOS_STICK_DEVICE`: the input device with the stick axes, if any.
    pub stick_device: Option<String>,
    /// `NIBOS_VOLUME_DEVICE`: the input device with the volume keys, if they
    /// are on one of their own.
    pub volume_device: Option<String>,
    /// `NIBOS_JACK_DEVICE`: the input device whose SW_HEADPHONE_INSERT switch
    /// is the headphone jack.
    pub jack_device: Option<String>,
    /// `NIBOS_HEADPHONES_SINK`: the start of the headphone sink's PipeWire
    /// node name, when the headphones are a card of their own.
    pub headphones_sink: Option<String>,
    /// `NIBOS_SPEAKERS_SINK`: the same for the speakers.
    pub speakers_sink: Option<String>,
    /// `NIBOS_SPEAKERS_KEEPALIVE=1`: keep a silent stream playing to the
    /// speakers, for amps that cut off the start of every sound while they
    /// lock to the I2S clock.
    pub speakers_keepalive: bool,
    /// `NIBOS_BACKLIGHTS`: each backlight's screen, as `screen:backlightN`
    /// pairs (`top:backlight1 bottom:backlight0`).
    pub backlights: Vec<(String, String)>,
    /// `NIBOS_BACKLIGHT_MIN_PERCENT`: the lowest level nibOS sets, as a share
    /// of the maximum in percent (2 if not given).
    pub backlight_min_percent: u32,
    /// `NIBOS_POWER_DEVICE`: the input device with the power key, which
    /// nibos-input handles (tap: sleep; hold: the power menu).
    pub power_device: Option<String>,
    /// `NIBOS_POWER_LONG_PRESS_MS`: how long the power key is held for the
    /// power menu (1000 if not given; 300-4000).
    pub power_long_press_ms: u32,
    /// `NIBOS_LID_DEVICE`: the input device whose SW_LID switch is the lid, if any.
    pub lid_device: Option<String>,
    /// `NIBOS_CHARGER`: the charger's power supply (`/sys/class/power_supply/<name>`).
    pub charger: Option<String>,
    /// `NIBOS_POWER_ON_SOURCE`: where the PMIC says what switched the device on
    /// ([`crate::charge::OnSource`]); without it, the device boots whatever switched it on.
    pub power_on_source: Option<crate::charge::OnSource>,
}

/// The `NAME=value` pairs of an environment file, in the subset described
/// above.
pub fn parse_env(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(name, value)| {
            let value = value.trim();
            let unquoted = [('"', '"'), ('\'', '\'')]
                .iter()
                .find_map(|&(open, close)| {
                    value.strip_prefix(open).and_then(|v| v.strip_suffix(close))
                })
                .unwrap_or(value);
            (name.trim().to_owned(), unquoted.to_owned())
        })
        .collect()
}

impl Board {
    /// The board described by an environment file's text.
    pub fn from_env(text: &str) -> Result<Board, String> {
        let env = parse_env(text);
        let optional = |name: &str| env.get(name).filter(|v| !v.is_empty()).cloned();
        let required = |name: &str| optional(name).ok_or(format!("{PATH} has no {name}"));
        Ok(Board {
            name: required("NIBOS_BOARD_NAME")?,
            pad_name: required("NIBOS_PAD_NAME")?,
            buttons_device: required("NIBOS_BUTTONS_DEVICE")?,
            stick_device: optional("NIBOS_STICK_DEVICE"),
            volume_device: optional("NIBOS_VOLUME_DEVICE"),
            jack_device: optional("NIBOS_JACK_DEVICE"),
            headphones_sink: optional("NIBOS_HEADPHONES_SINK"),
            speakers_sink: optional("NIBOS_SPEAKERS_SINK"),
            speakers_keepalive: optional("NIBOS_SPEAKERS_KEEPALIVE").as_deref() == Some("1"),
            backlights: optional("NIBOS_BACKLIGHTS")
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|pair| pair.split_once(':'))
                .map(|(screen, name)| (screen.to_owned(), name.to_owned()))
                .collect(),
            backlight_min_percent: match optional("NIBOS_BACKLIGHT_MIN_PERCENT") {
                Some(v) => v.parse().ok().filter(|&p| p < 100).ok_or(format!(
                    "{PATH}: NIBOS_BACKLIGHT_MIN_PERCENT={v} is not 0-99"
                ))?,
                None => 2,
            },
            power_device: optional("NIBOS_POWER_DEVICE"),
            power_long_press_ms: match optional("NIBOS_POWER_LONG_PRESS_MS") {
                Some(v) => v
                    .parse()
                    .ok()
                    .filter(|ms| (300..=4000).contains(ms))
                    .ok_or(format!(
                        "{PATH}: NIBOS_POWER_LONG_PRESS_MS={v} is not 300-4000"
                    ))?,
                None => 1000,
            },
            lid_device: optional("NIBOS_LID_DEVICE"),
            charger: optional("NIBOS_CHARGER"),
            power_on_source: optional("NIBOS_POWER_ON_SOURCE")
                .map(|v| crate::charge::OnSource::parse(&v).map_err(|e| format!("{PATH}: {e}")))
                .transpose()?,
        })
    }

    /// The installed board's description, with the administrator's changes
    /// from [`LOCAL_PATH`] over it, name by name.
    pub fn load() -> Result<Board, String> {
        let text = std::fs::read_to_string(PATH).map_err(|e| format!("{PATH}: {e}"))?;
        let local = std::fs::read_to_string(LOCAL_PATH).unwrap_or_default();
        Board::from_env(&format!("{text}\n{local}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The daemons' part of boards/rg-ds-plus/hw/files/usr/lib/nibos/board.conf.
    const RG_DS_PLUS: &str = r#"
# nibos-hw: the Anbernic RG DS+
NIBOS_BOARD=rg-ds-plus
NIBOS_BOARD_NAME="Anbernic RG DS+"
NIBOS_PAD_NAME="Anbernic RG DS+ Gamepad"
NIBOS_BUTTONS_DEVICE=gpio-keys-control
NIBOS_STICK_DEVICE=adc-joystick
NIBOS_VOLUME_DEVICE=gpio-keys-volume
NIBOS_JACK_DEVICE="rk817_ext Headphones"
NIBOS_HEADPHONES_SINK=alsa_output.platform-sound.
NIBOS_SPEAKERS_SINK=alsa_output.platform-sound-speakers.
NIBOS_SPEAKERS_KEEPALIVE=1
NIBOS_NETWORK_MODULES="rtw88_8821cs hci_uart"
NIBOS_BACKLIGHTS="top:backlight1 bottom:backlight0"
NIBOS_BACKLIGHT_MIN_PERCENT=2
NIBOS_POWER_DEVICE="rk805 pwrkey"
NIBOS_POWER_LONG_PRESS_MS=1000
NIBOS_LID_DEVICE=gpio-keys-hall
NIBOS_CHARGER=rk817-charger
NIBOS_POWER_ON_SOURCE="0 0x20 0xf5 0x40 0x80"
"#;

    #[test]
    fn env_lines_quotes_and_comments() {
        let env = parse_env(
            "# a comment\n\nA=plain\nB=\"double quoted\"\n  C = 'single' \nD=\nnot a pair\n",
        );
        assert_eq!(env["A"], "plain");
        assert_eq!(env["B"], "double quoted");
        assert_eq!(env["C"], "single");
        assert_eq!(env["D"], "");
        assert_eq!(env.len(), 4);
    }

    #[test]
    fn rg_ds_plus() {
        let board = Board::from_env(RG_DS_PLUS).unwrap();
        assert_eq!(board.name, "Anbernic RG DS+");
        assert_eq!(board.pad_name, "Anbernic RG DS+ Gamepad");
        assert_eq!(board.buttons_device, "gpio-keys-control");
        assert_eq!(board.stick_device.as_deref(), Some("adc-joystick"));
        assert_eq!(board.volume_device.as_deref(), Some("gpio-keys-volume"));
        assert_eq!(board.jack_device.as_deref(), Some("rk817_ext Headphones"));
        assert_eq!(
            board.headphones_sink.as_deref(),
            Some("alsa_output.platform-sound.")
        );
        assert_eq!(
            board.speakers_sink.as_deref(),
            Some("alsa_output.platform-sound-speakers.")
        );
        assert!(board.speakers_keepalive);
        assert_eq!(
            board.backlights,
            [
                ("top".to_owned(), "backlight1".to_owned()),
                ("bottom".to_owned(), "backlight0".to_owned())
            ]
        );
        assert_eq!(board.backlight_min_percent, 2);
        assert_eq!(board.power_device.as_deref(), Some("rk805 pwrkey"));
        assert_eq!(board.power_long_press_ms, 1000);
        assert_eq!(board.lid_device.as_deref(), Some("gpio-keys-hall"));
        assert_eq!(board.charger.as_deref(), Some("rk817-charger"));
        assert_eq!(board.power_on_source.map(|s| s.reg), Some(0xf5));
        // An administrator's line after the board's wins.
        let local = format!("{RG_DS_PLUS}\nNIBOS_POWER_LONG_PRESS_MS=1500\n");
        assert_eq!(Board::from_env(&local).unwrap().power_long_press_ms, 1500);
    }

    #[test]
    fn missing_and_empty_names() {
        let minimal = "NIBOS_BOARD_NAME=X\nNIBOS_PAD_NAME=X Gamepad\nNIBOS_BUTTONS_DEVICE=keys\nNIBOS_STICK_DEVICE=\n";
        let board = Board::from_env(minimal).unwrap();
        assert_eq!(board.stick_device, None);
        assert_eq!(board.speakers_sink, None);
        assert!(!board.speakers_keepalive);
        assert!(board.backlights.is_empty());
        assert_eq!(board.backlight_min_percent, 2);
        let bad = format!("{minimal}NIBOS_BACKLIGHT_MIN_PERCENT=x\n");
        assert!(Board::from_env(&bad).unwrap_err().contains("0-99"));
        let err = Board::from_env("NIBOS_BOARD_NAME=X\n").unwrap_err();
        assert!(err.contains("NIBOS_PAD_NAME"), "{err}");
    }
}
