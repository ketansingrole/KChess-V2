//! What a training attempt counts as, and the day streak derived from the attempt log.
//!
//! The puzzle session (`puzzle_session`) classifies each attempt for its tally, and the puzzle
//! service (`crates/kchess-core/src/puzzles`) classifies it again when it stores it, so the log and
//! the session agree. Rated Lichess results stay binary; these outcomes are local only.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Out;

/// How an attempt went. Each player move of the solution is scored by the first try at it, so a
/// mistake does not end the scoring: wrong, right, wrong on a 3-move puzzle is 1 found, 2 wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Every player move found, no hint.
    Solved,
    /// Every player move found after asking for a hint.
    Hinted,
    /// Some player moves found at the first try, others missed.
    Partial,
    /// No player move found at the first try.
    Missed,
    /// The solution was shown before any wrong move.
    Revealed,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Solved => "solved",
            Outcome::Hinted => "hinted",
            Outcome::Partial => "partial",
            Outcome::Missed => "missed",
            Outcome::Revealed => "revealed",
        }
    }

    pub fn parse(name: &str) -> Option<Outcome> {
        [
            Outcome::Solved,
            Outcome::Hinted,
            Outcome::Partial,
            Outcome::Missed,
            Outcome::Revealed,
        ]
        .into_iter()
        .find(|outcome| outcome.name() == name)
    }

    /// A solved puzzle, with or without a hint: what the day streak counts.
    pub fn solved(self) -> bool {
        matches!(self, Outcome::Solved | Outcome::Hinted)
    }
}

/// The facts of a finished attempt.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub win: bool,
    /// The solution was shown.
    #[serde(default)]
    pub revealed: bool,
    /// A hint was asked for at some point before the verdict.
    #[serde(default)]
    pub hinted: bool,
    /// Player moves right at the first try.
    #[serde(default)]
    pub found: u32,
    /// Player moves wrong at the first try (each counts once, however many tries followed).
    #[serde(default)]
    pub wrong: u32,
    /// Player moves in the solution.
    #[serde(default)]
    pub total: u32,
}

/// `puzzleAttemptOutcome`: a win (every move right at the first try) is solved, with or without a
/// hint; otherwise a reveal before any wrong move is revealed, and an attempt with a wrong move is
/// partial when some move was found at the first try, else missed.
pub fn classify(verdict: &Verdict) -> Outcome {
    if verdict.win {
        return if verdict.hinted {
            Outcome::Hinted
        } else {
            Outcome::Solved
        };
    }
    if verdict.revealed && verdict.wrong == 0 {
        return Outcome::Revealed;
    }
    if verdict.found > 0 {
        Outcome::Partial
    } else {
        Outcome::Missed
    }
}

/// Player moves found and tried by an attempt, first tries only. A solve finds them all (a mate other
/// than the solution's can end it early); moves never tried (shown, or left) count as neither.
pub fn moves(outcome: Outcome, verdict: &Verdict) -> (u32, u32) {
    if outcome.solved() {
        let all = verdict.total.max(verdict.found);
        return (all, all);
    }
    (verdict.found, verdict.found + verdict.wrong)
}

pub use crate::calendar::day_number;

/// The day streak: consecutive calendar days with a solved puzzle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStreak {
    /// Ending today, or yesterday while today has no solve yet; 0 once a day was missed.
    pub current: u32,
    /// The longest run ever.
    pub best: u32,
}

