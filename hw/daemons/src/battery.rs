//! The low battery, as `nibos-battery` watches it: a warning while there is time to find the
//! charger, and a clean power-off before the battery cuts out under a running game.
//!
//! Each reading (the gauge's percent, the cell's voltage under load, and whether the device is
//! running from the battery) moves the [`Policy`] between [`Level`]s:
//!
//! - **ok**, and back to it at once when the charger is plugged in;
//! - **warn** at [`Limits::warn_percent`] (10%), **low** at [`Limits::low_percent`] (5%): the
//!   daemon says so on the shelf and to apps, and lights the low-battery LED;
//! - **critical** at [`Limits::shutdown_percent`] (2%) or with the voltage at or under
//!   [`Limits::shutdown_uv`]: the daemon powers the device off, which asks a running game to
//!   save and quit first (systemd's SIGTERM, as for the power menu).
//!
//! The voltage sags while a game works the CPU and recovers when it rests, so one reading under
//! the line is not enough: critical needs [`Limits::confirm`] readings in a row, read
//! [`Level::every`] apart. Percent levels only go back up with [`HYSTERESIS`] to spare, so a
//! gauge wavering on a line doesn't flap.
//!
//! The limits are the board's (`NIBOS_BATTERY_*` in `board.conf`, [`Limits::from_env`]).

use std::collections::HashMap;
use std::time::Duration;

/// How far above a percent line the gauge must climb before its level is left.
pub const HYSTERESIS: u8 = 2;

/// Where the daemon tells apps the level ([`Level::name`] and the percent: `low 5`), written in
/// place so an inotify watch on it sees every change.
pub const ALERT_PATH: &str = "/run/nibos/battery-alert";

/// One look at the battery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The gauge's charge, 0-100.
    pub percent: u8,
    /// The cell's voltage now, in µV, if the gauge gives it.
    pub voltage_uv: Option<i64>,
    /// Running from the battery: no charger plugged in.
    pub discharging: bool,
}

impl Reading {
    /// A reading from the power supplies' sysfs values: the battery's `capacity`, `status` and
    /// `voltage_now`, and the charger's `online`. `None` without a capacity.
    pub fn from_sysfs(
        capacity: Option<&str>,
        status: Option<&str>,
        voltage_now: Option<&str>,
        charger_online: Option<&str>,
    ) -> Option<Reading> {
        let percent = capacity?.trim().parse::<i64>().ok()?.clamp(0, 100) as u8;
        let plugged = charger_online.map(str::trim) == Some("1");
        // "Not charging" is a charger plugged in that holds the charge (battery protect):
        // not running down.
        let charging = matches!(
            status.map(str::trim),
            Some("Charging" | "Full" | "Not charging")
        );
        Some(Reading {
            percent,
            voltage_uv: voltage_now.and_then(|v| v.trim().parse().ok()),
            discharging: !plugged && !charging,
        })
    }
}

/// Where the lines are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// `NIBOS_BATTERY_WARN_PERCENT`: the first warning (10).
    pub warn_percent: u8,
    /// `NIBOS_BATTERY_LOW_PERCENT`: the second (5).
    pub low_percent: u8,
    /// `NIBOS_BATTERY_SHUTDOWN_PERCENT`: power off at or under this (2).
    pub shutdown_percent: u8,
    /// `NIBOS_BATTERY_SHUTDOWN_UV`: power off with the voltage at or under this, in µV; none
    /// if not given.
    pub shutdown_uv: Option<i64>,
    /// `NIBOS_BATTERY_CONFIRM`: readings in a row under a shutdown line before acting (3).
    pub confirm: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            warn_percent: 10,
            low_percent: 5,
            shutdown_percent: 2,
            shutdown_uv: None,
            confirm: 3,
        }
    }
}

impl Limits {
    /// The limits in a board description's `NAME=value` pairs ([`crate::board::parse_env`]),
    /// the defaults where a name is left out.
    pub fn from_env(env: &HashMap<String, String>) -> Result<Limits, String> {
        let get = |name: &str| env.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
        let percent = |name: &str, default: u8| -> Result<u8, String> {
            get(name).map_or(Ok(default), |v| {
                v.parse::<u8>()
                    .ok()
                    .filter(|&p| p <= 50)
                    .ok_or(format!("{name}={v} is not 0-50"))
            })
        };
        let d = Limits::default();
        let limits = Limits {
            warn_percent: percent("NIBOS_BATTERY_WARN_PERCENT", d.warn_percent)?,
            low_percent: percent("NIBOS_BATTERY_LOW_PERCENT", d.low_percent)?,
            shutdown_percent: percent("NIBOS_BATTERY_SHUTDOWN_PERCENT", d.shutdown_percent)?,
            shutdown_uv: get("NIBOS_BATTERY_SHUTDOWN_UV")
                .map(|v| {
                    v.parse::<i64>()
                        .ok()
                        .filter(|uv| (2_500_000..=4_000_000).contains(uv))
                        .ok_or(format!(
                            "NIBOS_BATTERY_SHUTDOWN_UV={v} is not 2500000-4000000"
                        ))
                })
                .transpose()?,
            confirm: get("NIBOS_BATTERY_CONFIRM").map_or(Ok(d.confirm), |v| {
                v.parse::<u32>()
                    .ok()
                    .filter(|n| (1..=20).contains(n))
                    .ok_or(format!("NIBOS_BATTERY_CONFIRM={v} is not 1-20"))
            })?,
        };
        if !(limits.shutdown_percent < limits.low_percent
            && limits.low_percent < limits.warn_percent)
        {
            return Err(format!(
                "the battery's lines must go down: warn {} > low {} > shutdown {}",
                limits.warn_percent, limits.low_percent, limits.shutdown_percent
            ));
        }
        Ok(limits)
    }
}

