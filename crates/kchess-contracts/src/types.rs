//! `crates/kchess-contracts/ts/types.ts`: the shapes every frontend exchanges with the core.
//!
//! Conventions (they reproduce the TypeScript exactly):
//! - Field names are camelCase on the wire. `Option` fields are optional (`?:`); a field that is
//!   `T | null` is `#[ts(optional = nullable)]` when optional, or a `#[ts(type = ...)]` override.
//! - A string-literal union is a Rust enum. `wire_enum!` also emits the `as const` array that
//!   the TypeScript exports under the same name (`APPEARANCES`, ...); `union!` does not.
//! - `interface X extends Y` is `#[serde(flatten)] #[ts(flatten)]`.
//! - Numeric unions and `typeof` / indexed / generic forms are raw declarations (`raw.rs`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::{Config, TS};

use crate::generate::Item;
use crate::variant::Variant;

/// A string-literal union that the TypeScript exports only as a type.
macro_rules! union {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant,)+
        }
    };
}

/// A string-literal union with its runtime array (`export const NAME = [...] as const`).
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident, $konst:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        union!($(#[$meta])* $name { $($variant = $wire),+ });
        /// The wire values of the union, in declaration order.
        pub const $konst: &[&str] = &[$($wire),+];
    };
}

/* ── Choices and settings ───────────────────────────────────────────── */

wire_enum!(Appearance, APPEARANCES {
    System = "system",
    Light = "light",
    Dark = "dark",
});
wire_enum!(CoordinateMode, COORDINATE_MODES {
    None = "none",
    Inside = "inside",
    Outside = "outside",
});
wire_enum!(EngineLevel, ENGINE_LEVELS {
    Beginner = "beginner",
    Novice = "novice",
    Casual = "casual",
    Club = "club",
    StrongClub = "strong-club",
    Expert = "expert",
    Cm = "cm",
    Fm = "fm",
    Im = "im",
    Gm = "gm",
    SuperGm = "super-gm",
    Max = "max",
});
wire_enum!(PromotionMode, PROMOTION_MODES {
    Ask = "ask",
    Queen = "queen",
    Premove = "premove",
});
wire_enum!(#[doc = "How a pawn reaching the last rank is promoted: always ask, always to a queen, or a queen only when premoving."] PieceAnimation, PIECE_ANIMATIONS {
    None = "none",
    Fast = "fast",
    Normal = "normal",
    Slow = "slow",
});
wire_enum!(#[doc = "How long a piece takes to slide to its square."] NotificationKind, NOTIFICATION_KINDS {
    OpponentMove = "opponentMove",
    LowTime = "lowTime",
    GameEvents = "gameEvents",
    ComputerMove = "computerMove",
    Challenge = "challenge",
    Test = "test",
});
wire_enum!(#[doc = "What a desktop notification is about; each kind (except `test`) has its own switch in Settings."] OnlineAction, ONLINE_ACTIONS {
    Resign = "resign",
    Abort = "abort",
    Takeback = "takeback",
    DeclineTakeback = "declineTakeback",
    OfferDraw = "offerDraw",
    AcceptDraw = "acceptDraw",
    DeclineDraw = "declineDraw",
    ClaimVictory = "claimVictory",
    ClaimDraw = "claimDraw",
    Berserk = "berserk",
});
wire_enum!(#[doc = "Reasons Lichess accepts when declining a challenge (it translates them for the challenger)."] DeclineReason, DECLINE_REASONS {
    Generic = "generic",
    Later = "later",
    TooFast = "tooFast",
    TooSlow = "tooSlow",
    TimeControl = "timeControl",
    Rated = "rated",
    Casual = "casual",
    Standard = "standard",
    Variant = "variant",
});
wire_enum!(ChatRoom, CHAT_ROOMS {
    Player = "player",
    Spectator = "spectator",
});
wire_enum!(ChallengeColor, CHALLENGE_COLORS {
    Random = "random",
    White = "white",
    Black = "black",
});
wire_enum!(#[doc = "How far from your Lichess puzzle rating the next puzzle is aimed."] PuzzleDifficulty, PUZZLE_DIFFICULTIES {
    Easiest = "easiest",
    Easier = "easier",
    Normal = "normal",
    Harder = "harder",
    Hardest = "hardest",
});
wire_enum!(#[doc = "Everything the runs table (local scores) records."] RunKind, RUN_KINDS {
    Storm = "storm",
    Streak = "streak",
    Rush = "rush",
    Coordinates = "coordinates",
    SquareColor = "squareColor",
    KnightPath = "knightPath",
    Endgame = "endgame",
});
wire_enum!(#[doc = "Where a spoken phrase was said."] VoiceSource, VOICE_SOURCES {
    Computer = "computer",
    Coordinates = "coordinates",
    Analysis = "analysis",
    Editor = "editor",
    Local = "local",
});
wire_enum!(#[doc = "What became of a spoken phrase. Moves: `played` at once, `pending` a choice that then was"] #[doc = "`confirmed` (by voice), `picked` (clicked), `cancelled`, `replaced` by another phrase or"] #[doc = "`abandoned` (the position moved on). `invalid` matched no legal move, `unclear` was too uncertain"] #[doc = "to use, `command` was “confirm”/“cancel”. Coordinates: `correct` or `wrong`."] VoiceOutcome, VOICE_OUTCOMES {
    Played = "played",
    Pending = "pending",
    Confirmed = "confirmed",
    Picked = "picked",
    Cancelled = "cancelled",
    Replaced = "replaced",
    Abandoned = "abandoned",
    Invalid = "invalid",
    Unclear = "unclear",
    Command = "command",
    Correct = "correct",
    Wrong = "wrong",
});
wire_enum!(Judgment, JUDGMENTS {
    Inaccuracy = "inaccuracy",
    Mistake = "mistake",
    Blunder = "blunder",
});
wire_enum!(ReviewAuto, REVIEW_AUTO {
    Off = "off",
    Recent = "recent",
    All = "all",
});
wire_enum!(#[doc = "`opening` is the Lichess games database, `masters` over-the-board master games, `player` one player's games."] PositionLookupKind, POSITION_LOOKUP_KINDS {
    Opening = "opening",
    Masters = "masters",
    Player = "player",
    Tablebase = "tablebase",
});
wire_enum!(ExplorerSpeed, EXPLORER_SPEEDS {
    UltraBullet = "ultraBullet",
    Bullet = "bullet",
    Blitz = "blitz",
    Rapid = "rapid",
    Classical = "classical",
    Correspondence = "correspondence",
});
wire_enum!(PerfType, PERF_TYPES {
    UltraBullet = "ultraBullet",
    Bullet = "bullet",
    Blitz = "blitz",
    Rapid = "rapid",
    Classical = "classical",
    Correspondence = "correspondence",
    Chess960 = "chess960",
    KingOfTheHill = "kingOfTheHill",
    ThreeCheck = "threeCheck",
    Antichess = "antichess",
    Atomic = "atomic",
    Horde = "horde",
    RacingKings = "racingKings",
    Crazyhouse = "crazyhouse",
});
wire_enum!(ExportKind, EXPORT_KINDS {
    Gif = "gif",
    Png = "png",
    Pgn = "pgn",
});

