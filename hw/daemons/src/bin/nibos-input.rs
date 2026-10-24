//! nibos-input: one standard gamepad from the board's controls.
//!
//! The kernel exposes the controls as separate devices, named in the board's
//! description ([`nibos_hw::board`]): on the RG DS+ `gpio-keys-control` (face,
//! shoulder and D-pad buttons, Menu) and `adc-joystick` (the stick, as raw ADC
//! values). Games and SDL want a single gamepad, laid out as the kernel's
//! gamepad spec describes. This grabs both devices and re-emits them as one
//! uinput device, named by the board ("Anbernic RG DS+ Gamepad"); the layout
//! is [`nibos_hw::gamepad`].
//!
//! It also grabs the volume keys and passes them on through a device of its
//! own, "nibOS volume keys", except when Menu is held: Menu + volume up/down
//! changes both screens' brightness ([`nibos_hw::controls`]). Back, power,
//! the lid and touch stay on their own devices for the desktop or shell. The
//! board's udev rules (70-nibos-board.rules) hide the raw devices from games
//! and libinput.

use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AbsInfo, AbsoluteAxisType, AttributeSet, BusType, Device, InputEvent, InputId, Key,
    UinputAbsSetup,
};
use nibos_hw::{
    backlight::{Backlights, Target},
    board::Board,
    controls::{Action, Controls, Source},
    gamepad::{Axis, Mapper, AXIS_FLAT, AXIS_MAX, PASS_THROUGH, STICK_AXES},
    power::{time_asleep, PowerKey, Press, Wake, RESUMED_MARK, RESUME_HOLDOFF},
    VOLUME_KEYS_NAME,
};
use std::{
    process::{exit, Command},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant, SystemTime},
};

// pid.codes test range; no real vendor is implied.
const PAD_VENDOR: u16 = 0x1209;
const PAD_PRODUCT: u16 = 0x0001;

fn fail(what: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("nibos-input: {what}: {e}");
    exit(1);
}

/// The input device with this name, waiting up to 30 s for it to appear (the
/// devices can come up a moment after the service starts).
fn open(name: &str) -> Device {
    for _ in 0..60 {
        if let Some((_, dev)) = evdev::enumerate().find(|(_, d)| d.name() == Some(name)) {
            return dev;
        }
        thread::sleep(Duration::from_millis(500));
    }
    fail("no input device named", name);
}

fn build_pad(name: &str, sticks: &[AbsoluteAxisType]) -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    for key in PASS_THROUGH {
        keys.insert(*key);
    }
    keys.insert(Key::BTN_MODE);

    let mut builder = VirtualDeviceBuilder::new()?
        .name(name)
        .input_id(InputId::new(
            BusType::BUS_VIRTUAL,
            PAD_VENDOR,
            PAD_PRODUCT,
            0x0100,
        ))
        .with_keys(&keys)?;
    for &axis in sticks {
        let info = AbsInfo::new(0, -AXIS_MAX - 1, AXIS_MAX, 16, AXIS_FLAT, 0);
        builder = builder.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
    }
    for hat in [AbsoluteAxisType::ABS_HAT0X, AbsoluteAxisType::ABS_HAT0Y] {
        let info = AbsInfo::new(0, -1, 1, 0, 0, 0);
        builder = builder.with_absolute_axis(&UinputAbsSetup::new(hat, info))?;
    }
    builder.build()
}

/// The stick axes the device has, with their range and resting value. Read
/// before anyone touches the stick.
fn stick_axes(sticks: &Device) -> Vec<(AbsoluteAxisType, Axis)> {
    let state = sticks
        .get_abs_state()
        .unwrap_or_else(|e| fail("reading stick state", e));
    let present: Vec<_> = sticks
        .supported_absolute_axes()
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    STICK_AXES
        .iter()
        .filter(|a| present.contains(a))
        .map(|&a| {
            let s = state[a.0 as usize];
            let axis = Axis {
                min: s.minimum,
                centre: s.value,
                max: s.maximum,
            };
            eprintln!(
                "nibos-input: {a:?} {}..{}..{}",
                axis.min, axis.centre, axis.max
            );
            (a, axis)
        })
        .collect()
}

/// The volume keys device: the raw volume device's keys, passed on unless they
/// are the brightness shortcut.
fn build_volume_keys(raw: &Device) -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    for key in raw.supported_keys().into_iter().flat_map(|k| k.iter()) {
        keys.insert(key);
    }
    VirtualDeviceBuilder::new()?
        .name(VOLUME_KEYS_NAME)
        .input_id(InputId::new(
            BusType::BUS_VIRTUAL,
            PAD_VENDOR,
            PAD_PRODUCT + 1,
            0x0100,
        ))
        .with_keys(&keys)?
        .build()
}

/// Run a command without waiting for it (reaped by a thread of its own).
fn spawn(args: &[&str]) {
    match Command::new(args[0]).args(&args[1..]).spawn() {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("nibos-input: {}: {e}", args[0]),
    }
}

