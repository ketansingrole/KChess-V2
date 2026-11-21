//! Puzzles: Lichess's, the downloaded database, attempts and progress.

use super::*;

wire_enum!(#[doc = "How far from your Lichess puzzle rating the next puzzle is aimed."] PuzzleDifficulty, PUZZLE_DIFFICULTIES {
    Easiest = "easiest",
    Easier = "easier",
    Normal = "normal",
    Harder = "harder",
    Hardest = "hardest",
});

wire_enum!(#[doc = "Where a training puzzle came from: Lichess unrated, the downloaded database, or Lichess rated."] PuzzleTrainMode, PUZZLE_TRAIN_MODES {
    Practice = "practice",
    Offline = "offline",
    Rated = "rated",
});

union!(#[doc = "What a training attempt counts as, each move scored by its first try: every move found (`solved`, or `hinted`"] #[doc = "after a hint), `partial` (some found, some wrong), `missed` (none found) or `revealed` (the solution shown"] #[doc = "before any wrong move). Local only: Lichess results stay a win or a loss."] PuzzleAttemptOutcome {
    Solved = "solved",
    Hinted = "hinted",
    Partial = "partial",
    Missed = "missed",
    Revealed = "revealed",
});

union!(#[doc = "Progress stage of the puzzle database download."] PuzzleDbPhase {
    Downloading = "downloading",
    Importing = "importing",
    Done = "done",
    Cancelled = "cancelled",
    Failed = "failed",
});

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleGlicko {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub rating: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub deviation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleRequest {
    /// Connected account to train as; empty trains anonymously (no rating, nothing recorded).
    pub account: String,
    /// A puzzle theme key (`mateIn2`, `fork`, …) or `mix`.
    pub angle: String,
    pub difficulty: PuzzleDifficulty,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub color: Option<Color>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDraw {
    /// Absent when Lichess has no puzzle left for this theme and difficulty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puzzle: Option<Puzzle>,
    /// The account's current puzzle rating, when Lichess reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glicko: Option<PuzzleGlicko>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleSolveRequest {
    pub account: String,
    pub angle: String,
    pub id: String,
    pub win: bool,
    /// Rated results change the Lichess puzzle rating; unrated ones are only marked as seen.
    pub rated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleSolveResult {
    /// Rating change Lichess applied; absent for unrated results, null when Lichess sent none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_or_null"
    )]
    pub rating_diff: Option<Option<i64>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleActivityEntry {
    /// When it was played, in ms since the epoch; null when Lichess did not say.
    pub date: Option<i64>,
    pub win: bool,
    pub puzzle: Puzzle,
}

/// A move the player tried first at one of their moves of a puzzle, instead of the solution's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleWrongMove {
    /// The index in the solution of the move it was played instead of.
    pub index: u32,
    pub uci: String,
}

/// A training attempt as the board saw it at its first verdict, for the local log (`puzzles.db`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleAttemptInput {
    pub puzzle: Puzzle,
    pub mode: PuzzleTrainMode,
    /// Tried again from the history: kept in the log, left out of totals and the day streak.
    pub retry: bool,
    pub win: bool,
    /// The solution was shown.
    pub revealed: bool,
    /// A hint was asked for before the verdict.
    pub hinted: bool,
    /// Player moves right at the first try, and wrong at the first try, of `total` in the solution.
    pub found: u32,
    pub wrong: u32,
    pub total: u32,
    /// The local calendar day (`YYYY-MM-DD`) it was played on.
    pub day: String,
    /// What was played at each wrong first try.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrong_moves: Option<Vec<PuzzleWrongMove>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleAttempt {
    pub id: i64,
    /// When it was played, in ms since the epoch.
    pub at: i64,
    pub day: String,
    pub puzzle: Puzzle,
    pub mode: PuzzleTrainMode,
    pub retry: bool,
    pub outcome: PuzzleAttemptOutcome,
    pub hinted: bool,
    /// Player moves right at the first try (all of them for a solve), of `total`.
    pub found: u32,
    /// Player moves wrong at the first try.
    pub wrong: u32,
    pub total: u32,
    /// What was played at each wrong first try, when the log kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrong_moves: Option<Vec<PuzzleWrongMove>>,
}

/// Attempts by outcome, and player moves found of those tried (found plus each ending wrong move).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleTotals {
    pub attempts: i64,
    pub solved: i64,
    pub hinted: i64,
    pub partial: i64,
    pub missed: i64,
    pub revealed: i64,
    pub moves_found: i64,
    pub moves_tried: i64,
}

/// The local attempt log, summarized. Retries are listed but not counted.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleProgress {
    /// Newest first.
    pub recent: Vec<PuzzleAttempt>,
    /// Consecutive local days with a solved puzzle, ending today (or yesterday until today has one).
    pub day_streak: u32,
    pub best_day_streak: u32,
    pub today: PuzzleTotals,
    pub all: PuzzleTotals,
}

/// Which local day is today, and how many recent attempts to list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleProgressQuery {
    pub today: String,
    pub limit: u32,
}

/// A puzzle-database download or import in progress (or finished), streamed to the window.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDbProgress {
    pub phase: PuzzleDbPhase,
    /// Compressed bytes received so far.
    pub received: u64,
    /// Puzzles kept so far.
    pub kept: u64,
    /// Total compressed bytes, when the server said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PuzzleDbPhase {
    /// The last report of a download: none follows it.
    pub fn is_terminal(self) -> bool {
        match self {
            PuzzleDbPhase::Done | PuzzleDbPhase::Cancelled | PuzzleDbPhase::Failed => true,
            PuzzleDbPhase::Downloading | PuzzleDbPhase::Importing => false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDbStatus {
    pub installed: bool,
    pub count: i64,
    /// Approximate size on disk.
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<i64>,
    /// True while a download runs; its progress arrives as events.
    pub busy: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalPuzzleQuery {
    /// A theme or, as Lichess's puzzle angles allow, an opening key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_rating: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rating: Option<u32>,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLadderQuery {
    pub from: u32,
    pub to: u32,
    pub count: u32,
}