union!(PlayerColor { White = "white", Black = "black" });
union!(TournamentSystem { Arena = "arena", Swiss = "swiss" });
union!(LookupMode { Rated = "rated", Casual = "casual" });
union!(GameResultFilter { Win = "win", Loss = "loss", Draw = "draw" });
union!(ChallengeDirection { In = "in", Out = "out" });
union!(Lane { Events = "events", Game = "game", Seek = "seek" });
union!(OnlinePhase {
    Checking = "checking",
    Connecting = "connecting",
    Connected = "connected",
    Reconnecting = "reconnecting",
    Disconnected = "disconnected",
    AuthRequired = "auth-required",
    Idle = "idle",
});
union!(TournamentStatus { Created = "created", Started = "started", Finished = "finished" });
union!(UsageKind {
    Games = "games",
    Profile = "profile",
    Play = "play",
    Presence = "presence",
    Puzzles = "puzzles",
    Database = "database",
    Watch = "watch",
    Tournament = "tournament",
    Analysis = "analysis",
    Study = "study",
    Other = "other",
});
union!(ReviewSource { Lichess = "lichess", Local = "local" });
union!(WatchPhase { Connecting = "connecting", Connected = "connected", Ended = "ended", Error = "error" });
union!(WatchSource { Tv = "tv", Game = "game" });
union!(BroadcastSection { Active = "active", Upcoming = "upcoming", Past = "past" });
union!(AnalysisReason { Completed = "completed", Interrupted = "interrupted", Failed = "failed" });
union!(ReviewPause { Battery = "battery", Engine = "engine", Online = "online", Off = "off" });
union!(AppUpdatePhase {
    Idle = "idle",
    Checking = "checking",
    UpToDate = "up-to-date",
    Available = "available",
    Downloading = "downloading",
    Downloaded = "downloaded",
    Error = "error",
    Disabled = "disabled",
});
union!(MicrophoneStatus {
    Granted = "granted",
    Denied = "denied",
    Restricted = "restricted",
    NotDetermined = "not-determined",
    Unknown = "unknown",
});
union!(NotificationSkip {
    Disabled = "disabled",
    CategoryOff = "category-off",
    WindowState = "window-state",
    Unsupported = "unsupported",
    Failed = "failed",
});
union!(NotificationVia { System = "system", InApp = "in-app" });
union!(PuzzleDbPhase {
    Downloading = "downloading",
    Importing = "importing",
    Done = "done",
    Cancelled = "cancelled",
    Failed = "failed",
});
union!(VoiceModelPhase { Checking = "checking", Downloading = "downloading", Preparing = "preparing" });

/// Colors of one light or dark variant of an app theme, as `#rgb` / `#rrggbb`. Optional ones are derived from the rest.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ThemePalette {
    /// Page background.
    pub bg: String,
    /// Main text.
    pub text: String,
    /// Accent: buttons, links, focus.
    pub primary: String,
    /// Slightly raised areas.
    pub muted: Option<String>,
    /// Cards.
    pub elevated: Option<String>,
    /// Hover and selected backgrounds.
    pub accented: Option<String>,
    pub border: Option<String>,
    pub text_muted: Option<String>,
    pub success: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
    pub info: Option<String>,
}

/// A color theme for the whole app. A theme with only one variant uses it in both light and dark mode.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AppTheme {
    pub id: String,
    pub name: String,
    pub light: Option<ThemePalette>,
    pub dark: Option<ThemePalette>,
    /// Loaded from the user's themes folder rather than shipped with the app.
    pub custom: Option<bool>,
}

/// Resolved CSS colors the browser's Lichess sign-in page uses to match the app.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthPageColors {
    pub bg: String,
    pub elevated: String,
    pub text: String,
    pub text_muted: String,
    pub primary: String,
    pub border: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthPageLook {
    pub appearance: Appearance,
    pub light: OAuthPageColors,
    pub dark: OAuthPageColors,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomThemeReport {
    pub themes: Vec<AppTheme>,
    /// The folder custom theme files are read from.
    pub dir: String,
    /// One line per file that could not be used.
    pub problems: Vec<String>,
}

/// Preferences consumed by headless services.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoreSettings {
    pub engine_path: String,
    /// Keep a local log of what voice input heard and what came of it, to find what it mishears.
    pub voice_history: bool,
    /// Computer levels offered in Play with Computer, in ladder order; never empty.
    pub engine_levels: Vec<EngineLevel>,
    /// Review your games in the background: none, the last 30 days', or all of them.
    pub review_auto: ReviewAuto,
    /// Keep reviewing in the background on battery power.
    pub review_on_battery: bool,
    /// Keep the Lichess event stream open while idle, so challenges (and tournament pairings) arrive.
    pub receive_challenges: bool,
    /// Show the player chat in online games.
    pub online_chat: bool,
    /// Minutes between checks of correspondence games for your turn; 0 turns checking off.
    pub correspondence_poll: f64,
    /// Ask Lichess's cloud for a cached evaluation of analysis positions (sends the position).
    pub cloud_eval: bool,
}

