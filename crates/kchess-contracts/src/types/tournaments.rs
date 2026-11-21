//! Lichess tournaments: lists, standings, an arena's detail and the ones joined.

use super::*;

union!(#[doc = "Lifecycle stage of a tournament."] TournamentStatus { Created = "created", Started = "started", Finished = "finished" });

/// A tournament's clock: seconds and seconds per move.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentClock {
    pub limit: Number,
    pub increment: Number,
}

/// The team a Swiss event belongs to.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentTeam {
    pub id: String,
    pub name: String,
}

/// An arena or Swiss tournament in a list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentSummary {
    pub id: String,
    pub system: TournamentSystem,
    pub name: String,
    pub status: TournamentStatus,
    pub variant: String,
    pub variant_name: String,
    pub rated: bool,
    pub clock: TournamentClock,
    /// Length of an arena in minutes; Swiss events have rounds instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nb_rounds: Option<Number>,
    pub nb_players: Number,
    pub starts_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finishes_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TournamentTeam>,
    /// The Board API can play its games and KChess knows its variant.
    pub playable: bool,
    /// Why it cannot be played here, when it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perf: Option<String>,
    /// How often Lichess runs it (`hourly`, `eastern`, `weekly`…); absent for player-made events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq: Option<String>,
    /// Only players rated at most this may join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rating: Option<Number>,
}

/// The tournaments of the timeline, and notes about what could not be read.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentList {
    pub arenas: Vec<TournamentSummary>,
    pub swiss: Vec<TournamentSummary>,
    /// Teams of the account whose Swiss events could not be read, and other notes.
    pub problems: Vec<String>,
}

/// One row of a leaderboard.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentStanding {
    pub rank: Number,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Number>,
    /// Arena score sheet: one digit per game, newest last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire: Option<bool>,
}

/// A player of an arena's featured game or of a game in progress.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentPlayer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<Number>,
}

/// A game between two tournament players, with their current ranks.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentGame {
    pub id: String,
    pub white: TournamentPlayer,
    pub black: TournamentPlayer,
}

/// Seconds left on each clock.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentClocks {
    pub white: Number,
    pub black: Number,
}

/// The top game being played now in an arena.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentFeatured {
    pub id: String,
    pub fen: String,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub white: TournamentPlayer,
    pub black: TournamentPlayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clocks: Option<TournamentClocks>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentPodium {
    pub name: String,
    pub rank: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<Number>,
}

/// An arena's totals once it has finished.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentStats {
    pub games: Number,
    pub white_wins: Number,
    pub black_wins: Number,
    pub draws: Number,
    pub berserks: Number,
    pub average_rating: Number,
}

/// What an arena's detail adds: the games being played and, once it has finished, its results.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentExtras {
    /// The top game being played now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub featured: Option<TournamentFeatured>,
    /// Games being played now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duels: Option<Vec<TournamentGame>>,
    /// The top three once it has finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub podium: Option<Vec<TournamentPodium>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<TournamentStats>,
}

/// The account's entry in a tournament.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentMe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<Number>,
    pub withdraw: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

/// One entry condition and whether the account meets it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentCondition {
    pub condition: String,
    pub verdict: String,
}

/// Entry conditions and whether the account meets them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TournamentVerdicts {
    pub accepted: bool,
    pub list: Vec<TournamentCondition>,
}

/// One tournament, with the leaderboard, the games being played and (when finished) its results.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentDetail {
    #[serde(flatten)]
    pub summary: TournamentSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_start: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_finish: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub berserkable: Option<bool>,
    pub standing: Vec<TournamentStanding>,
    /// The leaderboard page shown (10 players each).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing_page: Option<Number>,
    /// You, when the account you asked as has joined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<TournamentMe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdicts: Option<TournamentVerdicts>,
    /// Swiss: when the next round starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_round_in: Option<Number>,
    #[serde(flatten)]
    pub extras: TournamentExtras,
}

/// An arena to create on Lichess.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewArena {
    /// Empty for a random Grandmaster name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Minutes on each clock.
    pub clock_time: f64,
    pub clock_increment: f64,
    pub minutes: f64,
    /// Minutes from now, unless `startDate` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_minutes: Option<f64>,
    /// Milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<i64>,
    pub variant: Variant,
    pub rated: bool,
    /// An entry code players must type to join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A tournament joined from KChess, kept until it ends so its account's event stream stays open.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JoinedTournament {
    pub system: TournamentSystem,
    pub id: String,
    /// The account that joined it.
    pub account: String,
    pub name: String,
    /// When it ends (or a day after joining a Swiss), in milliseconds since the epoch.
    #[serde(with = "js_number")]
    pub until: f64,
}

impl JoinedTournament {
    /// Joined by `account`, compared without case.
    pub fn joined_by(&self, account: &str) -> bool {
        self.account.eq_ignore_ascii_case(account)
    }
}
