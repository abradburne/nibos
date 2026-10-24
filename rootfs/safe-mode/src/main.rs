//! nibos-safe-mode: what the device shows when the boot UI keeps crashing.
//!
//! systemd restarts DSlot when it fails; when it has failed too often in a short time
//! (`dslot.service.d/nibos-recovery.conf`, nibos-hw), it starts `nibos-safe-mode.service` in
//! place of the console's login prompt. This puts up, through KMS (no GL, no SDL: either may be
//! what broke), what failed on the top screen and a menu on the bottom one, drawn as DSlot's
//! own (nibkit's menu, in Roboto Condensed), answered with the gamepad's D-pad and A:
//!
//! - **Restart DSlot**: its failures forgotten, started again;
//! - **Restart the device**, **Power off**;
//! - **Open a console**: the login prompt on tty1, for a keyboard or a look at the logs.

// The splash's KMS, of which this uses what puts a picture up.
#[path = "../../splash/src/kms.rs"]
#[allow(dead_code)]
mod kms;

use kms::{Kms, Screen};
use nibkit::menu::{
    colour, layout,
    raster::{Canvas, Fonts},
    top, Effect, Key, Menu, Row, Status, Value,
};
use std::{fs, io::Read, process::Command};

const UI: &str = "dslot.service";
const ROWS: [&str; 5] = [
    "Restart DSlot",
    "Restart the device",
    "Open a console",
    "Power off",
    "Go back to the previous version",
];

fn systemctl(args: &[&str]) {
    let _ = Command::new("systemctl").args(args).status();
}

fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// What failed, in two lines: how the boot UI ended, and its last error in the journal.
fn what_failed() -> (String, String) {
    let show = output(
        "systemctl",
        &[
            "show",
            UI,
            "-p",
            "Result",
            "-p",
            "ExecMainCode",
            "-p",
            "ExecMainStatus",
        ],
    );
    let get = |k: &str| {
        show.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
            .unwrap_or("")
            .to_owned()
    };
    let how = match (get("Result").as_str(), get("ExecMainCode").as_str()) {
        ("signal", _) | (_, "2") | (_, "3") => {
            format!("It crashed (signal {}).", get("ExecMainStatus"))
        }
        ("exit-code", _) => format!("It exited with status {}.", get("ExecMainStatus")),
        ("start-limit-hit", _) => "It failed again and again.".to_owned(),
        (r, _) => format!("It stopped ({r})."),
    };
    let last = output(
        "journalctl",
        &[
            "-u",
            UI,
            "-b",
            "-p",
            "err",
            "-n",
            "1",
            "-o",
            "cat",
            "--no-pager",
        ],
    );
    let last: String = last.chars().take(80).collect();
    (how, last)
}

/// RGBA to XRGB8888.
fn screen(c: &Canvas) -> Screen {
    let px =
        c.px.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
    Screen { w: c.w, h: c.h, px }
}

fn draw(kms: &Kms, fonts: &Fonts, menu: &Menu, why: &(String, String)) -> Result<(), String> {
    let mut up = Canvas::new(1.0);
    let words = top::paused_measured("DSlot stopped working", &why.0, |s, size, bold| {
        fonts.width(s, size, bold)
    });
    up.draw(fonts, &words, |_| None);
    up.text(
        fonts,
        &why.1,
        (24.0, false),
        (512.0, 450.0),
        0.5,
        colour::FAINT,
    );
    let mut down = Canvas::new(1.0);
    down.draw(fonts, &layout::menu(menu, &Status::default()), |_| None);
    kms.show(&[screen(&up), screen(&down)]).map(|_| ())
}

/// The merged gamepad's event device (board.conf's NIBOS_PAD_NAME).
fn pad() -> Option<fs::File> {
    let conf = fs::read_to_string("/usr/lib/nibos/board.conf").unwrap_or_default();
    let name = conf
        .lines()
        .find_map(|l| l.trim().strip_prefix("NIBOS_PAD_NAME="))?
        .trim_matches('"')
        .to_owned();
    fs::read_dir("/sys/class/input")
        .ok()?
        .flatten()
        .find_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let dev = fs::read_to_string(e.path().join("device/name")).ok()?;
            (n.starts_with("event") && dev.trim() == name)
                .then(|| fs::File::open(format!("/dev/input/{n}")).ok())?
        })
}