/// Presentation and desktop host preferences, preserved separately from the core API.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrontendPreferences {
    pub appearance: Appearance,
    pub board_theme: String,
    /// Color theme of the app itself in light mode: `kchess` (the default look), a preset, or a custom theme id.
    pub light_theme: String,
    /// The same for dark mode; the two are chosen independently.
    pub dark_theme: String,
    /// One of the piece sets shipped in `apps/desktop/app/assets/pieces`.
    pub piece_set: String,
    pub piece_animation: PieceAnimation,
    pub coordinates: CoordinateMode,
    pub sound_enabled: bool,
    pub sound_volume: f64,
    /// Let the player queue a move while it is the opponent's turn.
    pub premove: bool,
    pub promotion: PromotionMode,
    /// Dot the squares a selected piece can move to.
    pub show_legal_moves: bool,
    /// Master switch for desktop notifications.
    pub notifications_enabled: bool,
    /// Alert (in the app) while KChess is the window in use.
    pub notify_active: bool,
    /// Alert (a system notification) while KChess is in the background: unfocused, minimized or hidden.
    pub notify_background: bool,
    /// Your online opponent has moved.
    pub notify_opponent_move: bool,
    /// Your clock is running low in an online game.
    pub notify_low_time: bool,
    /// An online game starts or ends.
    pub notify_game_events: bool,
    /// Stockfish has replied in a computer game.
    pub notify_computer_move: bool,
    /// Let the operating system play its notification sound (KChess's own game sounds are separate).
    pub notify_sound: bool,
    /// Voice input listens only while Space or the on-screen button is held.
    pub voice_push_to_talk: bool,
    /// A spoken move waits for "confirm" (or a click) before it is played.
    pub voice_confirm_moves: bool,
    /// Check stable releases on startup and periodically while KChess is open.
    pub update_auto_check: bool,
    /// Download verified updates in the background when a check finds one.
    pub update_auto_download: bool,
    /// Apply a downloaded update when the user quits; never restart during play.
    pub update_install_on_quit: bool,
    /// Alert when someone challenges you.
    pub notify_challenges: bool,
    /// Hide everything but the board, clocks and essential controls while playing.
    pub zen_mode: bool,
    /// Hide the pieces in games you play (the move list and clocks stay).
    pub blindfold: bool,
    /// Name the opening of the position in games and analysis.
    pub show_opening_name: bool,
    /// Go back and forward with a horizontal two-finger swipe.
    pub swipe_navigation: bool,
    /// Show an edge hint that follows the swipe.
    pub swipe_indicator: bool,
}

