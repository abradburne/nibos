//! The power key, as `nibos-input` handles it (logind is told to ignore it):
//!
//! - **a tap** (released before the long-press time): sleep;
//! - **a hold** (still down at the long-press time, 1 s by default): the power menu. The hold
//!   acts when the time is reached, without waiting for the release, and its release does
//!   nothing.
//!
//! The press that wakes the device from sleep is ignored: the caller drops power key events
//! that arrive within [`RESUME_HOLDOFF`] of waking. It learns of the wake itself ([`Wake`]:
//! the time asleep shows as a jump between CLOCK_BOOTTIME and CLOCK_MONOTONIC), because the
//! waking press reaches it as soon as the system thaws, before the sleep hook has left the
//! time in [`RESUMED_MARK`] (which it still reads too).
//!
//! What the power menu is belongs to the boot UI: nibos-input sends its main process SIGUSR1,
//! and it puts away a running game (SIGTERM: save and quit) and shows Sleep / Restart / Power
//! off / Cancel (`docs/engineering/device-interface.md`, "The power menu").

use std::time::{Duration, Instant};

/// Written by the sleep hook (system-sleep/nibos-resumed) when the device wakes.
pub const RESUMED_MARK: &str = "/run/nibos/resumed";
/// Power key events this soon after waking are the press that woke it.
pub const RESUME_HOLDOFF: Duration = Duration::from_secs(3);

/// Notices that the system slept since it last looked: CLOCK_BOOTTIME counts the time asleep and
/// CLOCK_MONOTONIC doesn't, so the gap between them grows by the time asleep.
#[derive(Debug, Default)]
pub struct Wake {
    gap: Option<Duration>,
    woke_at: Option<Instant>,
}

/// CLOCK_BOOTTIME - CLOCK_MONOTONIC: the time the system has spent asleep since it started.
/// (Linux only; zero elsewhere, where the tests run.)
#[cfg(target_os = "linux")]
pub fn time_asleep() -> Duration {
    let read = |clock| {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: a valid clock id and a timespec to fill.
        unsafe { libc::clock_gettime(clock, &mut ts) };
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    };
    read(libc::CLOCK_BOOTTIME).saturating_sub(read(libc::CLOCK_MONOTONIC))
}

#[cfg(not(target_os = "linux"))]
pub fn time_asleep() -> Duration {
    Duration::ZERO
}

/// A sleep shorter than this isn't one (the two clocks' readings aren't taken at once).
const SLEPT: Duration = Duration::from_millis(500);

impl Wake {
    /// The clocks' gap now (boottime - monotonic), read at `now`; whether a wake was just seen.
    pub fn observe(&mut self, gap: Duration, now: Instant) -> bool {
        let woke = self.gap.is_some_and(|g| gap > g + SLEPT);
        self.gap = Some(gap);
        if woke {
            self.woke_at = Some(now);
        }
        woke
    }

    /// Whether `now` is within [`RESUME_HOLDOFF`] of the last wake seen.
    pub fn just_woke(&self, now: Instant) -> bool {
        self.woke_at
            .is_some_and(|t| now.saturating_duration_since(t) < RESUME_HOLDOFF)
    }
}

/// What a power key press comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Tap,
    Hold,
}

/// The power key's state between events.
#[derive(Debug)]
pub struct PowerKey {
    long_press: Duration,
    /// When the key went down, while it is down and hasn't been a hold yet.
    down_since: Option<Instant>,
    /// Down, and already acted on as a hold: its release does nothing.
    held: bool,
}

impl PowerKey {
    pub fn new(long_press: Duration) -> PowerKey {
        PowerKey {
            long_press,
            down_since: None,
            held: false,
        }
    }