/// The next button press, as the menu's keys (blocks).
fn key(pad: &mut fs::File) -> Option<Key> {
    const EV_KEY: u16 = 1;
    const EV_ABS: u16 = 3;
    const ABS_HAT0Y: u16 = 0x11;
    const BTN_SOUTH: u16 = 0x130;
    const BTN_EAST: u16 = 0x131;
    const BTN_START: u16 = 0x13b;
    let mut ev = [0u8; 24];
    loop {
        pad.read_exact(&mut ev).ok()?;
        let kind = u16::from_ne_bytes([ev[16], ev[17]]);
        let code = u16::from_ne_bytes([ev[18], ev[19]]);
        let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
        match (kind, code, value) {
            (EV_ABS, ABS_HAT0Y, -1) => return Some(Key::Up),
            (EV_ABS, ABS_HAT0Y, 1) => return Some(Key::Down),
            (EV_KEY, BTN_SOUTH | BTN_EAST | BTN_START, 1) => return Some(Key::A),
            _ => {}
        }
    }
}

/// Whether DSlot is running or about to be: OnFailure= starts this at every failure, also the
/// ones systemd answers by restarting DSlot (Restart=on-failure, RestartSec=1). Only once the
/// restarts have run out (StartLimitBurst) does DSlot stay failed.
fn ui_coming_back() -> bool {
    let show = output(
        "systemctl",
        &["show", UI, "-p", "ActiveState", "-p", "SubState"],
    );
    show.lines().any(|l| {
        matches!(
            l,
            "ActiveState=active"
                | "ActiveState=activating"
                | "ActiveState=reloading"
                | "SubState=auto-restart"
        )
    })
}

fn main() {
    // The restart's own delay, then a look: a DSlot that is back needs no safe mode.
    std::thread::sleep(std::time::Duration::from_secs(2));
    if ui_coming_back() {
        eprintln!("nibos-safe-mode: DSlot failed and was started again: nothing to do");
        return;
    }
    let why = what_failed();
    let unconfirmed = Command::new("nibos-bootctl")
        .arg("unconfirmed")
        .status()
        .is_ok_and(|s| s.success());
    eprintln!(
        "nibos-safe-mode: DSlot stopped working: {} {}",
        why.0, why.1
    );
    let mut menu = Menu {
        tabs: vec![("Safe mode".into(), true)],
        title: "Something went wrong".into(),
        // On a slot that isn't confirmed good (just updated), the way back too: this boot
        // already counts as a failed try (docs/design/ab-rollback.md).
        rows: ROWS[..if unconfirmed { 5 } else { 4 }]
            .iter()
            .map(|r| Row::new(*r, Value::None))
            .collect(),
        row: 0,
        scroll: 0,
        hint: "A  choose".into(),
    };
    let fonts = Fonts::new();
    // The screens lit: a charge-sleep or a program may have left them off.
    let _ = fs::write("/sys/class/graphics/fb0/blank", "0\n");
    for e in fs::read_dir("/sys/class/backlight")
        .into_iter()
        .flatten()
        .flatten()
    {
        let _ = fs::write(e.path().join("bl_power"), "0\n");
    }
    let kms = match Kms::open() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("nibos-safe-mode: {e}: the console instead");
            systemctl(&["start", "--no-block", "getty@tty1.service"]);
            return;
        }
    };
    let Some(mut pad) = pad() else {
        eprintln!("nibos-safe-mode: no gamepad: the console instead");
        systemctl(&["start", "--no-block", "getty@tty1.service"]);
        return;
    };
    loop {
        if let Err(e) = draw(&kms, &fonts, &menu, &why) {
            eprintln!("nibos-safe-mode: {e}");
        }
        let Some(k) = key(&mut pad) else { return };
        if let Some(Effect::Choose(row)) = menu.key(k) {
            eprintln!("nibos-safe-mode: {}", ROWS[row]);
            drop(kms);
            match row {
                0 => {
                    systemctl(&["reset-failed", UI]);
                    systemctl(&["start", "--no-block", UI]);
                }
                1 => systemctl(&["reboot"]),
                2 => systemctl(&["start", "--no-block", "getty@tty1.service"]),
                3 => systemctl(&["poweroff"]),
                _ => {
                    let _ = Command::new("nibos-bootctl").arg("rollback").status();
                    systemctl(&["reboot"]);
                }
            }
            return;
        }
    }
}