/// How low the battery is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Under the shutdown line: power off.
    Critical,
    /// Under the second line.
    Low,
    /// Under the first line.
    Warn,
    /// Above them, or charging.
    #[default]
    Ok,
}

impl Level {
    /// The level's word in [`ALERT_PATH`].
    pub const fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Low => "low",
            Level::Critical => "critical",
        }
    }

    /// How long until the next reading: often enough near the end to catch the sag before the
    /// cell cuts out, seldom while there is plenty.
    pub const fn every(self) -> Duration {
        match self {
            Level::Ok => Duration::from_secs(30),
            Level::Warn => Duration::from_secs(15),
            Level::Low | Level::Critical => Duration::from_secs(5),
        }
    }

    /// What the shelf says, if anything.
    pub fn notice(self, percent: u8) -> Option<String> {
        match self {
            Level::Ok => None,
            Level::Warn => Some(format!(
                "Battery low: {percent}% left. Plug in the charger soon."
            )),
            Level::Low => Some(format!(
                "Battery very low: {percent}% left. Plug in the charger now: \
                 nibOS will save your game and power off before it runs out."
            )),
            Level::Critical => Some("Battery empty: saving and powering off.".into()),
        }
    }
}

/// The level, kept between readings.
#[derive(Clone, Debug)]
pub struct Policy {
    limits: Limits,
    level: Level,
    under: u32,
}

/// What a reading changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The level now.
    pub level: Level,
    /// It differs from the last reading's.
    pub changed: bool,
}

