//! nibos-audio: sound to the headphones while they're plugged in, to the
//! speakers otherwise. Runs as a user service next to PipeWire. What it does
//! depends on what the board's description names ([`nibos_hw::board`]):
//!
//! - Where the headphones and the speakers are two cards (the RG DS+),
//!   PipeWire switches ports within a card but not between cards, so this
//!   watches the jack switch (SW_HEADPHONE_INSERT on the jack device, "rk817_ext
//!   Headphones") and makes the matching sink the default with `pactl`. A
//!   connected Bluetooth sink comes first: it becomes the default when it
//!   appears (sinks coming and going, from `pactl subscribe`), and when it goes
//!   the jack decides again.
//! - With `NIBOS_SPEAKERS_KEEPALIVE`, it keeps a silent stream playing to the
//!   speakers. The RG DS+'s amps take a few hundred milliseconds to lock to
//!   the I2S clock whenever it starts, which cut off the start of every sound;
//!   PipeWire only runs the speaker output while something is connected to it,
//!   so something always is.
//! - Outside a desktop it turns the volume keys (on the volume device,
//!   "gpio-keys-volume", as nibos-input passes them on through "nibOS volume
//!   keys") into volume steps on the default sink with `wpctl`;
//!   under a Wayland compositor the desktop's own bindings do it.

use evdev::{EventType, Key, SwitchType};
use nibos_hw::{
    audio::{
        find_sink, is_wayland_socket, parse_volume, pick_sink, sink_event, sink_names,
        volume_after_step, volume_direction, Sinks,
    },
    board::Board,
    steps::Repeats,
    VOLUME_KEYS_NAME,
};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

/// The sinks PipeWire has now; none if pactl can't reach it.
fn sinks() -> Vec<String> {
    match Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
    {
        Ok(out) if out.status.success() => sink_names(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// Make the right sink the default ([`pick_sink`]): Bluetooth, else by the jack. Waits up to
/// a minute for PipeWire to have it; returns the sink it set.
fn route(sinks_of: Sinks, plugged: bool, last: Option<&str>) -> Option<String> {
    for _ in 0..60 {
        let now = sinks();
        if let Some(sink) = pick_sink(&now, sinks_of, plugged).map(str::to_owned) {
            if last == Some(sink.as_str()) {
                return Some(sink);
            }
            let ok = Command::new("pactl")
                .args(["set-default-sink", &sink])
                .status()
                .is_ok_and(|s| s.success());
            eprintln!("nibos-audio: default sink -> {sink}");
            if ok {
                return Some(sink);
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
    eprintln!("nibos-audio: no sink to route to");
    None
}

/// Sinks coming and going (`pactl subscribe`), restarted if PipeWire goes away.
fn watch_sinks(tx: mpsc::Sender<Event>) {
    thread::spawn(move || loop {
        if let Ok(mut child) = Command::new("pactl")
            .arg("subscribe")
            .stdout(Stdio::piped())
            .spawn()
        {
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if sink_event(&line) && tx.send(Event::Sinks).is_err() {
                        return;
                    }
                }
            }
            let _ = child.wait();
        }
        thread::sleep(Duration::from_secs(2));
    });
}

/// What makes the router look again.
enum Event {
    Jack(bool),
    Sinks,
}

/// Play silence to the speakers for as long as this runs, restarting the
/// player if PipeWire goes away and comes back.
fn keep_speakers_awake(speakers: String) {
    thread::spawn(move || loop {
        if let Some(sink) = find_sink(&sinks(), &speakers).map(str::to_owned) {
            let status = std::fs::File::open("/dev/zero").and_then(|zero| {
                Command::new("pw-cat")
                    .args([
                        "--playback",
                        "--raw",
                        "--format=s16",
                        "--rate=48000",
                        "--channels=2",
                    ])
                    .arg(format!("--target={sink}"))
                    .args(["--volume=0", "--media-category=Playback"])
                    .arg(
                        "--properties={ media.name = \"Keep speakers awake\" \
                         node.name = \"nibos-audio-keepalive\" }",
                    )
                    .arg("-")
                    .stdin(Stdio::from(zero))
                    .stdout(Stdio::null())
                    .status()
            });
            eprintln!("nibos-audio: keep-alive stream ended ({status:?})");
        }
        thread::sleep(Duration::from_secs(2));
    });
}

/// A Wayland compositor is running for this user: its socket is in
/// XDG_RUNTIME_DIR.
fn desktop_running() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return false;
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| is_wayland_socket(&e.file_name().to_string_lossy()))
}

/// The default sink's volume onto the next multiple of 5% up or down for a press, and for
/// every fifth repeat while held ([`nibos_hw::steps`]), at most 100%.
fn volume_keys(device: String) {
    thread::spawn(move || loop {
        let Some((_, mut keys)) = evdev::enumerate().find(|(_, d)| d.name() == Some(&device))
        else {
            thread::sleep(Duration::from_secs(5));
            continue;
        };
        let mut repeats = Repeats::default();
        while let Ok(events) = keys.fetch_events() {
            for ev in events.collect::<Vec<_>>() {
                if ev.event_type() != EventType::KEY || desktop_running() {
                    continue;
                }
                let Some(dir) = volume_direction(Key::new(ev.code())) else {
                    continue;
                };
                if !repeats.step(ev.value()) {
                    continue;
                }
                let now = Command::new("wpctl")
                    .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
                    .output();
                let Some(percent) = now
                    .ok()
                    .and_then(|o| parse_volume(&String::from_utf8_lossy(&o.stdout)))
                else {
                    eprintln!("nibos-audio: wpctl get-volume gave no volume");
                    continue;
                };
                let to = volume_after_step(percent, dir);
                let status = Command::new("wpctl")
                    .args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &to])
                    .status();
                if let Err(e) = status {
                    eprintln!("nibos-audio: wpctl: {e}");
                }
            }
        }
        eprintln!("nibos-audio: lost {device}");
        thread::sleep(Duration::from_secs(1));
    });
}