    /// A key event: 1 down, 0 up (repeats, 2, are ignored).
    pub fn event(&mut self, value: i32, now: Instant) -> Option<Press> {
        match value {
            1 => {
                self.down_since = Some(now);
                self.held = false;
                None
            }
            0 => {
                let was_down = self.down_since.take();
                if std::mem::take(&mut self.held) {
                    return None;
                }
                match was_down {
                    Some(t) if now.duration_since(t) >= self.long_press => Some(Press::Hold),
                    Some(_) => Some(Press::Tap),
                    // A release without its press: the press woke the device, or came
                    // before nibos-input started.
                    None => None,
                }
            }
            _ => None,
        }
    }

    /// Forgets a press in progress (the system slept meanwhile).
    pub fn reset(&mut self) {
        self.down_since = None;
        self.held = false;
    }

    /// When the caller should call [`PowerKey::tick`] next: the moment the key, if still
    /// down, becomes a hold.
    pub fn deadline(&self) -> Option<Instant> {
        self.down_since.map(|t| t + self.long_press)
    }

    /// The key is still down at `now`: a hold once the long-press time has passed.
    pub fn tick(&mut self, now: Instant) -> Option<Press> {
        let t = self.down_since?;
        if now.duration_since(t) < self.long_press {
            return None;
        }
        self.down_since = None;
        self.held = true;
        Some(Press::Hold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> (PowerKey, Instant) {
        (PowerKey::new(Duration::from_millis(1000)), Instant::now())
    }

    fn ms(t: Instant, n: u64) -> Instant {
        t + Duration::from_millis(n)
    }

    #[test]
    fn a_tap_sleeps_on_release() {
        let (mut k, t) = key();
        assert_eq!(k.event(1, t), None);
        assert_eq!(k.tick(ms(t, 500)), None);
        assert_eq!(k.event(0, ms(t, 200)), Some(Press::Tap));
        assert_eq!(k.deadline(), None);
    }

    #[test]
    fn a_hold_acts_at_the_time_and_its_release_does_nothing() {
        let (mut k, t) = key();
        k.event(1, t);
        assert_eq!(k.deadline(), Some(ms(t, 1000)));
        assert_eq!(k.tick(ms(t, 999)), None);
        assert_eq!(k.tick(ms(t, 1000)), Some(Press::Hold));
        assert_eq!(k.tick(ms(t, 1500)), None, "once");
        assert_eq!(k.event(0, ms(t, 3000)), None);
    }

    #[test]
    fn a_late_tick_still_sees_a_long_release_as_a_hold() {
        // The release can arrive before the tick that would have caught the hold.
        let (mut k, t) = key();
        k.event(1, t);
        assert_eq!(k.event(0, ms(t, 1200)), Some(Press::Hold));
    }

    #[test]
    fn a_wake_is_the_clocks_gap_jumping() {
        let t = Instant::now();
        let mut w = Wake::default();
        assert!(
            !w.observe(Duration::from_secs(100), t),
            "the first look sets the baseline"
        );
        assert!(
            !w.observe(Duration::from_millis(100_200), ms(t, 10)),
            "jitter isn't a sleep"
        );
        assert!(!w.just_woke(ms(t, 20)));
        // Slept 30 s: the gap grew by that.
        assert!(w.observe(Duration::from_millis(130_200), ms(t, 30)));
        assert!(w.just_woke(ms(t, 30)), "the waking press, at once");
        assert!(w.just_woke(ms(t, 2900)));
        assert!(!w.just_woke(ms(t, 3100)), "a tap 3 s later sleeps again");
        assert!(!w.observe(Duration::from_millis(130_200), ms(t, 4000)));
    }

    #[test]
    fn reset_forgets_a_press_in_progress() {
        let (mut k, t) = key();
        k.event(1, t);
        k.reset();
        assert_eq!(k.deadline(), None);
        assert_eq!(k.event(0, ms(t, 200)), None, "its release is no tap");
    }

    #[test]
    fn a_release_without_its_press_is_ignored() {
        let (mut k, t) = key();
        assert_eq!(k.event(0, t), None);
        assert_eq!(k.event(2, t), None);
    }
}
