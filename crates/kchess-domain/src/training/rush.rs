//! Storm, Streak and Rush: a run's state and the rules that change it.
//! Each transition takes the state and returns the next one.

use serde::{Deserialize, Serialize};

use super::pad2;
use crate::js;

/// `[combo at which the bonus is earned, bonus in milliseconds]`
const COMBO_LEVELS: [(u32, f64); 4] = [(5, 3000.0), (12, 5000.0), (20, 7000.0), (30, 10_000.0)];

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ladder {
    pub from: f64,
    pub to: f64,
    pub count: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RushConfig {
    pub mode: String,
    pub variant: String,
    pub title: String,
    pub rules: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikes: Option<f64>,
    pub skips: f64,
    pub penalty_ms: f64,
    pub combo: bool,
    pub ladder: Ladder,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RushState {
    pub score: f64,
    pub moves: f64,
    pub mistakes: f64,
    pub combo: f64,
    pub max_combo: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_left_ms: Option<f64>,
    pub skips_left: f64,
    pub highest: f64,
    pub bonus_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<String>,
}

/// JavaScript truthiness of a number: zero and NaN are falsy.
fn truthy(n: f64) -> bool {
    n != 0.0 && !n.is_nan()
}

/// The bonus for reaching `combo`; every tenth combo past 30 earns 10 seconds.
pub fn combo_bonus_ms(combo: f64) -> f64 {
    if let Some(&(_, ms)) = COMBO_LEVELS.iter().find(|(at, _)| f64::from(*at) == combo) {
        return ms;
    }
    if combo > 30.0 && combo % 10.0 == 0.0 {
        10_000.0
    } else {
        0.0
    }
}

/// Progress toward the next bonus, from 0 to 1.
pub fn combo_progress(combo: f64) -> f64 {
    let previous = COMBO_LEVELS
        .iter()
        .rev()
        .map(|(at, _)| f64::from(*at))
        .find(|at| *at <= combo);
    let next = COMBO_LEVELS
        .iter()
        .map(|(at, _)| f64::from(*at))
        .find(|at| *at > combo)
        .unwrap_or(if combo < 30.0 {
            30.0
        } else {
            (combo / 10.0).floor() * 10.0 + 10.0
        });
    let base = if combo >= 30.0 {
        (combo / 10.0).floor() * 10.0
    } else {
        previous.unwrap_or(0.0)
    };
    js::min(1.0, (combo - base) / (next - base))
}

/// `m:ss`, or `0:ss.t` under ten seconds, where tenths matter.
pub fn format_run_clock(ms: f64) -> String {
    let clamped = js::max(0.0, ms);
    let total_seconds = (clamped / 1000.0).floor();
    let minutes = (total_seconds / 60.0).floor();
    let seconds = pad2(total_seconds % 60.0);
    if clamped < 10_000.0 && clamped > 0.0 {
        format!("0:{seconds}.{}", ((clamped % 1000.0) / 100.0).floor())
    } else {
        format!("{}:{seconds}", js::number_to_string(minutes))
    }
}

impl RushState {
    pub fn new(config: &RushConfig) -> Self {
        Self {
            score: 0.0,
            moves: 0.0,
            mistakes: 0.0,
            combo: 0.0,
            max_combo: 0.0,
            time_left_ms: config.duration_ms,
            skips_left: config.skips,
            highest: 0.0,
            bonus_ms: 0.0,
            over: None,
        }
    }

    fn is_over(&self) -> bool {
        self.over.is_some()
    }

    /// A correct move. Returns the bonus time it earned.
    pub fn correct_move(&mut self, config: &RushConfig) -> f64 {
        if self.is_over() {
            return 0.0;
        }
        self.moves += 1.0;
        self.combo += 1.0;
        self.max_combo = js::max(self.max_combo, self.combo);
        let bonus = if config.combo {
            combo_bonus_ms(self.combo)
        } else {
            0.0
        };
        if let Some(left) = self.time_left_ms.as_mut()
            && truthy(bonus)
        {
            *left += bonus;
            self.bonus_ms += bonus;
        }
        bonus
    }

    pub fn puzzle_solved(&mut self, rating: f64) {
        if self.is_over() {
            return;
        }
        self.score += 1.0;
        self.highest = js::max(self.highest, rating);
    }

    /// A wrong move breaks the combo and costs time or a strike.
    pub fn mistake(&mut self, config: &RushConfig) {
        if self.is_over() {
            return;
        }
        self.mistakes += 1.0;
        self.combo = 0.0;
        if truthy(config.penalty_ms)
            && let Some(left) = self.time_left_ms
        {
            let left = left - config.penalty_ms;
            self.time_left_ms = Some(left);
            if left <= 0.0 {
                self.finish("time");
            }
        }
        if let Some(strikes) = config.strikes.filter(|s| truthy(*s))
            && self.mistakes >= strikes
        {
            self.finish("strikes");
        }
    }

    /// Spends a skip. False when none is left.
    pub fn skip(&mut self) -> bool {
        if self.is_over() || self.skips_left <= 0.0 {
            return false;
        }
        self.skips_left -= 1.0;
        true
    }

    pub fn tick(&mut self, elapsed_ms: f64) {
        if self.is_over() {
            return;
        }
        if let Some(left) = self.time_left_ms {
            let left = left - elapsed_ms;
            self.time_left_ms = Some(left);
            if left <= 0.0 {
                self.finish("time");
            }
        }
    }

    /// Ends the run for `reason`; time left is never negative.
    pub fn finish(&mut self, reason: &str) {
        if self.is_over() {
            return;
        }
        self.over = Some(reason.to_string());
        if let Some(left) = self.time_left_ms {
            self.time_left_ms = Some(js::max(0.0, left));
        }
    }

    /// Correct moves as a percentage of all moves, rounded.
    pub fn accuracy(&self) -> f64 {
        let total = self.moves + self.mistakes;
        if truthy(total) {
            js::round(self.moves / total * 100.0)
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storm() -> RushConfig {
        RushConfig {
            mode: "storm".into(),
            variant: "standard".into(),
            title: "Storm".into(),
            rules: String::new(),
            duration_ms: Some(180_000.0),
            strikes: None,
            skips: 0.0,
            penalty_ms: 10_000.0,
            combo: true,
            ladder: Ladder {
                from: 800.0,
                to: 2600.0,
                count: 100.0,
            },
        }
    }

    #[test]
    fn storm_run_matches_the_smoke_test() {
        let config = storm();
        let mut run = RushState::new(&config);
        for _ in 0..5 {
            run.correct_move(&config);
        }
        assert_eq!(run.combo, 5.0);
        assert_eq!(run.time_left_ms, Some(183_000.0));
        assert_eq!(run.bonus_ms, 3000.0);
        run.mistake(&config);
        assert_eq!((run.combo, run.time_left_ms), (0.0, Some(173_000.0)));
        assert!(run.over.is_none());
        run.puzzle_solved(1200.0);
        run.tick(173_000.0);
        assert_eq!(run.over.as_deref(), Some("time"));
        assert_eq!(
            (run.time_left_ms, run.score, run.highest),
            (Some(0.0), 1.0, 1200.0)
        );
    }

    #[test]
    fn combo_helpers_and_clock_format() {
        let bonuses: Vec<f64> = [5.0, 12.0, 20.0, 30.0, 40.0, 41.0]
            .into_iter()
            .map(combo_bonus_ms)
            .collect();
        assert_eq!(bonuses, [3000.0, 5000.0, 7000.0, 10_000.0, 10_000.0, 0.0]);
        let bar: Vec<f64> = [0.0, 5.0, 30.0, 35.0]
            .into_iter()
            .map(combo_progress)
            .collect();
        assert_eq!(bar, [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(format_run_clock(183_000.0), "3:03");
        assert_eq!(format_run_clock(9_400.0), "0:09.4");
        assert_eq!(format_run_clock(0.0), "0:00");
    }

    #[test]
    fn accuracy_rounds_half_up() {
        let config = RushConfig {
            strikes: Some(3.0),
            ..storm()
        };
        let mut run = RushState::new(&config);
        run.mistake(&config);
        run.mistake(&config);
        run.correct_move(&config);
        run.mistake(&config);
        assert_eq!(run.over.as_deref(), Some("strikes"));
        assert_eq!(run.accuracy(), 25.0);
    }
}