fn main() {
    let board = Board::load().unwrap_or_else(|e| {
        eprintln!("nibos-audio: the board's description: {e}");
        std::process::exit(1);
    });
    if board.speakers_keepalive {
        if let Some(speakers) = &board.speakers_sink {
            keep_speakers_awake(speakers.clone());
        }
    }
    // nibos-input grabs the board's volume device and passes the keys on
    // (except Menu + volume, the brightness shortcut).
    if board.volume_device.is_some() {
        volume_keys(VOLUME_KEYS_NAME.to_owned());
    }
    let (Some(jack_device), Some(headphones), Some(speakers)) = (
        board.jack_device.as_deref(),
        board.headphones_sink.as_deref(),
        board.speakers_sink.as_deref(),
    ) else {
        // No second card to route to: only the threads above have work.
        loop {
            thread::park();
        }
    };
    let sinks_of = Sinks {
        headphones,
        speakers,
    };

    let mut jack = loop {
        if let Some((_, dev)) = evdev::enumerate().find(|(_, d)| d.name() == Some(jack_device)) {
            break dev;
        }
        thread::sleep(Duration::from_secs(1));
    };
    let mut plugged = jack
        .get_switch_state()
        .map(|s| s.contains(SwitchType::SW_HEADPHONE_INSERT))
        .unwrap_or(false);
    let (tx, rx) = mpsc::channel();
    watch_sinks(tx.clone());
    let jack_device = jack_device.to_owned();
    thread::spawn(move || loop {
        let events: Vec<_> = match jack.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) => {
                eprintln!("nibos-audio: reading {jack_device}: {e}");
                std::process::exit(1);
            }
        };
        for ev in events {
            if ev.event_type() == EventType::SWITCH
                && ev.code() == SwitchType::SW_HEADPHONE_INSERT.0
                && tx.send(Event::Jack(ev.value() != 0)).is_err()
            {
                return;
            }
        }
    });

    let mut current = route(sinks_of, plugged, None);
    for ev in rx {
        match ev {
            Event::Jack(now) if now == plugged => continue,
            Event::Jack(now) => {
                plugged = now;
                eprintln!(
                    "nibos-audio: headphones {}",
                    if plugged { "in" } else { "out" }
                );
            }
            // A sink came or went: give PipeWire a moment to finish setting it up.
            Event::Sinks => thread::sleep(Duration::from_millis(500)),
        }
        current = route(sinks_of, plugged, current.as_deref());
    }
}
