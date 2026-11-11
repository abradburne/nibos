//! nibos-battery: watches the battery while the device runs from it ([`nibos_hw::battery`]).
//!
//! - At the warning lines it leaves a notice for the shelf (`/run/nibos/notice/battery`,
//!   which DSlot shows), says the level to apps in [`ALERT_PATH`] (`warn 9`, `low 5`), and
//!   blinks the board's low-battery LED (`NIBOS_LOW_BATTERY_LED`), faster when low.
//! - At the shutdown line it powers the device off: systemd asks every program to quit with
//!   SIGTERM, on which a running game saves its resume state, and the cards are unmounted
//!   cleanly.
//! - Plugging the charger in clears all of it.
//!
//! ```text
//! nibos-battery            run (the service)
//! nibos-battery --once     read the battery once and print what the policy says; changes nothing
//! ```
//!
//! The limits are `NIBOS_BATTERY_*` in the board's description ([`Limits::from_env`]).

use nibos_hw::battery::{alert_line, Level, Limits, Policy, Reading, ALERT_PATH};
use nibos_hw::board::{parse_env, LOCAL_PATH, PATH};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{exit, Command};
use std::thread::sleep;

const BATTERY: &str = "/run/nibos/battery";
const CHARGER: &str = "/run/nibos/charger";
const NOTICE: &str = "/run/nibos/notice/battery";

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("nibos-battery: {msg}");
    exit(1);
}

fn value(dir: &str, file: &str) -> Option<String> {
    fs::read_to_string(Path::new(dir).join(file)).ok()
}

fn read() -> Option<Reading> {
    Reading::from_sysfs(
        value(BATTERY, "capacity").as_deref(),
        value(BATTERY, "status").as_deref(),
        // The RK817's driver has no voltage_now, only voltage_avg (its ADC's running average).
        value(BATTERY, "voltage_now")
            .or_else(|| value(BATTERY, "voltage_avg"))
            .as_deref(),
        value(CHARGER, "online").as_deref(),
    )
}

/// Writes `text` over `path` in place (not by renaming), so a reader's inotify watch on it
/// sees the change.
fn write_in_place(path: &str, text: &str) {
    if let Some(dir) = Path::new(path).parent() {
        let _ = fs::create_dir_all(dir);
    }
    let result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()));
    if let Err(e) = result {
        eprintln!("nibos-battery: {path}: {e}");
    }
}

/// The low-battery LED, if the board has one: blinking for a warning, on for the end.
struct Led(Option<String>);

impl Led {
    fn set(&self, level: Level) {
        let Some(name) = &self.0 else { return };
        let dir = format!("/sys/class/leds/{name}");
        let put = |file: &str, v: &str| {
            if let Err(e) = fs::write(format!("{dir}/{file}"), v) {
                eprintln!("nibos-battery: {dir}/{file}: {e}");
            }
        };
        let blink = |on_ms: &str, off_ms: &str| {
            // The timer trigger makes delay_on and delay_off appear; without it, steady on.
            if fs::write(format!("{dir}/trigger"), "timer").is_ok() {
                put("delay_on", on_ms);
                put("delay_off", off_ms);
            } else {
                put("brightness", "1");
            }
        };
        match level {
            Level::Ok => {
                put("trigger", "none");
                put("brightness", "0");
            }
            Level::Warn => blink("200", "2800"),
            Level::Low => blink("200", "800"),
            Level::Critical => {
                put("trigger", "none");
                put("brightness", "1");
            }
        }
    }
}

/// Says the level everywhere it is said.
fn show(level: Level, percent: u8, led: &Led) {
    write_in_place(ALERT_PATH, &alert_line(level, percent));
    match level.notice(percent) {
        Some(text) => write_in_place(NOTICE, &format!("{text}\n")),
        None => {
            let _ = fs::remove_file(NOTICE);
        }
    }
    led.set(level);
}

fn main() {
    let text = fs::read_to_string(PATH).unwrap_or_else(|e| fail(format!("{PATH}: {e}")));
    let local = fs::read_to_string(LOCAL_PATH).unwrap_or_default();
    let env = parse_env(&format!("{text}\n{local}"));
    let limits = Limits::from_env(&env).unwrap_or_else(|e| fail(format!("{PATH}: {e}")));
    let led = Led(env
        .get("NIBOS_LOW_BATTERY_LED")
        .filter(|v| !v.is_empty())
        .cloned());
    let mut policy = Policy::new(limits);

    if std::env::args().nth(1).as_deref() == Some("--once") {
        let r = read().unwrap_or_else(|| fail(format!("no battery at {BATTERY}")));
        let level = policy.observe(r).level;
        println!(
            "{}% {} {}: {} (critical after {} readings in a row under {}% or {})",
            r.percent,
            r.voltage_uv
                .map_or("?".into(), |uv| format!("{:.3} V", uv as f64 / 1e6)),
            if r.discharging {
                "on battery"
            } else {
                "charging"
            },
            level.name(),
            limits.confirm,
            limits.shutdown_percent,
            limits
                .shutdown_uv
                .map_or("no voltage line".into(), |uv| format!(
                    "{:.3} V",
                    uv as f64 / 1e6
                )),
        );
        return;
    }

    eprintln!("nibos-battery: watching, {limits:?}");
    // Start from a clean slate: nothing left over from a previous run of the daemon.
    show(Level::Ok, 100, &led);
    let mut missing = false;
    let mut said = 100;
    loop {
        let Some(r) = read() else {
            if !missing {
                eprintln!("nibos-battery: no battery at {BATTERY}; trying again");
                missing = true;
            }
            sleep(Level::Ok.every());
            continue;
        };
        missing = false;
        let step = policy.observe(r);
        if step.changed {
            eprintln!(
                "nibos-battery: {} at {}%{}",
                step.level.name(),
                r.percent,
                r.voltage_uv
                    .map_or(String::new(), |uv| format!(", {:.3} V", uv as f64 / 1e6))
            );
            show(step.level, r.percent, &led);
            said = r.percent;
            if step.level == Level::Critical {
                // A game is asked to save and quit as the boot UI's service stops.
                match Command::new("systemctl").arg("poweroff").status() {
                    Ok(s) if s.success() => {}
                    other => eprintln!("nibos-battery: systemctl poweroff: {other:?}"),
                }
                // Shutdown is under way (or failed, and the next readings try again).
            }
        } else if step.level != Level::Ok && r.percent != said {
            // Keep the percent in the alert current for apps that show it.
            write_in_place(ALERT_PATH, &alert_line(step.level, r.percent));
            said = r.percent;
        }
        if step.level == Level::Critical && !step.changed {
            // Still running at critical: poweroff didn't take. Ask again.
            let _ = Command::new("systemctl").arg("poweroff").status();
        }
        sleep(step.level.every());
    }
}
