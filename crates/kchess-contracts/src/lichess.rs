//! `crates/kchess-contracts/ts/lichessApi.ts`: the Lichess API shapes KChess reads, copied from
//! `@lichess-org/types` 2.0.176. Only the schemas the contracts and services reference are kept.
//!
//! The TypeScript exposes them as `components['schemas'][...]` and `paths[...]`. Here each
//! schema is a named `Api*` type (so none collides with a KChess name in `types.ts`), and
//! `ApiSchemas` / `ApiPaths` give the same indexed structure under the names `components` and
//! `paths`. Shared literal unions (`PlayerColor`, `PerfType`, `ChallengeColor`, `ChatRoom`,
//! `ChallengeDirection`) are KChess's own enums with the same wire values.

use serde::{Deserialize, Serialize};
use ts_rs::{Config, TS};

use crate::generate::Item;
use crate::types::{ChallengeColor, ChallengeDirection, ChatRoom, PerfType, PlayerColor};

/* ── Literal unions ─────────────────────────────────────────────────── */

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiTitle {
    #[serde(rename = "GM")]
    Gm,
    #[serde(rename = "WGM")]
    Wgm,
    #[serde(rename = "IM")]
    Im,
    #[serde(rename = "WIM")]
    Wim,
    #[serde(rename = "FM")]
    Fm,
    #[serde(rename = "WFM")]
    Wfm,
    #[serde(rename = "NM")]
    Nm,
    #[serde(rename = "CM")]
    Cm,
    #[serde(rename = "WCM")]
    Wcm,
    #[serde(rename = "WNM")]
    Wnm,
    #[serde(rename = "LM")]
    Lm,
    #[serde(rename = "BOT")]
    Bot,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiSpeed {
    #[serde(rename = "ultraBullet")]
    UltraBullet,
    #[serde(rename = "bullet")]
    Bullet,
    #[serde(rename = "blitz")]
    Blitz,
    #[serde(rename = "rapid")]
    Rapid,
    #[serde(rename = "classical")]
    Classical,
    #[serde(rename = "correspondence")]
    Correspondence,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiVariantKey {
    #[serde(rename = "standard")]
    Standard,
    #[serde(rename = "chess960")]
    Chess960,
    #[serde(rename = "crazyhouse")]
    Crazyhouse,
    #[serde(rename = "antichess")]
    Antichess,
    #[serde(rename = "atomic")]
    Atomic,
    #[serde(rename = "horde")]
    Horde,
    #[serde(rename = "kingOfTheHill")]
    KingOfTheHill,
    #[serde(rename = "racingKings")]
    RacingKings,
    #[serde(rename = "threeCheck")]
    ThreeCheck,
    #[serde(rename = "fromPosition")]
    FromPosition,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiGameStatusName {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "started")]
    Started,
    #[serde(rename = "aborted")]
    Aborted,
    #[serde(rename = "mate")]
    Mate,
    #[serde(rename = "resign")]
    Resign,
    #[serde(rename = "stalemate")]
    Stalemate,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "draw")]
    Draw,
    #[serde(rename = "outoftime")]
    Outoftime,
    #[serde(rename = "cheat")]
    Cheat,
    #[serde(rename = "noStart")]
    NoStart,
    #[serde(rename = "unknownFinish")]
    UnknownFinish,
    #[serde(rename = "insufficientMaterialClaim")]
    InsufficientMaterialClaim,
    #[serde(rename = "variantEnd")]
    VariantEnd,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiGameSource {
    #[serde(rename = "lobby")]
    Lobby,
    #[serde(rename = "friend")]
    Friend,
    #[serde(rename = "ai")]
    Ai,
    #[serde(rename = "api")]
    Api,
    #[serde(rename = "tournament")]
    Tournament,
    #[serde(rename = "position")]
    Position,
    #[serde(rename = "import")]
    Import,
    #[serde(rename = "importlive")]
    Importlive,
    #[serde(rename = "simul")]
    Simul,
    #[serde(rename = "relay")]
    Relay,
    #[serde(rename = "pool")]
    Pool,
    #[serde(rename = "arena")]
    Arena,
    #[serde(rename = "swiss")]
    Swiss,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiChallengeStatus {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "canceled")]
    Canceled,
    #[serde(rename = "declined")]
    Declined,
    #[serde(rename = "accepted")]
    Accepted,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiDeclineReasonKey {
    #[serde(rename = "generic")]
    Generic,
    #[serde(rename = "later")]
    Later,
    #[serde(rename = "toofast")]
    Toofast,
    #[serde(rename = "tooslow")]
    Tooslow,
    #[serde(rename = "timecontrol")]
    Timecontrol,
    #[serde(rename = "rated")]
    Rated,
    #[serde(rename = "casual")]
    Casual,
    #[serde(rename = "standard")]
    Standard,
    #[serde(rename = "variant")]
    Variant,
    #[serde(rename = "nobot")]
    Nobot,
    #[serde(rename = "onlybot")]
    Onlybot,
}

