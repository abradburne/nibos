//! The volume keys' steps, for both screens' brightness (Menu + volume, `nibos-input`) and the
//! volume (`nibos-audio`): a press goes to the next multiple of [`STEP`] percent up or down
//! (39 → 40 or 35, never 44 or 34), as DSlot's sliders do (nibkit's `backlight::step`); held,
//! the key repeats ~30 times a second, and every [`REPEAT_EVERY`]th repeat is a step, so the
//! whole range takes about three seconds.

pub const STEP: u32 = 5;
pub const REPEAT_EVERY: u32 = 5;

/// The next multiple of [`STEP`] from `from` in `dir` (+1 up, -1 down), within 0-100.
pub fn next(from: u32, dir: i32) -> u32 {
    let from = from.min(100);
    match dir.signum() {
        1 => ((from / STEP + 1) * STEP).min(100),
        -1 => from.div_ceil(STEP).saturating_sub(1) * STEP,
        _ => from,
    }
}

/// Which key events are steps: every press, and every [`REPEAT_EVERY`]th repeat after it.
#[derive(Debug, Default)]
pub struct Repeats {
    count: u32,
}

impl Repeats {
    /// An event's value (1 pressed, 2 repeat, 0 released): whether it makes a step.
    pub fn step(&mut self, value: i32) -> bool {
        match value {
            1 => {
                self.count = 0;
                true
            }
            2 => {
                self.count += 1;
                self.count.is_multiple_of(REPEAT_EVERY)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onto_multiples_of_five() {
        assert_eq!(next(39, 1), 40);
        assert_eq!(next(39, -1), 35);
        assert_eq!(next(40, 1), 45);
        assert_eq!(next(40, -1), 35);
        assert_eq!(next(98, 1), 100);
        assert_eq!(next(100, 1), 100);
        assert_eq!(next(3, -1), 0);
        assert_eq!(next(0, -1), 0);
        assert_eq!(next(150, -1), 95);
    }

    #[test]
    fn every_fifth_repeat() {
        let mut r = Repeats::default();
        assert!(r.step(1));
        let steps: Vec<bool> = (0..10).map(|_| r.step(2)).collect();
        assert_eq!(steps.iter().filter(|&&s| s).count(), 2);
        assert!(steps[4] && steps[9]);
        assert!(!r.step(0));
        assert!(r.step(1), "a new press starts again");
        assert!(!r.step(2));
    }
}
