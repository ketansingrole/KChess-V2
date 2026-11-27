//! Looking up a position: the opening explorer, the tablebase, cloud evaluations and an opening's description and popularity.

use super::*;

wire_enum!(#[doc = "`opening` is the Lichess games database, `masters` over-the-board master games, `player` one player's games, `database` a game database on this computer (`gameDbImport`)."] PositionLookupKind, POSITION_LOOKUP_KINDS {
    Opening = "opening",
    Masters = "masters",
    Player = "player",
    Tablebase = "tablebase",
    Database = "database",
});

/// Time control of an opening-explorer game pool.
pub use kchess_domain::perf::{SPEEDS, Speed};

union!(#[doc = "Whether play is rated or casual."] LookupMode { Rated = "rated", Casual = "casual" });

/// Filters of the opening explorer; each database uses the ones it understands.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LookupOptions {
    /// Lichess and player databases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speeds: Option<Vec<Speed>>,
    /// Lichess database: rating groups (each is the lower bound of its band).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratings: Option<Vec<u32>>,
    /// Player database: whose games (a Lichess username), and with which colour. A game
    /// database: a name its players' names match word by word, and the side they had.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    #[serde(
        default,
        with = "kchess_domain::color::option_by_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<Color>,
    /// Player database: rated, casual or both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<LookupMode>>,
    /// Games since this month (`YYYY-MM`; masters and game databases use a year).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// A game database: games up to this year (`YYYY`, or `YYYY-MM` read as its year).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// A game database: which one (`gameDbStatus`'s ids). Required for its lookups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// A game database: games whose players are both rated at least this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_elo: Option<u32>,
    /// The Lichess and player databases' variant; standard chess when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
}

/// A game the explorer lists as notable for the position.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerGame {
    pub id: String,
    pub white: String,
    pub black: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white_rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black_rating: Option<f64>,
    #[serde(
        default,
        with = "kchess_domain::color::option_by_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub winner: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month: Option<String>,
    /// The move played from this position, as SAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub san: Option<String>,
}

/// One move of a lookup: the database's counts and the SAN checked against the position.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LookupMove {
    pub uci: String,
    pub san: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draws: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<i64>,
    /// Tablebase: the move's outcome (`win`, `syzygy-loss`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Tablebase: absent for a database move, null when the tablebase gives no distance.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_or_null"
    )]
    pub dtz: Option<Option<i64>>,
}

/// A lookup as the profile keeps it: what `PositionLookup` answers, but where it came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedLookup {
    pub kind: PositionLookupKind,
    pub fen: String,
    pub fetched_at: i64,
    /// Games in the database reaching this position (opening databases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    /// Tablebase: the position's outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_or_null"
    )]
    pub dtz: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<Vec<ExplorerGame>>,
    pub moves: Vec<LookupMove>,
}

/// A position lookup: the saved one while it is fresh, or `stale` (with a `message`) when a
/// refresh failed and the saved one is shown instead.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PositionLookup {
    #[serde(flatten)]
    pub saved: SavedLookup,
    pub cached: bool,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A cloud evaluation: lines from White's point of view, like the local engine's. Lichess's
/// figures are whole numbers.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudEval {
    pub fen: String,
    pub depth: u64,
    pub knodes: u64,
    pub lines: Vec<CloudLine>,
}

/// One line of a cloud evaluation, at the evaluation's depth.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudLine {
    /// 1 for the best line, 2 for the next best…
    pub rank: u32,
    pub depth: u64,
    /// Centipawns, when there is no forced mate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<i64>,
    /// Moves to mate: positive when White mates, negative when Black does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<i64>,
    /// Principal variation as UCI moves.
    pub pv: Vec<String>,
}

/// A paragraph, or a section heading, of an opening description.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionBlock {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
    pub text: String,
}

/// A line's Wikibooks “Chess Opening Theory” page as plain text (CC BY-SA 4.0): shown with its
/// title, a link to `url` and the licence.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpeningDescription {
    pub title: String,
    pub url: String,
    pub blocks: Vec<DescriptionBlock>,
    pub fetched_at: i64,
}

/// One month of a line's popularity: of the Lichess games at the position before its last move,
/// the share (0–1) that went on with it, and how many games that was.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PopularityMonth {
    /// `YYYY-MM`.
    pub month: String,
    pub share: f64,
    pub games: i64,
}
