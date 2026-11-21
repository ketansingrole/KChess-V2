//! Insights: the report over a player's synced games.

use super::*;

/// The insights filter: one account's finished games.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InsightsQuery {
    pub account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<ExplorerSpeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
    /// Only games from the last this-many days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u64>,
    /// Standard chess when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Streaks {
    pub longest_win: i64,
    pub longest_loss: i64,
    /// Positive for wins in a row, negative for losses.
    pub current: i64,
}

/// One account's finished games as insights: results overall and broken down, streaks, and
/// accuracy and time use where reviews and clocks allow.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightsReport {
    pub account: String,
    pub total: i64,
    pub record: GameRecord,
    pub by_color: ByColor,
    /// Most games first.
    pub by_speed: Vec<SpeedRecord>,
    /// Opening families with two games or more, most games first.
    pub by_opening: Vec<OpeningRecord>,
    /// Sunday first, in local time.
    pub by_weekday: Vec<GameRecord>,
    /// Hours 0 to 23, in local time.
    pub by_hour: Vec<GameRecord>,
    /// By the opponent's rating against the player's.
    pub by_opponent: Vec<BandRecord>,
    /// By the game's length in moves.
    pub by_length: Vec<BandRecord>,
    /// By how the game ended, most games first.
    pub endings: Vec<EndingRecord>,
    pub streaks: Streaks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<AccuracyReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_use: Option<TimeUseReport>,
    /// The variants other than standard the account has finished games in.
    pub variants: Vec<Variant>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ByColor {
    pub white: GameRecord,
    pub black: GameRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SpeedRecord {
    pub speed: String,
    pub record: GameRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpeningRecord {
    pub name: String,
    pub record: GameRecord,
    /// Of those games, the ones played as White.
    pub as_white: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BandRecord {
    pub label: String,
    pub record: GameRecord,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EndingRecord {
    /// The game's Lichess status (`mate`, `resign`, ...).
    pub status: String,
    pub record: GameRecord,
}

/// The reviewed games' accuracy and mistakes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccuracyReport {
    pub games: i64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub average: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub acpl: Option<f64>,
    pub per_game: MistakeRates,
}

/// Inaccuracies, mistakes and blunders per reviewed game.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct MistakeRates {
    pub inaccuracy: f64,
    pub mistake: f64,
    pub blunder: f64,
}

/// How the clock was used in the clocked games.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimeUseReport {
    pub games: i64,
    /// Results when the player's clock fell under a tenth of its start.
    pub trouble: GameRecord,
    /// Results of the rest.
    pub spare: GameRecord,
    /// The phases the player moved in, in game order.
    pub phases: Vec<PhaseTime>,
    pub clock_at: Vec<ClockAt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blunders: Option<BlundersByTime>,
}

/// The player's average time per move in one phase of the game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PhaseTime {
    pub phase: kchess_domain::time_use::Phase,
    pub moves: i64,
    #[serde(with = "js_number")]
    pub seconds: f64,
    /// Percent of all the time the player spent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub share: Option<f64>,
}

/// The clocks at a move, as percentages of the start, averaged over the games that reached it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClockAt {
    #[serde(rename = "move")]
    pub at_move: i64,
    pub games: i64,
    #[serde(with = "js_number")]
    pub player: f64,
    #[serde(with = "js_number")]
    pub opponent: f64,
}

/// Blunders when short of time against with time to spare, in the reviewed clocked games.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlundersByTime {
    pub games: i64,
    pub short: Blunders,
    pub spare: Blunders,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Blunders {
    pub moves: i64,
    pub blunders: i64,
}