/// Whether the sleep hook says the device woke just now (the press that woke it comes next).
fn hook_says_woke() -> bool {
    std::fs::metadata(RESUMED_MARK)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < RESUME_HOLDOFF)
}

/// A tap of power sleeps; a hold asks the boot UI for its power menu, or, with no boot UI (the
/// base image's console, a program started from SSH), powers off cleanly: otherwise the only
/// way off was the PMIC's cut-off after a long hold, which loses what isn't written yet and
/// the power-off's log on the card.
fn power(press: Press) {
    match press {
        Press::Tap => spawn(&["systemctl", "suspend"]),
        Press::Hold => {
            let dslot = Command::new("systemctl")
                .args(["-q", "is-active", "dslot.service"])
                .status()
                .is_ok_and(|s| s.success());
            if dslot {
                spawn(&[
                    "systemctl",
                    "kill",
                    "--kill-whom=main",
                    "--signal=SIGUSR1",
                    "dslot.service",
                ]);
            } else {
                eprintln!("nibos-input: power held, and no boot UI for a power menu: powering off");
                spawn(&["systemctl", "poweroff"]);
            }
        }
    }
}

fn main() {
    let board = Board::load().unwrap_or_else(|e| fail("the board's description", e));
    let mut devices = vec![(open(&board.buttons_device), Source::Buttons)];
    if let Some(name) = &board.stick_device {
        devices.push((open(name), Source::Sticks));
    }
    if let Some(name) = &board.volume_device {
        devices.push((open(name), Source::Volume));
    }
    // The power key is read, not grabbed: logind ignores it (the board's logind.conf.d), and a
    // desktop may want to see it.
    let power_device = board.power_device.as_deref().map(open);
    let mut power_key = PowerKey::new(Duration::from_millis(board.power_long_press_ms.into()));
    let mut wake = Wake::default();
    wake.observe(time_asleep(), Instant::now());

    let find = |source| devices.iter().find(|(_, s)| *s == source).map(|(d, _)| d);
    let axes = find(Source::Sticks).map(stick_axes).unwrap_or_default();
    let mut volume_keys = find(Source::Volume).map(|raw| {
        build_volume_keys(raw).unwrap_or_else(|e| fail("creating the volume keys device", e))
    });
    let mut controls = Controls::new(Mapper::new(axes));
    let mut pad = build_pad(&board.pad_name, &controls.mapper().stick_axes())
        .unwrap_or_else(|e| fail("creating the virtual gamepad", e));
    let backlights = Backlights::for_board(&board);
    // Only the merged gamepad and the volume keys device should be visible to
    // programs.
    for (dev, _) in &mut devices {
        if let Err(e) = dev.grab() {
            eprintln!("nibos-input: grabbing {}: {e}", dev.name().unwrap_or("?"));
        }
    }

    // One reader thread per device; the main thread keeps the state and
    // writes the virtual devices.
    let (tx, rx) = mpsc::channel::<(Source, Vec<InputEvent>)>();
    let readers = devices
        .into_iter()
        .chain(power_device.map(|dev| (dev, Source::Power)));
    for (mut dev, source) in readers {
        let tx = tx.clone();
        thread::spawn(move || loop {
            match dev.fetch_events() {
                Ok(events) => {
                    if tx.send((source, events.collect())).is_err() {
                        return;
                    }
                }
                Err(e) => fail("reading", e),
            }
        });
    }
    drop(tx);

    loop {
        // Wake for the power key's long-press time while it is down.
        let received = match power_key.deadline() {
            Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        // Whatever woke us, see first whether the system slept: the waking press must not be a
        // tap that sleeps it again, nor a press left down from before.
        if wake.observe(time_asleep(), Instant::now()) {
            power_key.reset();
            eprintln!(
                "nibos-input: woke from sleep: the power key is ignored for {RESUME_HOLDOFF:?}"
            );
        }
        let (source, events) = match received {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(press) = power_key.tick(Instant::now()) {
                    power(press);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if source == Source::Power {
            for ev in events {
                if ev.event_type() == evdev::EventType::KEY
                    && ev.code() == Key::KEY_POWER.code()
                    && !wake.just_woke(Instant::now())
                    && !hook_says_woke()
                {
                    if let Some(press) = power_key.event(ev.value(), Instant::now()) {
                        power(press);
                    }
                }
            }
            continue;
        }
        for ev in events {
            for action in controls.handle(source, ev) {
                let written = match action {
                    // Each its own report: a held-back Menu's press and release
                    // must reach readers as two.
                    Action::Pad(out) => pad.emit(&out),
                    Action::Volume(ev) => match &mut volume_keys {
                        Some(keys) => keys.emit(&[ev]),
                        None => Ok(()),
                    },
                    Action::Brightness(dir) => {
                        if let Err(e) = backlights.step(&Target::All, dir) {
                            eprintln!("nibos-input: brightness: {e}");
                        }
                        Ok(())
                    }
                };
                if let Err(e) = written {
                    fail("writing", e);
                }
            }
        }
    }
}