/// Every setting: the core's preferences and the desktop's.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(flatten)]
    #[ts(flatten)]
    pub core: CoreSettings,
    #[serde(flatten)]
    #[ts(flatten)]
    pub frontend: FrontendPreferences,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateProgress {
    pub percent: f64,
    pub transferred: f64,
    pub total: f64,
    pub bytes_per_second: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AppUpdateStatus {
    pub phase: AppUpdatePhase,
    pub current_version: String,
    pub can_check: bool,
    pub can_install: bool,
    /// Why this installation needs a manual download, or cannot check at all.
    pub reason: Option<String>,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub checked_at: Option<f64>,
    pub progress: Option<AppUpdateProgress>,
    pub error: Option<String>,
}

/// The operating system's microphone permission for KChess (`granted` where the OS has none).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneAccess {
    pub status: MicrophoneStatus,
    /// A privacy settings page can be opened (macOS and Windows).
    pub can_open_settings: bool,
    /// A macOS dev build: the permission belongs to the terminal or editor that launched KChess.
    pub launched_from_terminal: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRequest {
    pub kind: NotificationKind,
    pub title: String,
    pub body: String,
}

/// Why a notification was not shown, when it was not.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct NotificationResult {
    pub shown: bool,
    /// `system` is an operating-system notification; `in-app` is an alert inside the window.
    pub via: Option<NotificationVia>,
    pub skipped: Option<NotificationSkip>,
    /// The operating system's reason, when it refused the notification (`skipped` is `failed`).
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LichessAccount {
    pub username: String,
    pub connected: bool,
    pub last_synced_at: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LichessGame {
    pub id: String,
    pub account: String,
    pub created_at: f64,
    pub last_move_at: f64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub status: String,
    pub winner: Option<PlayerColor>,
    pub color: PlayerColor,
    pub opponent: String,
    pub opponent_rating: Option<f64>,
    pub player_rating: Option<f64>,
    pub rating_diff: Option<f64>,
    pub opening: Option<String>,
    pub moves: String,
    pub pgn: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GamePageQuery {
    pub account: Option<String>,
    pub result: Option<GameResultFilter>,
    pub rated: Option<bool>,
    pub offset: f64,
    pub limit: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GamePage {
    pub games: Vec<LichessGame>,
    pub total: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub total: f64,
    pub win: f64,
    pub loss: f64,
    pub draw: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct InsightsQuery {
    pub account: String,
    pub speed: Option<String>,
    pub rated: Option<bool>,
    /// Only games from the last this-many days.
    pub days: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColorRecords {
    pub white: GameRecord,
    pub black: GameRecord,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeedRecord {
    pub speed: String,
    pub record: GameRecord,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpeningRecord {
    pub name: String,
    pub record: GameRecord,
    pub as_white: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LabeledRecord {
    pub label: String,
    pub record: GameRecord,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EndingRecord {
    pub status: String,
    pub record: GameRecord,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Streaks {
    pub longest_win: f64,
    pub longest_loss: f64,
    pub current: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GamePerfAverages {
    pub inaccuracy: f64,
    pub mistake: f64,
    pub blunder: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct InsightsAccuracy {
    pub games: f64,
    pub average: Option<f64>,
    pub acpl: Option<f64>,
    pub per_game: GamePerfAverages,
}

/// Patterns in one account's synced games (worked out locally).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct InsightsReport {
    pub account: String,
    pub total: f64,
    pub record: GameRecord,
    pub by_color: ColorRecords,
    pub by_speed: Vec<SpeedRecord>,
    /// Opening families, most played first.
    pub by_opening: Vec<OpeningRecord>,
    /// Sunday first, in this computer's time zone.
    pub by_weekday: Vec<GameRecord>,
    pub by_hour: Vec<GameRecord>,
    /// By the opponent's rating minus yours.
    pub by_opponent: Vec<LabeledRecord>,
    pub by_length: Vec<LabeledRecord>,
    /// How games ended (`mate`, `resign`, `outoftime`, `draw`…).
    pub endings: Vec<EndingRecord>,
    pub streaks: Streaks,
    /// From reviewed games only.
    pub accuracy: Option<InsightsAccuracy>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameLibraryOverview {
    pub by_account: BTreeMap<String, GameRecord>,
    pub versus: BTreeMap<String, GameRecord>,
}

/// One player's side of a review.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewSide {
    pub accuracy: Option<f64>,
    /// Average centipawn loss.
    pub acpl: Option<f64>,
    pub inaccuracy: f64,
    pub mistake: f64,
    pub blunder: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct OnlineConnection {
    pub session: f64,
    pub account: String,
    pub game_id: String,
    pub lane: Lane,
    pub phase: OnlinePhase,
    pub message: Option<String>,
}

/// A player as a challenge or ongoing game names them.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PlayerRef {
    pub name: String,
    pub rating: Option<f64>,
    pub title: Option<String>,
    pub provisional: Option<bool>,
    pub online: Option<bool>,
}

/// Time control of a challenge.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChallengeTimeControl {
    #[serde(rename = "clock")]
    Clock { limit: f64, increment: f64 },
    #[serde(rename = "correspondence")]
    Correspondence { days: f64 },
    #[serde(rename = "unlimited")]
    Unlimited,
}

/// A pending challenge to or from one of the connected accounts.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ChallengeInfo {
    pub id: String,
    /// The connected account it was sent to (incoming) or from (outgoing).
    pub account: String,
    pub direction: ChallengeDirection,
    /// The other player.
    pub opponent: PlayerRef,
    /// Lichess variant key (`standard`, `chess960`, `crazyhouse`…).
    pub variant: String,
    pub variant_name: String,
    pub rated: bool,
    pub speed: String,
    pub time_control: ChallengeTimeControl,
    /// The colour the challenger asked for.
    pub color: ChallengeColor,
    /// A rematch offer for this game.
    pub rematch_of: Option<String>,
    pub initial_fen: Option<String>,
    /// Lichess says KChess (a Board API client) can play this game, and KChess knows its variant.
    pub playable: bool,
    /// Why it cannot be accepted here, when it cannot.
    pub problem: Option<String>,
    pub received_at: f64,
}

/// Health of the idle event stream that waits for challenges and pairings.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LobbyState {
    pub account: String,
    pub phase: OnlinePhase,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: ChatRoom,
}

/// A game a connected account is playing, real-time or correspondence.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    pub color: PlayerColor,
    pub fen: String,
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    /// Seconds left on your clock (or for your correspondence move), when Lichess says.
    pub seconds_left: Option<f64>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    pub tournament_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentClock {
    /// Seconds and seconds per move.
    pub limit: f64,
    pub increment: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentTeam {
    pub id: String,
    pub name: String,
}

/// An arena or Swiss tournament in a list.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
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
    pub minutes: Option<f64>,
    pub round: Option<f64>,
    pub nb_rounds: Option<f64>,
    pub nb_players: f64,
    pub starts_at: f64,
    pub finishes_at: Option<f64>,
    /// The team a Swiss event belongs to.
    pub team: Option<TournamentTeam>,
    /// The Board API can play its games and KChess knows its variant.
    pub playable: bool,
    pub problem: Option<String>,
    pub perf: Option<String>,
    /// How often Lichess runs it (`hourly`, `eastern`, `weekly`…); absent for player-made events.
    pub freq: Option<String>,
    /// Only players rated at most this may join.
    pub max_rating: Option<f64>,
}

/// An arena to create on Lichess.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct NewArena {
    /// Empty for a random Grandmaster name.
    pub name: Option<String>,
    #[ts(type = "(typeof ARENA_CLOCK_MINUTES)[number]")]
    pub clock_time: f64,
    #[ts(type = "(typeof ARENA_INCREMENTS)[number]")]
    pub clock_increment: f64,
    #[ts(type = "(typeof ARENA_DURATIONS)[number]")]
    pub minutes: f64,
    /// Minutes from now, unless `startDate` is set.
    #[ts(type = "(typeof ARENA_WAIT_MINUTES)[number]", optional)]
    pub wait_minutes: Option<f64>,
    pub start_date: Option<f64>,
    pub variant: String,
    pub rated: bool,
    /// An entry code players must type to join.
    pub password: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentStanding {
    pub rank: f64,
    pub name: String,
    pub title: Option<String>,
    pub rating: Option<f64>,
    pub score: Option<f64>,
    /// Arena score sheet: one digit per game, newest last.
    pub sheet: Option<String>,
    pub fire: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentPlayer {
    pub name: String,
    pub rating: Option<f64>,
    pub rank: Option<f64>,
}

/// A game between two tournament players, with their current ranks.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentGame {
    pub id: String,
    pub white: TournamentPlayer,
    pub black: TournamentPlayer,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentMe {
    pub rank: Option<f64>,
    pub withdraw: Option<bool>,
    pub game_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentCondition {
    pub condition: String,
    pub verdict: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentVerdicts {
    pub accepted: bool,
    pub list: Vec<TournamentCondition>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentClocks {
    pub white: f64,
    pub black: f64,
}

/// The top game being played now in an arena.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentFeatured {
    #[serde(flatten)]
    #[ts(flatten)]
    pub game: TournamentGame,
    pub fen: String,
    pub orientation: PlayerColor,
    pub last_move: Option<String>,
    /// Seconds left on each clock when Lichess sent it.
    pub clocks: Option<TournamentClocks>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentPodium {
    pub name: String,
    pub rank: f64,
    pub rating: Option<f64>,
    pub score: Option<f64>,
    pub performance: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentStats {
    pub games: f64,
    pub white_wins: f64,
    pub black_wins: f64,
    pub draws: f64,
    pub berserks: f64,
    pub average_rating: f64,
}

/// One tournament, with the leaderboard, the games being played and (when finished) its results.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TournamentDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub summary: TournamentSummary,
    pub description: Option<String>,
    pub seconds_to_start: Option<f64>,
    pub seconds_to_finish: Option<f64>,
    pub berserkable: Option<bool>,
    pub standing: Vec<TournamentStanding>,
    /// You, when the account you asked as has joined.
    pub me: Option<TournamentMe>,
    /// Entry conditions and whether the account meets them.
    pub verdicts: Option<TournamentVerdicts>,
    /// Swiss: when the next round starts.
    pub next_round_in: Option<f64>,
    /// The leaderboard page shown (10 players each).
    pub standing_page: Option<f64>,
    /// Arena: the top game being played now.
    pub featured: Option<TournamentFeatured>,
    /// Arena: games being played now.
    pub duels: Option<Vec<TournamentGame>>,
    /// Arena: the top three once it has finished.
    pub podium: Option<Vec<TournamentPodium>>,
    /// Arena: totals once it has finished.
    pub stats: Option<TournamentStats>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentList {
    pub arenas: Vec<TournamentSummary>,
    pub swiss: Vec<TournamentSummary>,
    /// Teams of the account whose Swiss events could not be read, and other notes.
    pub problems: Vec<String>,
}

/* ── Players ──────────────────────────────────────────────────────────── */

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfResult {
    pub opponent: String,
    pub opponent_rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfMark {
    pub rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfStreak {
    pub current: f64,
    pub best: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfCount {
    pub all: f64,
    pub rated: f64,
    pub win: f64,
    pub loss: f64,
    pub draw: f64,
    pub tour: f64,
    pub berserk: f64,
    pub op_avg: f64,
    pub seconds: f64,
    pub disconnects: f64,
}

/// One player's record in one rating category, as Lichess's perf page shows it.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PerfStats {
    pub perf: PerfType,
    pub rating: Option<f64>,
    pub deviation: Option<f64>,
    pub provisional: Option<bool>,
    /// Position on the leaderboard, when ranked.
    pub rank: Option<f64>,
    /// Better than this share of players.
    pub percentile: Option<f64>,
    pub progress: Option<f64>,
    pub count: PerfCount,
    pub highest: Option<PerfMark>,
    pub lowest: Option<PerfMark>,
    pub best_wins: Vec<PerfResult>,
    pub worst_losses: Vec<PerfResult>,
    pub win_streak: PerfStreak,
    pub loss_streak: PerfStreak,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrosstableMatchup {
    pub users: BTreeMap<String, f64>,
    pub nb_games: f64,
}

/// Two players' lifetime score against each other (and in their current match, if any).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Crosstable {
    /// Lower-cased username → points (wins plus half-points for draws).
    pub users: BTreeMap<String, f64>,
    pub nb_games: f64,
    pub matchup: Option<CrosstableMatchup>,
}

/* ── Watching games ─────────────────────────────────────────────────── */

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WatchPlayer {
    pub name: String,
    pub title: Option<String>,
    pub rating: Option<f64>,
}

/// Connection health of the TV/game feed, scoped to its watch session.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WatchState {
    pub session: f64,
    pub phase: WatchPhase,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchClock {
    pub initial: f64,
    pub increment: f64,
}

/// A game a TV channel showed before the current one, with its result once Lichess has it.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TvPastGame {
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    /// The last position the feed sent.
    pub fen: String,
    pub last_move: Option<String>,
    pub orientation: PlayerColor,
    pub status: Option<String>,
    pub winner: Option<PlayerColor>,
}

/// One position of a game being watched (TV or a game by id).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WatchFrame {
    /// Which `watch` call it belongs to; frames of an earlier one are stale.
    pub session: f64,
    pub source: WatchSource,
    pub channel: Option<String>,
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    pub orientation: PlayerColor,
    /// Board placement plus side to move (pockets of Crazyhouse are dropped).
    pub fen: String,
    pub last_move: Option<String>,
    /// Clocks in seconds when the frame was sent.
    pub white_clock: Option<f64>,
    pub black_clock: Option<f64>,
    pub variant: String,
    pub speed: Option<String>,
    /// Rated on Lichess; unknown until the game's details arrive.
    pub rated: Option<bool>,
    /// Time control in seconds; absent for correspondence or until the details arrive.
    pub clock: Option<WatchClock>,
    pub status: Option<String>,
    pub winner: Option<PlayerColor>,
    pub finished: bool,
    /// FEN before the first move, once the move list is known.
    pub start_fen: Option<String>,
    /// UCI moves from `startFen`; absent until the TV feed and Lichess's delayed export line up.
    pub moves: Option<Vec<String>>,
    /// Games this channel showed earlier in this session, newest first.
    pub previous: Option<Vec<TvPastGame>>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TvChannel {
    pub key: String,
    pub label: String,
    pub game_id: Option<String>,
    pub player: Option<WatchPlayer>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct BroadcastSummary {
    pub players: Option<String>,
    pub tour_id: String,
    pub tour_name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub round_id: Option<String>,
    pub round_name: Option<String>,
    pub ongoing: bool,
    pub starts_at: Option<f64>,
    pub section: BroadcastSection,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct BroadcastRoundRef {
    pub id: String,
    pub name: String,
    pub ongoing: bool,
    pub finished: bool,
    pub starts_at: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct BroadcastTourDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub rounds: Vec<BroadcastRoundRef>,
    pub default_round_id: Option<String>,
}

/// One board of a broadcast round, as the live PGN feed last described it.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
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
    pub last_move: Option<String>,
    /// Clocks in seconds, from the last comments of each side.
    pub white_clock: Option<f64>,
    pub black_clock: Option<f64>,
    pub ongoing: bool,
    /// The game as PGN, for the analysis board.
    pub pgn: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct BroadcastUpdate {
    pub session: f64,
    pub round_id: String,
    /// Games that changed (all of them at first).
    pub games: Vec<BroadcastGame>,
    /// The feed ended (the round is over or the connection dropped).
    pub ended: Option<bool>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct EngineManaged {
    pub installed: bool,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct EngineStatus {
    /// Identity of the executable currently selected, including native file changes.
    pub identity: Option<String>,
    pub ready: bool,
    /// The executable in use (a downloaded or chosen one), or empty when the bundled engine is.
    pub path: String,
    pub bundled: bool,
    /// The Stockfish KChess downloaded, whether or not it is the one in use.
    pub managed: EngineManaged,
    /// An official native download exists for this platform and architecture.
    pub can_download: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct OnlineOptions {
    /// Ignored for correspondence games (`days`).
    pub minutes: f64,
    pub increment: f64,
    /// Correspondence: days per move instead of a clock.
    #[ts(type = "CorrespondenceDays", optional)]
    pub days: Option<f64>,
    /// Lichess variant; standard when omitted.
    pub variant: Option<Variant>,
    /// Start a direct challenge from this position (standard or Chess960 rules).
    pub fen: Option<String>,
    pub color: ChallengeColor,
    /// Rated games change the Lichess rating; casual ones do not.
    pub rated: bool,
    pub target: Option<String>,
    /// Which connected account plays; defaults to the first connected one.
    pub account: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct FollowedRatings {
    pub bullet: Option<f64>,
    pub blitz: Option<f64>,
    pub rapid: Option<f64>,
    pub classical: Option<f64>,
}

/// A player one of the connected accounts follows on Lichess.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct FollowedUser {
    pub username: String,
    pub title: Option<String>,
    pub ratings: FollowedRatings,
    /// Connected accounts that follow them.
    pub followed_by: Vec<String>,
    /// Already a friend in KChess (or one of your own accounts).
    pub already_added: bool,
    /// Removed as a friend earlier; not suggested again unless asked.
    pub dismissed: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingProblem {
    pub account: String,
    pub message: String,
    /// Connecting the account again grants the missing permission.
    pub needs_reconnect: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingReport {
    pub users: Vec<FollowedUser>,
    pub problems: Vec<FollowingProblem>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageCell {
    pub requests: f64,
    /// Bytes received, after decompression.
    pub bytes_in: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub total: UsageCell,
    #[ts(type = "Partial<Record<UsageKind, UsageCell>>")]
    pub by_kind: BTreeMap<UsageKind, UsageCell>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorage {
    pub games: f64,
    /// Approximate size of the stored games.
    pub bytes: f64,
    /// Profile and rating data cached for the account.
    pub cache_bytes: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct UsageReport {
    /// When counting started; undefined when nothing has been counted.
    pub since: Option<f64>,
    /// Lower-cased username → downloads from Lichess; the empty key is traffic with no account (login, etc.).
    pub accounts: BTreeMap<String, AccountUsage>,
    /// Lower-cased username → what is kept on this computer.
    pub storage: BTreeMap<String, AccountStorage>,
    /// Size of the database file.
    pub db_bytes: f64,
}

/// Lichess's view of one user's connection. `signal` runs 1 (poor, lag > 500 ms) to 4 (great, lag < 150 ms).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    pub signal: Option<f64>,
    /// The game they are playing, when Lichess says (watchable from the Watch page).
    pub playing_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    /// Lower-cased username → status; users Lichess did not report are absent.
    pub users: BTreeMap<String, UserPresence>,
    /// Round trip of the status request itself, i.e. this computer's latency to Lichess.
    pub latency_ms: f64,
}

/// One puzzle, whichever source it came from.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Puzzle {
    pub id: String,
    /// The position the player starts from: it is the player's move, after the opponent's `lastMove`.
    pub fen: String,
    /// UCI of the opponent's move that led here (for the board highlight).
    pub last_move: Option<String>,
    /// UCI moves, alternating player, opponent, … and ending with the player's move.
    pub solution: Vec<String>,
    pub rating: f64,
    pub themes: Vec<String>,
    pub plays: Option<f64>,
    /// Lichess game the position came from.
    pub game_id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleGlicko {
    pub rating: Option<f64>,
    pub deviation: Option<f64>,
    pub provisional: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleRequest {
    /// Connected account to train as; empty trains anonymously (no rating, nothing recorded).
    pub account: String,
    /// A puzzle theme key (`mateIn2`, `fork`, …) or `mix`.
    pub angle: String,
    pub difficulty: PuzzleDifficulty,
    pub color: Option<PlayerColor>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleDraw {
    pub puzzle: Puzzle,
    /// The account's current puzzle rating, when Lichess reports it.
    pub glicko: Option<PuzzleGlicko>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleSolveRequest {
    pub account: String,
    pub angle: String,
    pub id: String,
    pub win: bool,
    /// Rated results change the Lichess puzzle rating; unrated ones are only marked as seen.
    pub rated: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleSolveResult {
    /// Rating change Lichess applied; absent for unrated results.
    pub rating_diff: Option<f64>,
}

/// Returned instead of data when the account's Lichess login lacks (or lost) the puzzle permission.
#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NeedsReconnect {
    #[ts(type = "true")]
    pub needs_reconnect: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzlePerformance {
    pub first_wins: f64,
    pub nb: f64,
    pub performance: f64,
    pub puzzle_rating_avg: f64,
    pub replay_wins: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleThemeResult {
    pub theme: String,
    pub results: PuzzlePerformance,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDashboard {
    pub days: f64,
    pub global: PuzzlePerformance,
    pub themes: BTreeMap<String, PuzzleThemeResult>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleActivityEntry {
    /// When it was played, in ms since the epoch.
    pub date: f64,
    pub win: bool,
    pub puzzle: Puzzle,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StormHigh {
    pub all_time: f64,
    pub day: f64,
    pub month: f64,
    pub week: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StormDay {
    /// `YYYY/M/D` (Lichess's own key).
    #[serde(rename = "_id")]
    pub day_id: String,
    pub combo: f64,
    pub errors: f64,
    pub highest: f64,
    pub moves: f64,
    pub runs: f64,
    pub score: f64,
    pub time: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StormDashboard {
    pub high: StormHigh,
    pub days: Vec<StormDay>,
}

/// A puzzle-database download or import in progress (or finished), streamed to the window.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleDbProgress {
    pub phase: PuzzleDbPhase,
    /// Compressed bytes received so far.
    pub received: f64,
    /// Total compressed bytes, when the server said.
    pub total: Option<f64>,
    /// Puzzles kept so far.
    pub kept: f64,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PuzzleDbStatus {
    pub installed: bool,
    pub count: f64,
    /// Approximate size on disk.
    pub bytes: f64,
    pub imported_at: Option<f64>,
    /// True while a download runs; its progress arrives as events.
    pub busy: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LocalPuzzleQuery {
    pub theme: Option<String>,
    pub min_rating: Option<f64>,
    pub max_rating: Option<f64>,
    pub count: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLadderQuery {
    pub from: f64,
    pub to: f64,
    pub count: f64,
}

/// A run's score and details, as the game saves it.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunInput {
    pub kind: RunKind,
    /// Which flavour of the kind (`3min`, `find`, `kq-vs-k`, …).
    pub variant: String,
    pub score: f64,
    /// Extra numbers and words shown with the score (accuracy, combo, …).
    #[ts(type = "Record<string, string | number | boolean>")]
    pub detail: BTreeMap<String, Value>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    #[serde(flatten)]
    #[ts(flatten)]
    pub input: RunInput,
    pub id: f64,
    pub played_at: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunBest {
    pub score: f64,
    pub played_at: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub kind: RunKind,
    /// Best score per variant.
    pub best: BTreeMap<String, RunBest>,
    /// Newest first.
    pub recent: Vec<RunRecord>,
    /// All runs of this kind.
    pub total: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunSaved {
    pub summary: RunSummary,
    /// The run beat every earlier run of its variant.
    pub is_best: bool,
}

/// One recognized word and the recognizer's confidence in it (0–1).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWord {
    pub word: String,
    pub conf: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VoiceModelProgress {
    pub phase: VoiceModelPhase,
    pub received: f64,
    pub total: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VoiceModelStatus {
    pub installed: bool,
    pub bytes: f64,
    pub busy: bool,
    pub progress: Option<VoiceModelProgress>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VoiceAttemptInput {
    pub source: VoiceSource,
    /// The recognizer's text, as heard.
    pub heard: String,
    /// Average word confidence, 0–1.
    pub confidence: f64,
    pub words: Vec<VoiceWord>,
    pub outcome: VoiceOutcome,
    /// How KChess read it: SAN choices joined by `|`, a square, or `confirm`/`cancel`.
    pub parsed: Option<String>,
    /// What the player meant, when known: the square asked for, or the move they went on to play.
    pub expected: Option<String>,
    /// The position it was said in (FEN), so the phrase can be parsed again later.
    pub fen: Option<String>,
    /// An earlier attempt at the same move or question that this one repeats.
    pub retry_of: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttempt {
    #[serde(flatten)]
    #[ts(flatten)]
    pub input: VoiceAttemptInput,
    pub id: f64,
    pub at: f64,
}

/// What is learned about an attempt after it was logged.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VoiceAttemptUpdate {
    pub outcome: Option<VoiceOutcome>,
    pub expected: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct BestMoveOptions {
    /// Start from this position instead of the standard one.
    pub fen: Option<String>,
    /// Think for this many milliseconds instead of the level's default.
    pub movetime: Option<f64>,
    /// Chess960 rules: castling is king-takes-rook in UCI.
    pub chess960: Option<bool>,
}

/// Ask the analysis engine to study one position until stopped or a limit is reached.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AnalysisRequest {
    pub fen: String,
    /// Repetition context: root plus moves must produce fen.
    pub root_fen: Option<String>,
    pub moves: Option<Vec<String>>,
    pub client_id: Option<f64>,
    /// How many of the best lines to report (MultiPV).
    pub lines: f64,
    /// Keep searching until stopped instead of stopping at a sensible depth.
    pub infinite: Option<bool>,
}

/// One engine line. Scores are from White's point of view, like Lichess shows them.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct EngineLine {
    /// 1 for the best line, 2 for the next best…
    pub rank: f64,
    pub depth: f64,
    /// Centipawns, when there is no forced mate.
    pub cp: Option<f64>,
    /// Moves to mate: positive when White mates, negative when Black does.
    pub mate: Option<f64>,
    /// Principal variation as UCI moves.
    pub pv: Vec<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AnalysisUpdate {
    /// The request this belongs to; the renderer drops updates of superseded ones.
    pub id: f64,
    pub fen: String,
    pub depth: f64,
    /// Nodes per second, for the engine's status line.
    pub nps: Option<f64>,
    pub lines: Vec<EngineLine>,
    /// The search finished (limit reached, or stopped) and no more updates follow.
    pub done: bool,
    pub context: Option<String>,
    pub client_id: Option<f64>,
    pub reason: Option<AnalysisReason>,
    pub engine: Option<String>,
    pub error: Option<String>,
}

/* ── Game review ───────────────────────────────────────────────────── */

/// One position of a reviewed game: its score from White's side and the engine's choice there.
/// A checkmated position is `mate: 0` (the side to move has lost).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewEval {
    pub cp: Option<f64>,
    pub mate: Option<f64>,
    /// The engine's move in this position (UCI), and the line it expects after it.
    pub best: Option<String>,
    pub pv: Option<Vec<String>>,
    pub depth: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewAccuracy {
    pub white: Option<f64>,
    pub black: Option<f64>,
}

/// A game's review as stored: the raw scores; labels and accuracy are worked out from them.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct StoredReview {
    /// `reviewKey(fen, moves)`: the same moves from the same start share one review.
    pub key: String,
    pub fen: String,
    /// UCI, from `fen`.
    pub moves: Vec<String>,
    pub source: ReviewSource,
    pub engine: Option<String>,
    /// Index 0 is the starting position, index i the position after move i; null until analysed.
    pub evals: Vec<Option<ReviewEval>>,
    /// Lichess's own label for each move (index i is move i+1), when Lichess did the analysis.
    pub judgments: Option<Vec<Option<Judgment>>>,
    /// Lichess's own accuracy figures, when Lichess did the analysis.
    pub accuracy: Option<ReviewAccuracy>,
    /// Depth the local analysis aims for; 0 for Lichess's analysis.
    pub depth: f64,
    pub complete: bool,
    pub updated_at: f64,
    /// The Lichess game it belongs to, if any.
    pub game_id: Option<String>,
}

/// What the game list shows for a reviewed game.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewSummary {
    pub key: String,
    pub source: ReviewSource,
    pub engine: Option<String>,
    pub complete: bool,
    pub white: ReviewSide,
    pub black: ReviewSide,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewRequest {
    pub fen: String,
    pub moves: Vec<String>,
    /// A Lichess game: its own analysis is fetched first when it has one.
    pub game_id: Option<String>,
    pub account: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewCurrent {
    pub key: String,
    pub game_id: Option<String>,
    pub done: f64,
    pub total: f64,
    pub background: bool,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFailure {
    pub key: String,
    pub message: String,
}

/// Progress of the review queue, sent whenever it changes.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ReviewStatus {
    /// The review being worked on, with how many positions are done.
    pub current: Option<ReviewCurrent>,
    /// Reviews waiting, asked for and automatic.
    pub waiting: f64,
    /// Why automatic reviews are on hold, when they are.
    pub paused: Option<ReviewPause>,
    /// The last review asked for that could not be done, and why.
    pub failed: Option<ReviewFailure>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUpdate {
    pub review: StoredReview,
    pub summary: ReviewSummary,
}

/// Filters of the opening explorer; each database uses the ones it understands.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LookupOptions {
    /// Lichess and player databases.
    pub speeds: Option<Vec<ExplorerSpeed>>,
    /// Lichess database: rating groups (each is the lower bound of its band).
    #[ts(type = "number[]", optional)]
    pub ratings: Option<Vec<f64>>,
    /// Player database: whose games, and with which colour.
    pub player: Option<String>,
    pub color: Option<PlayerColor>,
    /// Player database: rated, casual or both.
    pub modes: Option<Vec<LookupMode>>,
    /// Games since this month (`YYYY-MM`; masters use a year).
    pub since: Option<String>,
}

/// A game the explorer lists as notable for the position.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ExplorerGame {
    pub id: String,
    pub white: String,
    pub black: String,
    pub white_rating: Option<f64>,
    pub black_rating: Option<f64>,
    pub winner: Option<PlayerColor>,
    pub year: Option<f64>,
    pub month: Option<String>,
    /// The move played from this position, as SAN.
    pub san: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LookupMove {
    pub uci: String,
    pub san: String,
    pub white: Option<f64>,
    pub draws: Option<f64>,
    pub black: Option<f64>,
    pub category: Option<String>,
    #[ts(optional = nullable)]
    pub dtz: Option<f64>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PositionLookup {
    pub kind: PositionLookupKind,
    pub fen: String,
    /// Games in the database reaching this position (opening databases).
    pub total: Option<f64>,
    pub games: Option<Vec<ExplorerGame>>,
    pub fetched_at: f64,
    pub cached: bool,
    pub stale: bool,
    pub message: Option<String>,
    pub opening: Option<String>,
    pub category: Option<String>,
    #[ts(optional = nullable)]
    pub dtz: Option<f64>,
    pub moves: Vec<LookupMove>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessStudy {
    pub id: String,
    pub name: String,
    pub updated_at: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudySyncRequest {
    pub account: String,
    pub study_id: String,
    pub baseline: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessStudyChapter {
    pub name: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    /// File name without extension.
    pub name: String,
    pub kind: ExportKind,
    #[ts(type = "Uint8Array | string")]
    pub data: Value,
}

/// A cloud evaluation: lines from White's point of view, like the local engine's.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudEval {
    pub fen: String,
    pub depth: f64,
    pub knodes: f64,
    pub lines: Vec<EngineLine>,
}

/// Every declaration and `as const` array of `types.ts`, in the file's order.
pub fn items(cfg: &Config) -> Vec<Item> {
    use crate::strs;
    let mut out = Vec::new();
    macro_rules! decl {
        ($($t:ty),+ $(,)?) => { $(out.push(Item::of::<$t>(cfg));)+ };
    }
    macro_rules! konst {
        ($($name:ident),+ $(,)?) => {
            $(out.push(Item::Const {
                name: stringify!($name).to_string(),
                values: strs($name),
                suffix: String::new(),
            });)+
        };
    }
    konst!(
        APPEARANCES,
        COORDINATE_MODES,
        ENGINE_LEVELS,
        PROMOTION_MODES,
        PIECE_ANIMATIONS,
        NOTIFICATION_KINDS,
        ONLINE_ACTIONS,
        DECLINE_REASONS,
        CHAT_ROOMS,
        CHALLENGE_COLORS,
        PUZZLE_DIFFICULTIES,
        RUN_KINDS,
        VOICE_SOURCES,
        VOICE_OUTCOMES,
        JUDGMENTS,
        REVIEW_AUTO,
        POSITION_LOOKUP_KINDS,
        EXPLORER_SPEEDS,
        PERF_TYPES,
        EXPORT_KINDS,
    );
    decl!(
        Appearance,
        CoordinateMode,
        EngineLevel,
        PromotionMode,
        PieceAnimation,
        NotificationKind,
        OnlineAction,
        DeclineReason,
        ChatRoom,
        ChallengeColor,
        PuzzleDifficulty,
        RunKind,
        VoiceSource,
        VoiceOutcome,
        Judgment,
        ReviewAuto,
        PositionLookupKind,
        ExplorerSpeed,
        PerfType,
        ExportKind,
        PlayerColor,
        TournamentSystem,
        LookupMode,
        GameResultFilter,
        ChallengeDirection,
        Lane,
        OnlinePhase,
        TournamentStatus,
        UsageKind,
        ReviewSource,
        WatchPhase,
        WatchSource,
        BroadcastSection,
        AnalysisReason,
        ReviewPause,
        AppUpdatePhase,
        MicrophoneStatus,
        NotificationSkip,
        NotificationVia,
        PuzzleDbPhase,
        VoiceModelPhase,
        ThemePalette,
        AppTheme,
        OAuthPageColors,
        OAuthPageLook,
        CustomThemeReport,
        CoreSettings,
        FrontendPreferences,
        Settings,
        AppUpdateProgress,
        AppUpdateStatus,
        MicrophoneAccess,
        NotificationRequest,
        NotificationResult,
        LichessAccount,
        LichessGame,
        GamePageQuery,
        GamePage,
        GameRecord,
        InsightsQuery,
        ColorRecords,
        SpeedRecord,
        OpeningRecord,
        LabeledRecord,
        EndingRecord,
        Streaks,
        GamePerfAverages,
        InsightsAccuracy,
        InsightsReport,
        GameLibraryOverview,
        ReviewSide,
        OnlineConnection,
        PlayerRef,
        ChallengeTimeControl,
        ChallengeInfo,
        LobbyState,
        ChatLine,
        OngoingGame,
        TournamentClock,
        TournamentTeam,
        TournamentSummary,
        NewArena,
        TournamentStanding,
        TournamentPlayer,
        TournamentGame,
        TournamentMe,
        TournamentCondition,
        TournamentVerdicts,
        TournamentClocks,
        TournamentFeatured,
        TournamentPodium,
        TournamentStats,
        TournamentDetail,
        TournamentList,
        PerfResult,
        PerfMark,
        PerfStreak,
        PerfCount,
        PerfStats,
        CrosstableMatchup,
        Crosstable,
        WatchPlayer,
        WatchState,
        WatchClock,
        TvPastGame,
        WatchFrame,
        TvChannel,
        BroadcastSummary,
        BroadcastRoundRef,
        BroadcastTourDetail,
        BroadcastGame,
        BroadcastUpdate,
        EngineManaged,
        EngineStatus,
        OnlineOptions,
        FollowedRatings,
        FollowedUser,
        FollowingProblem,
        FollowingReport,
        UsageCell,
        AccountUsage,
        AccountStorage,
        UsageReport,
        UserPresence,
        PresenceReport,
        Puzzle,
        PuzzleGlicko,
        PuzzleRequest,
        PuzzleDraw,
        PuzzleSolveRequest,
        PuzzleSolveResult,
        NeedsReconnect,
        PuzzlePerformance,
        PuzzleThemeResult,
        PuzzleDashboard,
        PuzzleActivityEntry,
        StormHigh,
        StormDay,
        StormDashboard,
        PuzzleDbProgress,
        PuzzleDbStatus,
        LocalPuzzleQuery,
        LocalLadderQuery,
        RunInput,
        RunRecord,
        RunBest,
        RunSummary,
        RunSaved,
        VoiceWord,
        VoiceModelProgress,
        VoiceModelStatus,
        VoiceAttemptInput,
        VoiceAttempt,
        VoiceAttemptUpdate,
        BestMoveOptions,
        AnalysisRequest,
        EngineLine,
        AnalysisUpdate,
        ReviewEval,
        ReviewAccuracy,
        StoredReview,
        ReviewSummary,
        ReviewRequest,
        ReviewCurrent,
        ReviewFailure,
        ReviewStatus,
        ReviewUpdate,
        LookupOptions,
        ExplorerGame,
        LookupMove,
        PositionLookup,
        LichessStudy,
        StudySyncRequest,
        LichessStudyChapter,
        ExportRequest,
        CloudEval,
    );
    out.extend(crate::raw::types_items());
    out
}
