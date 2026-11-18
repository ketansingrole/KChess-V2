//! The low-time warning of a game played here (Play with computer, Over the board), as Lichess
//! sounds it in an online game: once as a player's clock drops under lila's threshold
//! (`low_time_threshold`), again only after an increment took it back above.

use shakmaty::Color;

/// Whether each side (White, Black) has been warned since its clock last stood above the threshold.
#[derive(Default)]
pub struct LowTime {
    warned: [bool; 2],
}

impl LowTime {
    /// `color`'s clock shows `remaining` milliseconds of `initial_seconds`: true when the warning
    /// sounds now.
    pub fn crossed(&mut self, color: Color, remaining: f64, initial_seconds: f64) -> bool {
        let side = usize::from(color == Color::Black);
        if initial_seconds <= 0. || remaining <= 0. {
            return false;
        }
        let low = remaining <= kchess_domain::training::low_time_threshold(initial_seconds);
        let sound = low && !self.warned[side];
        self.warned[side] = low;
        sound
    }

    /// Sound the warning as `color`'s clock crosses the threshold.
    pub fn warn(
        &mut self,
        color: Color,
        remaining: f64,
        initial_seconds: f64,
        cx: &mut gpui_kit::App,
    ) {
        if self.crossed(color, remaining, initial_seconds) {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LowTime;
    use shakmaty::Color::{Black, White};

    /// A five-minute clock warns under 37.5 s, once, and again after an increment lifted it.
    #[test]
    fn a_clock_warns_once_as_it_runs_low() {
        let mut low = LowTime::default();
        assert!(!low.crossed(White, 40_000., 300.));
        assert!(low.crossed(White, 37_000., 300.));
        assert!(!low.crossed(White, 30_000., 300.));
        // Black's clock is its own.
        assert!(low.crossed(Black, 20_000., 300.));
        // An increment takes White back above; running low again warns again.
        assert!(!low.crossed(White, 39_000., 300.));
        assert!(low.crossed(White, 36_000., 300.));
        // A flag that fell is not a warning; a new game, its clock full, warns afresh.
        assert!(!low.crossed(Black, 0., 300.));
        assert!(!low.crossed(White, 300_000., 300.));
        assert!(low.crossed(White, 10_000., 300.));
    }
}