/// `puzzleDayStreak`: the streak of `days` (days with a solve, in any order, repeats allowed) as of
/// `today`. Unreadable days are ignored; days after `today` (a clock moved back) end no run.
pub fn day_streak<'a>(days: impl IntoIterator<Item = &'a str>, today: &str) -> DayStreak {
    let mut numbers: Vec<i64> = days.into_iter().filter_map(day_number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    let mut streak = DayStreak::default();
    let mut run = 0u32;
    let mut previous: Option<i64> = None;
    for &day in &numbers {
        run = if previous == Some(day - 1) {
            run + 1
        } else {
            1
        };
        streak.best = streak.best.max(run);
        previous = Some(day);
    }
    let Some(today) = day_number(today) else {
        return streak;
    };
    // The run that reaches today or yesterday, counted back from its last day.
    let Some(end) = [today, today - 1]
        .into_iter()
        .find(|day| numbers.binary_search(day).is_ok())
    else {
        return streak;
    };
    let mut day = end;
    while numbers.binary_search(&day).is_ok() {
        streak.current += 1;
        day -= 1;
    }
    streak
}

fn verdict_arg(args: &[Value]) -> Result<Verdict, String> {
    serde_json::from_value(args.first().cloned().unwrap_or(Value::Null))
        .map_err(|error| format!("verdict: {error}"))
}

/// This module's methods for `api::call`; None when the method is not one of them.
pub(super) fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "puzzleAttemptOutcome" => verdict_arg(args).map(|verdict| {
            let outcome = classify(&verdict);
            let (found, tried) = moves(outcome, &verdict);
            json!({ "outcome": outcome.name(), "found": found, "tried": tried })
        }),
        "puzzleDayStreak" => {
            let days: Vec<&str> = args
                .first()
                .and_then(Value::as_array)
                .map(|days| days.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let today = args.get(1).and_then(Value::as_str).unwrap_or_default();
            serde_json::to_value(day_streak(days, today)).map_err(|error| error.to_string())
        }
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(win: bool, revealed: bool, hinted: bool, found: u32, total: u32) -> Verdict {
        let wrong = if win || revealed { 0 } else { 1 };
        Verdict {
            win,
            revealed,
            hinted,
            found,
            wrong,
            total,
        }
    }

    #[test]
    fn classifies_partial_credit() {
        assert_eq!(
            classify(&verdict(true, false, false, 3, 3)),
            Outcome::Solved
        );
        assert_eq!(classify(&verdict(true, false, true, 3, 3)), Outcome::Hinted);
        assert_eq!(
            classify(&verdict(false, false, false, 2, 5)),
            Outcome::Partial
        );
        assert_eq!(
            classify(&verdict(false, false, true, 1, 5)),
            Outcome::Partial
        );
        assert_eq!(
            classify(&verdict(false, false, false, 0, 5)),
            Outcome::Missed
        );
        assert_eq!(
            classify(&verdict(false, true, false, 2, 5)),
            Outcome::Revealed
        );
        assert_eq!(
            classify(&verdict(false, true, false, 0, 1)),
            Outcome::Revealed
        );
    }

    #[test]
    fn scores_every_move_by_its_first_try() {
        // Wrong on the first move, right on the second, wrong on the third.
        let mixed = Verdict {
            found: 1,
            wrong: 2,
            total: 3,
            ..Verdict::default()
        };
        assert_eq!(classify(&mixed), Outcome::Partial);
        assert_eq!(moves(classify(&mixed), &mixed), (1, 3));
        let all_wrong = Verdict {
            wrong: 3,
            total: 3,
            ..Verdict::default()
        };
        assert_eq!(classify(&all_wrong), Outcome::Missed);
        assert_eq!(moves(classify(&all_wrong), &all_wrong), (0, 3));
        // A mistake then the solution shown: the moves shown were never tried.
        let shown = Verdict {
            revealed: true,
            wrong: 1,
            total: 3,
            ..Verdict::default()
        };
        assert_eq!(classify(&shown), Outcome::Missed);
        assert_eq!(moves(classify(&shown), &shown), (0, 1));
    }

    #[test]
    fn counts_moves_found_and_tried() {
        let partial = verdict(false, false, false, 3, 5);
        assert_eq!(moves(classify(&partial), &partial), (3, 4));
        let missed = verdict(false, false, false, 0, 2);
        assert_eq!(moves(classify(&missed), &missed), (0, 1));
        let revealed = verdict(false, true, false, 1, 3);
        assert_eq!(moves(classify(&revealed), &revealed), (1, 1));
        // A mate other than the solution's can end a puzzle early: it is still every move.
        let solved = verdict(true, false, false, 2, 3);
        assert_eq!(moves(classify(&solved), &solved), (3, 3));
    }

    #[test]
    fn reads_calendar_days() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2024-02-29"), Some(19_782));
        assert_eq!(day_number("2024-03-01"), Some(19_783));
        assert_eq!(day_number("2023-02-29"), None);
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("2026-1-01"), None);
        assert_eq!(day_number("２０２６-01-01"), None);
    }

    #[test]
    fn streak_spans_month_and_year_boundaries() {
        let days = ["2025-12-30", "2025-12-31", "2026-01-01", "2026-01-01"];
        assert_eq!(
            day_streak(days, "2026-01-01"),
            DayStreak {
                current: 3,
                best: 3
            }
        );
        let leap = ["2024-02-28", "2024-02-29", "2024-03-01"];
        assert_eq!(day_streak(leap, "2024-03-01").current, 3);
    }

    #[test]
    fn streak_survives_until_today_ends_and_breaks_on_a_missed_day() {
        let days = ["2026-10-16", "2026-10-17", "2026-10-18"];
        // Nothing solved yet today: yesterday's run still stands.
        assert_eq!(day_streak(days, "2026-10-19").current, 3);
        // A whole day without a solve ends it; the best run is kept.
        assert_eq!(
            day_streak(days, "2026-10-20"),
            DayStreak {
                current: 0,
                best: 3
            }
        );
        let gap = [
            "2026-10-14",
            "2026-10-15",
            "2026-10-17",
            "2026-10-18",
            "2026-10-19",
        ];
        assert_eq!(
            day_streak(gap, "2026-10-19"),
            DayStreak {
                current: 3,
                best: 3
            }
        );
        assert_eq!(day_streak([], "2026-10-19"), DayStreak::default());
        assert_eq!(day_streak(["junk"], "2026-10-19"), DayStreak::default());
    }
}