impl Policy {
    pub fn new(limits: Limits) -> Policy {
        Policy {
            limits,
            level: Level::Ok,
            under: 0,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// Takes a reading.
    pub fn observe(&mut self, r: Reading) -> Step {
        let before = self.level;
        let l = &self.limits;
        if !r.discharging {
            self.under = 0;
            self.level = Level::Ok;
        } else {
            let under_line = r.percent <= l.shutdown_percent
                || l.shutdown_uv
                    .zip(r.voltage_uv)
                    .is_some_and(|(line, v)| v <= line);
            self.under = if under_line { self.under + 1 } else { 0 };
            self.level = if self.under >= l.confirm {
                Level::Critical
            } else {
                // The percent's level; leaving one for a better one needs HYSTERESIS to spare.
                let by_percent = |p: u8| {
                    if p <= l.low_percent {
                        Level::Low
                    } else if p <= l.warn_percent {
                        Level::Warn
                    } else {
                        Level::Ok
                    }
                };
                let now = by_percent(r.percent);
                // The level held so far (critical, never left once reached, counts as low).
                let worst = before.max(Level::Low);
                if now > worst && by_percent(r.percent.saturating_sub(HYSTERESIS)) <= worst {
                    worst
                } else if self.under > 0 {
                    // On the way to critical: at least low, so the warning is up.
                    now.min(Level::Low)
                } else {
                    now
                }
            };
        }
        Step {
            level: self.level,
            changed: self.level != before,
        }
    }
}

/// [`ALERT_PATH`]'s line for a level and percent.
pub fn alert_line(level: Level, percent: u8) -> String {
    format!("{} {percent}\n", level.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(percent: u8, mv: i64) -> Reading {
        Reading {
            percent,
            voltage_uv: Some(mv * 1000),
            discharging: true,
        }
    }

    fn ds_plus() -> Policy {
        Policy::new(Limits {
            shutdown_uv: Some(3_450_000),
            ..Limits::default()
        })
    }

    #[test]
    fn sysfs_readings() {
        let r = Reading::from_sysfs(
            Some("7\n"),
            Some("Discharging\n"),
            Some("3650000\n"),
            Some("0"),
        )
        .unwrap();
        assert_eq!(r, on_battery(7, 3650));
        // Battery protect holding the charge with the charger in: not running down.
        let held = Reading::from_sysfs(Some("80"), Some("Not charging"), None, Some("1")).unwrap();
        assert!(!held.discharging);
        // A charger the gauge hasn't noticed yet still counts.
        let plugged = Reading::from_sysfs(Some("4"), Some("Discharging"), None, Some("1")).unwrap();
        assert!(!plugged.discharging);
        assert_eq!(Reading::from_sysfs(None, Some("Full"), None, None), None);
        assert_eq!(
            Reading::from_sysfs(Some("120"), None, None, None).map(|r| r.percent),
            Some(100)
        );
    }

    #[test]
    fn levels_going_down() {
        let mut p = ds_plus();
        assert_eq!(p.observe(on_battery(50, 3800)).level, Level::Ok);
        let warn = p.observe(on_battery(10, 3660));
        assert_eq!((warn.level, warn.changed), (Level::Warn, true));
        assert!(!p.observe(on_battery(9, 3660)).changed);
        assert_eq!(p.observe(on_battery(5, 3600)).level, Level::Low);
    }

    #[test]
    fn critical_needs_readings_in_a_row() {
        let mut p = ds_plus();
        p.observe(on_battery(6, 3620));
        // A sag under load, then the voltage recovers: no power-off.
        assert_eq!(p.observe(on_battery(6, 3440)).level, Level::Low);
        assert_eq!(p.observe(on_battery(6, 3440)).level, Level::Low);
        assert_eq!(p.observe(on_battery(6, 3500)).level, Level::Low);
        assert_eq!(p.observe(on_battery(6, 3440)).level, Level::Low);
        assert_eq!(p.observe(on_battery(6, 3440)).level, Level::Low);
        let step = p.observe(on_battery(6, 3430));
        assert_eq!((step.level, step.changed), (Level::Critical, true));
    }

    #[test]
    fn critical_by_percent_alone() {
        // A gauge without a voltage still powers off at the percent line.
        let mut p = Policy::new(Limits::default());
        let empty = Reading {
            percent: 2,
            voltage_uv: None,
            discharging: true,
        };
        assert_eq!(p.observe(empty).level, Level::Low);
        assert_eq!(p.observe(empty).level, Level::Low);
        assert_eq!(p.observe(empty).level, Level::Critical);
    }

    #[test]
    fn the_voltage_line_warns_before_it_acts() {
        // 40% on the gauge but the cell at the line (a gauge that has drifted): the warning
        // comes up at the first reading under it.
        let mut p = ds_plus();
        assert_eq!(p.observe(on_battery(40, 3440)).level, Level::Low);
    }

    #[test]
    fn plugging_in_clears_it_at_once() {
        let mut p = ds_plus();
        p.observe(on_battery(3, 3500));
        p.observe(on_battery(3, 3440));
        let charging = Reading {
            discharging: false,
            ..on_battery(3, 3700)
        };
        let step = p.observe(charging);
        assert_eq!((step.level, step.changed), (Level::Ok, true));
        // And the count under the line starts again.
        p.observe(on_battery(3, 3440));
        p.observe(on_battery(3, 3440));
        assert_eq!(p.level(), Level::Low);
    }

    #[test]
    fn a_wavering_gauge_does_not_flap() {
        let mut p = ds_plus();
        p.observe(on_battery(10, 3660));
        assert_eq!(p.observe(on_battery(11, 3670)).level, Level::Warn);
        assert_eq!(p.observe(on_battery(12, 3670)).level, Level::Warn);
        assert_eq!(p.observe(on_battery(13, 3680)).level, Level::Ok);
    }

    #[test]
    fn limits_from_the_board() {
        let env = crate::board::parse_env(
            "NIBOS_BATTERY_SHUTDOWN_UV=3450000\nNIBOS_BATTERY_WARN_PERCENT=15\n",
        );
        let limits = Limits::from_env(&env).unwrap();
        assert_eq!(limits.shutdown_uv, Some(3_450_000));
        assert_eq!(limits.warn_percent, 15);
        assert_eq!(limits.low_percent, 5);
        assert_eq!(
            Limits::from_env(&HashMap::new()).unwrap(),
            Limits::default()
        );
        let upside_down = crate::board::parse_env("NIBOS_BATTERY_LOW_PERCENT=12\n");
        assert!(Limits::from_env(&upside_down)
            .unwrap_err()
            .contains("go down"));
        let volts = crate::board::parse_env("NIBOS_BATTERY_SHUTDOWN_UV=3.4\n");
        assert!(Limits::from_env(&volts).is_err());
    }

    #[test]
    fn alert_lines_and_notices() {
        assert_eq!(alert_line(Level::Low, 5), "low 5\n");
        assert_eq!(Level::Ok.notice(50), None);
        assert!(Level::Warn.notice(9).unwrap().contains("9%"));
    }
}
