//! Choosing sinks and reading PipeWire's command-line tools for
//! `nibos-audio`.
//!
//! On a board whose headphones and speakers are two sound cards (the RG DS+:
//! the RK817 codec drives the jack, two AW88166 amps on I2S3 the speakers),
//! their sinks are told apart by the start of their PipeWire node names, which
//! the board's description gives ([`crate::board`]).

use evdev::Key;

/// The sink names in `pactl list short sinks` output (the second of its
/// tab-separated columns).
pub fn sink_names(pactl_output: &str) -> Vec<String> {
    pactl_output
        .lines()
        .filter_map(|l| l.split('\t').nth(1).map(str::to_owned))
        .collect()
}

/// The headphone and speaker sinks' node name prefixes.
#[derive(Clone, Copy, Debug)]
pub struct Sinks<'a> {
    pub headphones: &'a str,
    pub speakers: &'a str,
}

impl<'a> Sinks<'a> {
    /// The prefix of the sink that should be the default: the headphones
    /// while they're plugged in, the speakers otherwise.
    pub fn prefix(&self, headphones_plugged: bool) -> &'a str {
        if headphones_plugged {
            self.headphones
        } else {
            self.speakers
        }
    }
}

/// The first sink whose name starts with `prefix`.
pub fn find_sink<'a>(sinks: &'a [String], prefix: &str) -> Option<&'a str> {
    sinks
        .iter()
        .find(|s| s.starts_with(prefix))
        .map(String::as_str)
}

/// Bluetooth audio sinks' node names start with this (BlueZ through WirePlumber).
pub const BLUETOOTH_SINK: &str = "bluez_output.";

/// The sink that should be the default: a connected Bluetooth sink first (someone put their
/// headphones on), then the headphones on the jack while plugged in, then the speakers.
pub fn pick_sink<'a>(sinks: &'a [String], of: Sinks, plugged: bool) -> Option<&'a str> {
    find_sink(sinks, BLUETOOTH_SINK).or_else(|| find_sink(sinks, of.prefix(plugged)))
}

/// Whether a `pactl subscribe` line says a sink came or went.
pub fn sink_event(line: &str) -> bool {
    (line.starts_with("Event 'new'") || line.starts_with("Event 'remove'"))
        && line.contains(" on sink #")
}

/// A volume key's direction: 1 up, -1 down.
pub fn volume_direction(key: Key) -> Option<i32> {
    match key {
        Key::KEY_VOLUMEUP => Some(1),
        Key::KEY_VOLUMEDOWN => Some(-1),
        _ => None,
    }
}

/// The volume in percent from `wpctl get-volume` (`Volume: 0.37`, maybe with ` [MUTED]`).
pub fn parse_volume(wpctl_output: &str) -> Option<u32> {
    let v: f64 = wpctl_output
        .trim()
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((v * 100.0).round().clamp(0.0, 100.0) as u32)
}

/// `wpctl set-volume`'s argument for a step from `percent` in `dir`: the next multiple of 5%,
/// at most 100% ([`crate::steps`]).
pub fn volume_after_step(percent: u32, dir: i32) -> String {
    format!("{}%", crate::steps::next(percent, dir))
}

/// Whether a file in XDG_RUNTIME_DIR is a Wayland compositor's socket (and
/// not its lock file): a desktop is running and handles the volume keys.
pub fn is_wayland_socket(file_name: &str) -> bool {
    file_name.starts_with("wayland-") && !file_name.ends_with(".lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_first_then_the_jack() {
        let have = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cards = have(&[
            "alsa_output.platform-sound.HiFi__Speaker__sink",
            "alsa_output.platform-sound-speakers.stereo-fallback",
        ]);
        assert_eq!(
            pick_sink(&cards, SINKS, false),
            Some("alsa_output.platform-sound-speakers.stereo-fallback")
        );
        assert_eq!(
            pick_sink(&cards, SINKS, true),
            Some("alsa_output.platform-sound.HiFi__Speaker__sink")
        );
        let mut bt = cards.clone();
        bt.push("bluez_output.AC_80_0A_12_34_56.1".into());
        assert_eq!(
            pick_sink(&bt, SINKS, false),
            Some("bluez_output.AC_80_0A_12_34_56.1")
        );
        assert_eq!(
            pick_sink(&bt, SINKS, true),
            Some("bluez_output.AC_80_0A_12_34_56.1"),
            "BT over the jack"
        );
    }

    #[test]
    fn sink_events_from_pactl_subscribe() {
        assert!(sink_event("Event 'new' on sink #57"));
        assert!(sink_event("Event 'remove' on sink #57"));
        assert!(!sink_event("Event 'change' on sink #57"), "a volume change");
        assert!(
            !sink_event("Event 'new' on sink-input #90"),
            "a stream, not a sink"
        );
        assert!(!sink_event("Event 'new' on source #58"));
    }

    // The RG DS+'s two cards.
    const SINKS: Sinks = Sinks {
        headphones: "alsa_output.platform-sound.",
        speakers: "alsa_output.platform-sound-speakers.",
    };

    const PACTL: &str = "\
55\talsa_output.platform-sound-speakers.stereo-fallback\tPipeWire\ts16le 2ch 48000Hz\tRUNNING
56\talsa_output.platform-sound.HiFi__Headphones__sink\tPipeWire\ts16le 2ch 48000Hz\tSUSPENDED
";

    #[test]
    fn sink_names_from_pactl() {
        assert_eq!(
            sink_names(PACTL),
            [
                "alsa_output.platform-sound-speakers.stereo-fallback",
                "alsa_output.platform-sound.HiFi__Headphones__sink",
            ]
        );
        assert!(sink_names("").is_empty());
        assert!(sink_names("garbage without tabs\n").is_empty());
    }

    #[test]
    fn prefixes_tell_the_cards_apart() {
        // "platform-sound." must not match "platform-sound-speakers.".
        let sinks = sink_names(PACTL);
        assert_eq!(
            find_sink(&sinks, SINKS.prefix(true)),
            Some("alsa_output.platform-sound.HiFi__Headphones__sink")
        );
        assert_eq!(
            find_sink(&sinks, SINKS.prefix(false)),
            Some("alsa_output.platform-sound-speakers.stereo-fallback")
        );
        assert_eq!(find_sink(&sinks[..1], SINKS.headphones), None);
    }

    #[test]
    fn volume_keys_step_by_five_percent() {
        assert_eq!(volume_direction(Key::KEY_VOLUMEUP), Some(1));
        assert_eq!(volume_direction(Key::KEY_VOLUMEDOWN), Some(-1));
        assert_eq!(parse_volume("Volume: 0.37\n"), Some(37));
        assert_eq!(parse_volume("Volume: 0.40 [MUTED]\n"), Some(40));
        assert_eq!(parse_volume("Volume: 1.20"), Some(100));
        assert_eq!(parse_volume("nothing"), None);
        assert_eq!(volume_after_step(37, 1), "40%");
        assert_eq!(volume_after_step(37, -1), "35%");
        assert_eq!(volume_after_step(100, 1), "100%");
        assert_eq!(volume_direction(Key::KEY_MUTE), None);
    }

    #[test]
    fn wayland_sockets_but_not_their_locks() {
        assert!(is_wayland_socket("wayland-1"));
        assert!(!is_wayland_socket("wayland-1.lock"));
        assert!(!is_wayland_socket("pipewire-0"));
        assert!(!is_wayland_socket("bus"));
    }
}
