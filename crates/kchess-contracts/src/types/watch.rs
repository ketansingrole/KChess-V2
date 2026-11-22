//! Watching games: Lichess TV, a game followed move by move and broadcasts.

use super::*;

union!(#[doc = "Connection state while watching a game."] WatchPhase { Connecting = "connecting", Connected = "connected", Ended = "ended", Error = "error" });

union!(#[doc = "Whether watching Lichess TV or a specific game."] WatchSource { Tv = "tv", Game = "game" });

union!(#[doc = "Time section of a broadcast list."] BroadcastSection { Active = "active", Upcoming = "upcoming", Past = "past" });

/// Connection health of the TV/game feed, scoped to its watch session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchState {
    pub session: u64,
    pub phase: WatchPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The kind of failure of an `error` phase, so the viewer is offered a retry only when one
    /// can help.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::ErrorKind>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchClock {
    pub initial: f64,
    pub increment: f64,
}

/// A game a TV channel showed before the current one, with its result once Lichess has it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TvPastGame {
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    /// The last position the feed sent.
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub winner: Option<Color>,
}

/// One position of a game being watched (TV or a game by id).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchFrame {
    /// Which `watch` call it belongs to; frames of an earlier one are stale.
    pub session: u64,
    pub source: WatchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
    /// Board placement plus side to move (pockets of Crazyhouse are dropped).
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    /// Clocks in seconds when the frame was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white_clock: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black_clock: Option<f64>,
    pub variant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    /// Rated on Lichess; unknown until the game's details arrive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
    /// Time control in seconds; absent for correspondence or until the details arrive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<WatchClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub winner: Option<Color>,
    pub finished: bool,
    /// FEN before the first move, once the move list is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_fen: Option<String>,
    /// UCI moves from `startFen`; absent until Lichess's delayed export of a TV game arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
    /// Plies played after `moves` that Lichess has not named yet: the list stops this far short
    /// of the board until a later move names them. Absent when the list reaches the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves_behind: Option<u32>,
    /// Why a TV game's moves are missing when they will not simply arrive with the next moves:
    /// Lichess refused its export, or KChess does not play its variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves_error: Option<String>,
    /// Games this channel showed earlier in this session, newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Vec<TvPastGame>>,
}

/// One board of a broadcast round, as the live PGN feed last described it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastGame {
    /// The study chapter id.
    pub id: String,
    pub name: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    pub result: String,
    pub start_fen: String,
    /// UCI moves of the main line.
    pub moves: Vec<String>,
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    /// Clocks in seconds, from the last comments of each side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white_clock: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black_clock: Option<f64>,
    pub ongoing: bool,
    /// The game as PGN, for the analysis board.
    pub pgn: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastUpdate {
    pub session: u64,
    pub round_id: String,
    /// Games that changed (all of them at first).
    pub games: Vec<BroadcastGame>,
    /// The feed ended (the round is over or the connection dropped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A game to watch: a TV channel (`{ channel }`) or any game by its id (`{ gameId }`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum WatchTarget {
    #[serde(rename = "channel")]
    Channel(String),
    #[serde(rename = "gameId")]
    Game(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchPlayer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
}

/// A Lichess TV channel and who is on it now.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TvChannel {
    pub key: String,
    pub label: String,
    /// Sent as `game_id`, unlike every other key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<WatchPlayer>,
}

/// A broadcast in a listing, with its current round.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<String>,
    pub tour_id: String,
    pub tour_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_name: Option<String>,
    pub ongoing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<i64>,
    pub section: BroadcastSection,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRoundRef {
    pub id: String,
    pub name: String,
    pub ongoing: bool,
    pub finished: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<i64>,
}

/// A broadcast tournament with its rounds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastTourDetail {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub rounds: Vec<BroadcastRoundRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_round_id: Option<String>,
}

/// The tournament a broadcast round is in, as a round's link names only the round.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRoundPlace {
    pub tour_id: String,
    pub round_id: String,
}