/* ── Users ──────────────────────────────────────────────────────────── */

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiPerf {
    pub games: f64,
    pub rating: f64,
    /// rating deviation
    pub rd: f64,
    pub prog: f64,
    pub prov: Option<bool>,
    pub rank: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiPuzzleModePerf {
    pub runs: f64,
    pub score: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiPerfs {
    pub chess960: Option<ApiPerf>,
    pub atomic: Option<ApiPerf>,
    pub racing_kings: Option<ApiPerf>,
    pub ultra_bullet: Option<ApiPerf>,
    pub blitz: Option<ApiPerf>,
    pub king_of_the_hill: Option<ApiPerf>,
    pub three_check: Option<ApiPerf>,
    pub antichess: Option<ApiPerf>,
    pub crazyhouse: Option<ApiPerf>,
    pub bullet: Option<ApiPerf>,
    pub correspondence: Option<ApiPerf>,
    pub horde: Option<ApiPerf>,
    pub puzzle: Option<ApiPerf>,
    pub classical: Option<ApiPerf>,
    pub rapid: Option<ApiPerf>,
    pub storm: Option<ApiPuzzleModePerf>,
    pub racer: Option<ApiPuzzleModePerf>,
    pub streak: Option<ApiPuzzleModePerf>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiProfile {
    pub flag: Option<String>,
    pub location: Option<String>,
    pub bio: Option<String>,
    pub real_name: Option<String>,
    pub fide_rating: Option<f64>,
    pub uscf_rating: Option<f64>,
    pub ecf_rating: Option<f64>,
    pub cfc_rating: Option<f64>,
    pub rcf_rating: Option<f64>,
    pub dsb_rating: Option<f64>,
    pub links: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiPlayTime {
    pub total: f64,
    pub tv: f64,
    pub human: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiUser {
    pub id: String,
    pub username: String,
    pub perfs: Option<ApiPerfs>,
    pub title: Option<ApiTitle>,
    pub flair: Option<String>,
    pub created_at: Option<f64>,
    /// only appears if a user's account is closed
    pub disabled: Option<bool>,
    /// only appears if a user's account is marked for the violation of Lichess TOS
    pub tos_violation: Option<bool>,
    pub profile: Option<ApiProfile>,
    pub seen_at: Option<f64>,
    pub play_time: Option<ApiPlayTime>,
    /// deprecated: use patronColor
    pub patron: Option<bool>,
    pub patron_color: Option<f64>,
    pub verified: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiCount {
    pub all: f64,
    pub rated: f64,
    pub ai: Option<f64>,
    pub draw: f64,
    pub draw_h: Option<f64>,
    pub loss: f64,
    pub loss_h: Option<f64>,
    pub win: f64,
    pub win_h: Option<f64>,
    pub bookmark: f64,
    pub playing: f64,
    #[serde(rename = "import")]
    pub import_: f64,
    pub me: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiStreamChannel {
    pub channel: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiUserStreamer {
    pub twitch: Option<ApiStreamChannel>,
    pub youtube: Option<ApiStreamChannel>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiUserExtended {
    #[serde(flatten)]
    #[ts(flatten)]
    pub user: ApiUser,
    pub url: String,
    pub playing: Option<String>,
    pub count: Option<ApiCount>,
    pub streaming: Option<bool>,
    pub streamer: Option<ApiUserStreamer>,
    /// only appears if the request is authenticated with OAuth2
    pub followable: Option<bool>,
    /// only appears if the request is authenticated with OAuth2
    pub following: Option<bool>,
    /// only appears if the request is authenticated with OAuth2
    pub blocking: Option<bool>,
    pub fide_id: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiRatingHistoryEntry {
    #[ts(type = "'puzzle' | PerfType", optional)]
    pub name: Option<String>,
    pub points: Option<Vec<Vec<f64>>>,
}

/* ── Games ──────────────────────────────────────────────────────────── */

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiVariant {
    pub key: ApiVariantKey,
    pub name: String,
    pub short: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiGameStatus {
    #[ts(type = "10 | 20 | 25 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 60")]
    pub id: f64,
    pub name: ApiGameStatusName,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ApiGameEventOpponent {
    #[serde(rename_all = "camelCase")]
    Human {
        id: String,
        username: String,
        rating: f64,
        #[ts(optional)]
        rating_diff: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Ai {
        id: (),
        username: String,
        /// AI level, from 1 to 8, where 1 is the weakest and 8 is the strongest.
        ai: f64,
    },
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameCompat {
    /// Compatible with Bot API
    pub bot: Option<bool>,
    /// Compatible with Board API
    pub board: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameEventInfo {
    pub full_id: String,
    pub game_id: String,
    pub fen: Option<String>,
    pub color: Option<PlayerColor>,
    pub last_move: Option<String>,
    pub source: Option<ApiGameSource>,
    pub status: Option<ApiGameStatus>,
    pub variant: Option<ApiVariant>,
    pub speed: Option<ApiSpeed>,
    pub perf: Option<String>,
    pub rating: Option<f64>,
    pub rated: Option<bool>,
    pub has_moved: Option<bool>,
    pub opponent: Option<ApiGameEventOpponent>,
    pub is_my_turn: Option<bool>,
    pub seconds_left: Option<f64>,
    pub winner: Option<PlayerColor>,
    pub rating_diff: Option<f64>,
    pub compat: Option<ApiGameCompat>,
    pub id: Option<String>,
    pub tournament_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiGameStartEvent {
    #[serde(rename = "type")]
    #[ts(type = "'gameStart'")]
    pub kind: String,
    pub game: ApiGameEventInfo,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiGameFinishEvent {
    #[serde(rename = "type")]
    #[ts(type = "'gameFinish'")]
    pub kind: String,
    pub game: ApiGameEventInfo,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiChallengeUser {
    pub id: String,
    pub name: String,
    pub rating: Option<f64>,
    pub title: Option<ApiTitle>,
    pub flair: Option<String>,
    pub patron: Option<bool>,
    pub patron_color: Option<f64>,
    pub provisional: Option<bool>,
    pub online: Option<bool>,
    pub lag: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum ApiTimeControl {
    #[serde(rename = "clock")]
    Clock {
        #[ts(optional)]
        limit: Option<f64>,
        #[ts(optional)]
        increment: Option<f64>,
        #[ts(optional)]
        show: Option<String>,
    },
    #[serde(rename = "correspondence")]
    #[serde(rename_all = "camelCase")]
    Correspondence {
        #[ts(optional)]
        days_per_turn: Option<f64>,
    },
    #[serde(rename = "unlimited")]
    Unlimited,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiChallengePerf {
    pub icon: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiChallengeJson {
    pub id: String,
    pub url: String,
    pub status: ApiChallengeStatus,
    pub challenger: ApiChallengeUser,
    #[ts(optional = false)]
    pub dest_user: Option<ApiChallengeUser>,
    pub variant: ApiVariant,
    pub rated: bool,
    pub speed: ApiSpeed,
    pub time_control: ApiTimeControl,
    pub color: ChallengeColor,
    pub final_color: Option<PlayerColor>,
    pub perf: ApiChallengePerf,
    pub direction: Option<ChallengeDirection>,
    pub initial_fen: Option<String>,
    pub rematch_of: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiChallengeEvent {
    #[serde(rename = "type")]
    #[ts(type = "'challenge'")]
    pub kind: String,
    pub challenge: ApiChallengeJson,
    pub compat: Option<ApiGameCompat>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiChallengeCanceledEvent {
    #[serde(rename = "type")]
    #[ts(type = "'challengeCanceled'")]
    pub kind: String,
    pub challenge: ApiChallengeJson,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiChallengeDeclinedJson {
    #[serde(flatten)]
    #[ts(flatten)]
    pub challenge: ApiChallengeJson,
    /// Human readable, possibly translated reason why the challenge was declined.
    pub decline_reason: String,
    /// Untranslated, computer-matchable reason why the challenge was declined.
    pub decline_reason_key: ApiDeclineReasonKey,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiChallengeDeclinedEvent {
    #[serde(rename = "type")]
    #[ts(type = "'challengeDeclined'")]
    pub kind: String,
    pub challenge: ApiChallengeDeclinedJson,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameEventPlayer {
    pub ai_level: Option<f64>,
    pub id: String,
    pub name: String,
    #[ts(optional = nullable)]
    pub title: Option<ApiTitle>,
    pub rating: Option<f64>,
    pub rating_diff: Option<f64>,
    pub provisional: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiExpiration {
    /// Milliseconds since the last move was played, or since the game started
    pub idle_millis: f64,
    /// Time each player has to make their first move, before the game is aborted
    pub millis_to_move: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameStateEvent {
    #[serde(rename = "type")]
    #[ts(type = "'gameState'")]
    pub kind: String,
    /// Current moves in UCI format (King to rook for Chess690-compatible castling notation)
    pub moves: String,
    /// Integer of milliseconds White has left on the clock
    pub wtime: f64,
    /// Integer of milliseconds Black has left on the clock
    pub btime: f64,
    /// Integer of White Fisher increment.
    pub winc: f64,
    /// Integer of Black Fisher increment.
    pub binc: f64,
    pub status: ApiGameStatusName,
    /// Color of the winner, if any
    pub winner: Option<PlayerColor>,
    /// true if white is offering draw, else omitted
    pub wdraw: Option<bool>,
    /// true if black is offering draw, else omitted
    pub bdraw: Option<bool>,
    /// true if white is proposing takeback, else omitted
    pub wtakeback: Option<bool>,
    /// true if black is proposing takeback, else omitted
    pub btakeback: Option<bool>,
    /// A game may be aborted if a player doesn't make their first move in time
    pub expiration: Option<ApiExpiration>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameClock {
    /// Initial time in milliseconds
    pub initial: Option<f64>,
    /// Increment time in milliseconds
    pub increment: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiPerfName {
    /// Translated perf name (e.g. "Classical" or "Blitz")
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiGameFullEvent {
    #[serde(rename = "type")]
    #[ts(type = "'gameFull'")]
    pub kind: String,
    pub id: String,
    pub variant: ApiVariant,
    pub clock: Option<ApiGameClock>,
    pub speed: ApiSpeed,
    pub perf: ApiPerfName,
    pub rated: bool,
    pub created_at: f64,
    pub white: ApiGameEventPlayer,
    pub black: ApiGameEventPlayer,
    /// default startpos
    pub initial_fen: String,
    pub state: ApiGameStateEvent,
    /// If the game is correspondence
    pub days_per_turn: Option<f64>,
    pub tournament_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiChatLineEvent {
    #[serde(rename = "type")]
    #[ts(type = "'chatLine'")]
    pub kind: String,
    pub room: ChatRoom,
    pub username: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiOpponentGoneEvent {
    #[serde(rename = "type")]
    #[ts(type = "'opponentGone'")]
    pub kind: String,
    pub gone: bool,
    pub claim_win_in_seconds: Option<f64>,
}

/// `components['schemas']`: the schemas by their Lichess names.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiSchemas {
    #[serde(rename = "Flair")]
    pub flair: String,
    #[serde(rename = "Title")]
    pub title: ApiTitle,
    #[serde(rename = "Patron")]
    pub patron: bool,
    #[serde(rename = "PatronColor")]
    pub patron_color: f64,
    #[serde(rename = "Perf")]
    pub perf: ApiPerf,
    #[serde(rename = "PuzzleModePerf")]
    pub puzzle_mode_perf: ApiPuzzleModePerf,
    #[serde(rename = "Perfs")]
    pub perfs: ApiPerfs,
    #[serde(rename = "Profile")]
    pub profile: ApiProfile,
    #[serde(rename = "PlayTime")]
    pub play_time: ApiPlayTime,
    #[serde(rename = "User")]
    pub user: ApiUser,
    #[serde(rename = "Count")]
    pub count: ApiCount,
    #[serde(rename = "UserStreamer")]
    pub user_streamer: ApiUserStreamer,
    #[serde(rename = "UserExtended")]
    pub user_extended: ApiUserExtended,
    #[serde(rename = "PerfType")]
    pub perf_type: PerfType,
    #[serde(rename = "RatingHistoryEntry")]
    pub rating_history_entry: ApiRatingHistoryEntry,
    #[serde(rename = "RatingHistory")]
    pub rating_history: Vec<ApiRatingHistoryEntry>,
    #[serde(rename = "GameColor")]
    pub game_color: PlayerColor,
    #[serde(rename = "VariantKey")]
    pub variant_key: ApiVariantKey,
    #[serde(rename = "Speed")]
    pub speed: ApiSpeed,
    #[serde(rename = "GameStatusName")]
    pub game_status_name: ApiGameStatusName,
    #[serde(rename = "GameStatusId")]
    #[ts(type = "10 | 20 | 25 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 60")]
    pub game_status_id: f64,
    #[serde(rename = "GameSource")]
    pub game_source: ApiGameSource,
    #[serde(rename = "GameStatus")]
    pub game_status: ApiGameStatus,
    #[serde(rename = "Variant")]
    pub variant: ApiVariant,
    #[serde(rename = "GameEventOpponent")]
    pub game_event_opponent: ApiGameEventOpponent,
    #[serde(rename = "GameCompat")]
    pub game_compat: ApiGameCompat,
    #[serde(rename = "GameEventInfo")]
    pub game_event_info: ApiGameEventInfo,
    #[serde(rename = "GameStartEvent")]
    pub game_start_event: ApiGameStartEvent,
    #[serde(rename = "GameFinishEvent")]
    pub game_finish_event: ApiGameFinishEvent,
    #[serde(rename = "ChallengeStatus")]
    pub challenge_status: ApiChallengeStatus,
    #[serde(rename = "ChallengeUser")]
    pub challenge_user: ApiChallengeUser,
    #[serde(rename = "TimeControl")]
    pub time_control: ApiTimeControl,
    #[serde(rename = "ChallengeColor")]
    pub challenge_color: ChallengeColor,
    #[serde(rename = "ChallengeJson")]
    pub challenge_json: ApiChallengeJson,
    #[serde(rename = "ChallengeEvent")]
    pub challenge_event: ApiChallengeEvent,
    #[serde(rename = "ChallengeCanceledEvent")]
    pub challenge_canceled_event: ApiChallengeCanceledEvent,
    #[serde(rename = "ChallengeDeclinedJson")]
    pub challenge_declined_json: ApiChallengeDeclinedJson,
    #[serde(rename = "ChallengeDeclinedEvent")]
    pub challenge_declined_event: ApiChallengeDeclinedEvent,
    #[serde(rename = "GameEventPlayer")]
    pub game_event_player: ApiGameEventPlayer,
    #[serde(rename = "GameStateEvent")]
    pub game_state_event: ApiGameStateEvent,
    #[serde(rename = "GameFullEvent")]
    pub game_full_event: ApiGameFullEvent,
    #[serde(rename = "ChatLineEvent")]
    pub chat_line_event: ApiChatLineEvent,
    #[serde(rename = "OpponentGoneEvent")]
    pub opponent_gone_event: ApiOpponentGoneEvent,
}

/// `components`: the Lichess schemas KChess reads.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[ts(rename = "components")]
#[allow(non_camel_case_types)]
pub struct ApiComponents {
    pub schemas: ApiSchemas,
}

/// The account's now-playing response, the only path KChess reads.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiNowPlayingGame {
    pub full_id: String,
    pub game_id: String,
    pub fen: String,
    pub color: PlayerColor,
    pub last_move: String,
    pub source: ApiGameSource,
    pub status: Option<ApiGameStatus>,
    pub variant: ApiVariant,
    pub speed: ApiSpeed,
    pub perf: PerfType,
    pub rated: bool,
    pub rating: f64,
    pub has_moved: bool,
    pub opponent: ApiNowPlayingOpponent,
    pub is_my_turn: bool,
    pub seconds_left: f64,
    pub tournament_id: Option<String>,
    pub swiss_id: Option<String>,
    pub winner: Option<PlayerColor>,
    pub rating_diff: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ApiNowPlayingOpponent {
    #[serde(rename_all = "camelCase")]
    Human {
        id: String,
        username: String,
        #[ts(optional)]
        rating: Option<f64>,
        #[ts(optional)]
        rating_diff: Option<f64>,
    },
    Anonymous {
        id: (),
        username: String,
    },
    #[serde(rename_all = "camelCase")]
    Ai {
        id: (),
        username: String,
        /// AI level, from 1 to 8.
        ai: f64,
    },
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiNowPlaying {
    /// Number of games where it is my turn to play
    pub nb_my_turn: f64,
    /// Games I'm currently playing
    pub now_playing: Vec<ApiNowPlayingGame>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiPlayingContent {
    #[serde(rename = "application/json")]
    pub json: ApiNowPlaying,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct ApiPlayingResponses {
    #[serde(rename = "200")]
    pub ok: ApiPlayingContentResponse,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct ApiPlayingContentResponse {
    pub content: ApiPlayingContent,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct ApiPlayingGet {
    pub responses: ApiPlayingResponses,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct ApiPlayingPath {
    pub get: ApiPlayingGet,
}

/// `paths`: the Lichess endpoints KChess reads.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[ts(rename = "paths")]
#[allow(non_camel_case_types)]
pub struct ApiPaths {
    #[serde(rename = "/api/account/playing")]
    pub account_playing: ApiPlayingPath,
}

/// The Lichess schema and path types, in declaration order.
pub fn items(cfg: &Config) -> Vec<Item> {
    let mut out = Vec::new();
    macro_rules! decl {
        ($($t:ty),+ $(,)?) => { $(out.push(Item::of::<$t>(cfg));)+ };
    }
    decl!(
        ApiTitle,
        ApiSpeed,
        ApiVariantKey,
        ApiGameStatusName,
        ApiGameSource,
        ApiChallengeStatus,
        ApiDeclineReasonKey,
        ApiPerf,
        ApiPuzzleModePerf,
        ApiPerfs,
        ApiProfile,
        ApiPlayTime,
        ApiUser,
        ApiCount,
        ApiStreamChannel,
        ApiUserStreamer,
        ApiUserExtended,
        ApiRatingHistoryEntry,
        ApiVariant,
        ApiGameStatus,
        ApiGameEventOpponent,
        ApiGameCompat,
        ApiGameEventInfo,
        ApiGameStartEvent,
        ApiGameFinishEvent,
        ApiChallengeUser,
        ApiTimeControl,
        ApiChallengePerf,
        ApiChallengeJson,
        ApiChallengeEvent,
        ApiChallengeCanceledEvent,
        ApiChallengeDeclinedJson,
        ApiChallengeDeclinedEvent,
        ApiGameEventPlayer,
        ApiExpiration,
        ApiGameStateEvent,
        ApiGameClock,
        ApiPerfName,
        ApiGameFullEvent,
        ApiChatLineEvent,
        ApiOpponentGoneEvent,
        ApiSchemas,
        ApiComponents,
        ApiNowPlayingGame,
        ApiNowPlayingOpponent,
        ApiNowPlaying,
        ApiPlayingContent,
        ApiPlayingResponses,
        ApiPlayingContentResponse,
        ApiPlayingGet,
        ApiPlayingPath,
        ApiPaths,
    );
    out
}
