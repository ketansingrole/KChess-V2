//! Runs: the local scores of timed training (Puzzle Rush, drills, coordinates, …).

use super::*;

wire_enum!(#[doc = "Everything the runs table (local scores) records."] RunKind, RUN_KINDS {
    Storm = "storm",
    Streak = "streak",
    Rush = "rush",
    Coordinates = "coordinates",
    SquareColor = "squareColor",
    KnightPath = "knightPath",
    Endgame = "endgame",
    Lesson = "lesson",
});

/// A drill missed at least once (a run of its kind that scored nothing), and when it comes
/// back: its spaced-repetition card, from its runs in order.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RunCard {
    pub variant: String,
    pub card: kchess_domain::training::repetition::Card,
}

/// A run's score and details, as the game saves it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunInput {
    pub kind: RunKind,
    /// Which flavour of the kind (`3min`, `find`, `kq-vs-k`, …).
    pub variant: String,
    #[serde(with = "js_number")]
    pub score: f64,
    /// Extra numbers, words and flags shown with the score (accuracy, combo, …), in order.
    pub detail: serde_json::Map<String, Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: i64,
    pub kind: RunKind,
    pub variant: String,
    #[serde(with = "js_number")]
    pub score: f64,
    pub detail: serde_json::Map<String, Value>,
    pub played_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunBest {
    #[serde(with = "js_number")]
    pub score: f64,
    pub played_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub kind: RunKind,
    /// Best score per variant.
    pub best: Keyed<RunBest>,
    /// Newest first.
    pub recent: Vec<RunRecord>,
    /// All runs of this kind.
    pub total: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunSaved {
    pub summary: RunSummary,
    /// The run beat every earlier run of its variant.
    pub is_best: bool,
}
