//! The low-time warning of a game played here (Play with computer, Over the board), as Lichess
//! sounds it in an online game: once as a player's clock drops under lila's threshold
//! (`low_time_threshold`), again only after an increment took it back above.

/// Whether each side (White, Black) has been warned since its clock last stood above the threshold.
#[derive(Default)]
pub struct LowTime {
    warned: [bool; 2],
}

impl LowTime {
    /// `color`'s clock shows `remaining` milliseconds of `initial_seconds`: true when the warning
    /// sounds now.
    pub fn crossed(&mut self, color: &str, remaining: f64, initial_seconds: f64) -> bool {
        let side = usize::from(color == "black");
        if initial_seconds <= 0. || remaining <= 0. {
            return false;
        }
        let low = remaining <= kchess_domain::training::low_time_threshold(initial_seconds);
        let sound = low && !self.warned[side];
        self.warned[side] = low;
        sound
    }
}

#[cfg(test)]
mod tests {
    use super::LowTime;

    /// A five-minute clock warns under 37.5 s, once, and again after an increment lifted it.
    #[test]
    fn a_clock_warns_once_as_it_runs_low() {
        let mut low = LowTime::default();
        assert!(!low.crossed("white", 40_000., 300.));
        assert!(low.crossed("white", 37_000., 300.));
        assert!(!low.crossed("white", 30_000., 300.));
        // Black's clock is its own.
        assert!(low.crossed("black", 20_000., 300.));
        // An increment takes White back above; running low again warns again.
        assert!(!low.crossed("white", 39_000., 300.));
        assert!(low.crossed("white", 36_000., 300.));
        // A flag that fell is not a warning; a new game, its clock full, warns afresh.
        assert!(!low.crossed("black", 0., 300.));
        assert!(!low.crossed("white", 300_000., 300.));
        assert!(low.crossed("white", 10_000., 300.));
    }
}
