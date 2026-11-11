//! The shapes every frontend exchanges with the core.
//!
//! Conventions:
//! - Field names are camelCase on the wire. `Option` fields are omitted when absent unless a
//!   field says otherwise with its serde attributes.
//! - A string-literal union is a Rust enum (`kchess_domain::union!`). `wire_enum!` also emits the
//!   array of wire values under the constant's name (`APPEARANCES`, ...); `union!` does not. The
//!   unions domain rules read are declared in the domain and re-exported here.
//! - A flattened parent is `#[serde(flatten)]`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::Variant;
use kchess_domain::color::Color;

use kchess_domain::{union, wire_enum};

pub use kchess_domain::misc::constants::{
    APPEARANCES, Appearance, ENGINE_LEVELS, EngineLevel, HistoryKind, NotificationKind,
    SESSION_KINDS, SessionKind, TournamentSystem,
};
pub use kchess_domain::online_game::connection::{Lane, OnlineConnection, OnlinePhase};
pub use kchess_domain::online_game::model::{CHAT_ROOMS, ChatRoom};

/* ── Choices and settings ───────────────────────────────────────────── */

wire_enum!(#[doc = "Where board coordinates are shown."] CoordinateMode, COORDINATE_MODES {
    None = "none",
    Inside = "inside",
    Outside = "outside",
});
wire_enum!(#[doc = "How a pawn reaching the last rank is promoted: always ask, always to a queen, or a queen only when premoving."] PromotionMode, PROMOTION_MODES {
    Ask = "ask",
    Queen = "queen",
    Premove = "premove",
});
wire_enum!(#[doc = "How long a piece takes to slide to its square."] PieceAnimation, PIECE_ANIMATIONS {
    None = "none",
    Fast = "fast",
    Normal = "normal",
    Slow = "slow",
});
wire_enum!(#[doc = "An action the player can take in an online game, such as resigning, aborting, offering or declining a takeback or draw, claiming a result, or going berserk."] OnlineAction, ONLINE_ACTIONS {
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
wire_enum!(#[doc = "Which color the challenger requests."] ChallengeColor, CHALLENGE_COLORS {
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
    Lesson = "lesson",
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
wire_enum!(#[doc = "Where a spoken phrase was said."] VoiceSource, VOICE_SOURCES {
    Computer = "computer",
    Coordinates = "coordinates",
    SquareColor = "squareColor",
    KnightPath = "knightPath",
    Analysis = "analysis",
    Editor = "editor",
    Local = "local",
    Puzzles = "puzzles",
});
wire_enum!(#[doc = "What became of a spoken phrase. Moves: `played` at once, `pending` a choice that then was"] #[doc = "`confirmed` (by voice), `picked` (clicked), `cancelled`, `replaced` by another phrase or"] #[doc = "`abandoned` (the position moved on). `invalid` matched no legal move, `unclear` was too uncertain"] #[doc = "to use, `command` was “confirm”/“cancel”. Coordinates and square colours: `correct` or `wrong`;"] #[doc = "knight paths: `played` for a jump, `invalid` for a square the knight cannot reach."] VoiceOutcome, VOICE_OUTCOMES {
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
union!(#[doc = "Severity of a move mistake in review."] Judgment {
    Inaccuracy = "inaccuracy",
    Mistake = "mistake",
    Blunder = "blunder",
});
wire_enum!(#[doc = "Which games are reviewed automatically."] ReviewAuto, REVIEW_AUTO {
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
wire_enum!(#[doc = "Time control of an opening-explorer game pool."] ExplorerSpeed, EXPLORER_SPEEDS {
    UltraBullet = "ultraBullet",
    Bullet = "bullet",
    Blitz = "blitz",
    Rapid = "rapid",
    Classical = "classical",
    Correspondence = "correspondence",
});
wire_enum!(#[doc = "Lichess rating category, including variants."] PerfType, PERF_TYPES {
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
wire_enum!(#[doc = "What a connected account can do about a player on Lichess."] RelationAction, RELATION_ACTIONS {
    Follow = "follow",
    Unfollow = "unfollow",
    Block = "block",
    Unblock = "unblock",
});

union!(#[doc = "Whether play is rated or casual."] LookupMode { Rated = "rated", Casual = "casual" });
/// Filter games by result. A refusal names it `GameResult`, as the recorded checks did.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(expecting = "enum GameResult")]
pub enum GameResultFilter {
    #[serde(rename = "win")]
    Win,
    #[serde(rename = "loss")]
    Loss,
    #[serde(rename = "draw")]
    Draw,
}

impl GameResultFilter {
    /// Every value, in declaration order.
    pub const ALL: &[GameResultFilter] = &[
        GameResultFilter::Win,
        GameResultFilter::Loss,
        GameResultFilter::Draw,
    ];

    /// Its wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            GameResultFilter::Win => "win",
            GameResultFilter::Loss => "loss",
            GameResultFilter::Draw => "draw",
        }
    }
}
union!(#[doc = "Lifecycle stage of a tournament."] TournamentStatus { Created = "created", Started = "started", Finished = "finished" });
union!(#[doc = "Which app area a metered action belongs to."] UsageKind {
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
union!(#[doc = "Where a game review came from."] ReviewSource { Lichess = "lichess", Local = "local" });
union!(#[doc = "Connection state while watching a game."] WatchPhase { Connecting = "connecting", Connected = "connected", Ended = "ended", Error = "error" });
union!(#[doc = "Whether watching Lichess TV or a specific game."] WatchSource { Tv = "tv", Game = "game" });
union!(#[doc = "Time section of a broadcast list."] BroadcastSection { Active = "active", Upcoming = "upcoming", Past = "past" });
union!(#[doc = "Why engine analysis stopped."] AnalysisReason { Completed = "completed", Interrupted = "interrupted", Failed = "failed" });
union!(#[doc = "Why automatic review is paused."] ReviewPause { Battery = "battery", Engine = "engine", Online = "online", Off = "off" });
union!(#[doc = "OS permission state of the microphone."] MicrophoneStatus {
    Granted = "granted",
    Denied = "denied",
    Restricted = "restricted",
    NotDetermined = "not-determined",
    Unknown = "unknown",
});
union!(#[doc = "Progress stage of the puzzle database download."] PuzzleDbPhase {
    Downloading = "downloading",
    Importing = "importing",
    Done = "done",
    Cancelled = "cancelled",
    Failed = "failed",
});
/// Colors of one light or dark variant of an app theme, as `#rgb` / `#rrggbb`. Optional ones are derived from the rest.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThemePalette {
    /// Page background.
    pub bg: String,
    /// Main text.
    pub text: String,
    /// Accent: buttons, links, focus.
    pub primary: String,
    /// Slightly raised areas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<String>,
    /// Cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevated: Option<String>,
    /// Hover and selected backgrounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accented: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_muted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
}

/// A color theme for the whole app. A theme with only one variant uses it in both light and dark mode.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppTheme {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ThemePalette>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<ThemePalette>,
    /// Loaded from the user's themes folder rather than shipped with the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomThemeReport {
    pub themes: Vec<AppTheme>,
    /// The folder custom theme files are read from.
    pub dir: String,
    /// One line per file that could not be used.
    pub problems: Vec<String>,
}

/// Preferences consumed by headless services.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrontendPreferences {
    pub appearance: Appearance,
    pub board_theme: String,
    /// Color theme of the app itself in light mode: `kchess` (the default look), a preset, or a custom theme id.
    pub light_theme: String,
    /// The same for dark mode; the two are chosen independently.
    pub dark_theme: String,
    /// One of the piece sets shipped in `apps/gpui/assets/pieces`.
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
    /// Load the next training puzzle as soon as one is solved.
    pub puzzle_auto_next: bool,
    /// Ask before a correspondence move is sent: there it cannot be taken back.
    pub confirm_correspondence: bool,
    /// Write moves with piece symbols (♘f3) rather than letters (Nf3).
    pub figurine_notation: bool,
}

/// Every setting: the core's preferences and the desktop's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(flatten)]
    pub core: CoreSettings,
    #[serde(flatten)]
    pub frontend: FrontendPreferences,
}

/// A synced game, seen from its account's side.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessGame {
    pub id: String,
    pub account: String,
    pub created_at: i64,
    pub last_move_at: i64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub status: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub winner: Option<Color>,
    #[serde(with = "kchess_domain::color::by_name")]
    pub color: Color,
    pub opponent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opponent_rating: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_rating: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_diff: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    /// SAN moves, separated by spaces.
    pub moves: String,
    /// Only `gamePgn` reads it: list rows leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pgn: Option<String>,
}

/// A page of the synced games, filtered.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(expecting = "an object")]
pub struct GamePageQuery {
    /// One account's games (a Lichess username or `name@chess.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<GameResultFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
    /// Part of an opponent's name or an opening's; blank is no search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GamePage {
    pub games: Vec<LichessGame>,
    /// The games the filters match, on every page.
    pub total: i64,
}

/// Win, loss and draw counts.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub total: i64,
    pub win: i64,
    pub loss: i64,
    pub draw: i64,
}

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
    /// `opening`, `middlegame` or `endgame`.
    pub phase: String,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameLibraryOverview {
    /// By account, lower-cased.
    pub by_account: BTreeMap<String, GameRecord>,
    /// The connected accounts' games against each added player, lower-cased.
    pub versus: BTreeMap<String, GameRecord>,
}

/// One rating category's rating, the last rated one each day: `[year, month (0–11), day,
/// rating]`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RatingSeries {
    pub name: String,
    pub points: Vec<[i64; 4]>,
}

/// A Lichess account or added player, with what was synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub username: String,
    /// Signed in (a Lichess login is kept), rather than a player added to follow.
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    /// The player's Lichess play time when their games were last synced, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_play_time: Option<i64>,
}

/// The core's settings, the accounts and how many games are synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppData {
    pub settings: CoreSettings,
    pub accounts: Vec<Account>,
    pub game_count: i64,
}

/// A Lichess account signed in through the browser, and the data with it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ConnectedLichess {
    pub data: AppData,
    /// The account Lichess authorised.
    pub username: String,
}

/// A Chess.com player whose games are synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChesscomAccount {
    /// As Chess.com spells it.
    pub username: String,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

/// A player's profile and rating history as last saved, with no network.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedProfile {
    /// The Lichess user as Lichess sent it; null when never fetched.
    pub profile: Option<Value>,
    /// The rating history as Lichess sent it; null when never fetched.
    pub rating_history: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_fetched_at: Option<i64>,
}

/// A private message went through.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSent {
    pub sent: bool,
}

/// Health of the idle event stream that waits for challenges and pairings.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LobbyState {
    pub account: String,
    pub phase: OnlinePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: ChatRoom,
}

/* ── Players ──────────────────────────────────────────────────────────── */

/// A game in a player's record: the opponent and when.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfResult {
    pub opponent: String,
    pub opponent_rating: f64,
    pub at: String,
    pub game_id: String,
}

/// A rating and the game it came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfMark {
    pub rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PerfStreak {
    pub current: i64,
    pub best: i64,
}

/// One player's record in one rating category, as Lichess's perf page shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub perf: PerfType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deviation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
    /// Position on the leaderboard, when ranked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    /// Better than this share of players.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
    /// Lichess's game counts as it sent them (`all`, `rated`, `win`, `loss`, `draw`, `tour`,
    /// `berserk`, `opAvg`, `seconds`, `disconnects`), null when it sent none.
    pub count: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest: Option<PerfMark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest: Option<PerfMark>,
    pub best_wins: Vec<PerfResult>,
    pub worst_losses: Vec<PerfResult>,
    pub win_streak: PerfStreak,
    pub loss_streak: PerfStreak,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrosstableMatchup {
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
}

/// Two players' lifetime score against each other (and in their current match, if any).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Crosstable {
    /// Lower-cased username → points (wins plus half-points for draws).
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matchup: Option<CrosstableMatchup>,
}

/// Whether a connected account follows or blocks a player on Lichess.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LichessRelation {
    pub following: bool,
    pub blocking: bool,
}

/// One place on a Lichess leaderboard.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rating: f64,
    /// Rating change over the last games.
    pub progress: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// A player whose name starts with what was typed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSuggestion {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// One line of a player's activity or of a timeline, ready to show: `kind` picks the icon
/// (`games`, `puzzles`, `tournament`, `follow`, `study`, `team`, `post`, `simul`, `patron`,
/// `stream`, `correspondence`, `other`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub kind: String,
    pub text: String,
    /// A player the line is about, for a profile link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// A game the line is about, to open it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// The rating over the line's games or puzzles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<RatingChange>,
}

/// A rating before and after a day's games or puzzles.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct RatingChange {
    pub before: f64,
    pub after: f64,
}

/// A day of a player's Lichess activity, newest first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDay {
    /// Start of the day, epoch milliseconds.
    pub start: f64,
    pub items: Vec<ActivityItem>,
}

/// One line of the Following list's timeline (what its players did), newest first.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Start of the day, epoch milliseconds.
    pub date: f64,
    /// Who on the Following list the line is about.
    pub player: String,
    pub item: ActivityItem,
}

/// A game uploaded to Lichess, with the address of its page.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedGame {
    pub id: String,
    pub url: String,
}

/* ── Watching games ─────────────────────────────────────────────────── */

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
    /// UCI moves from `startFen`; absent until the TV feed and Lichess's delayed export line up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
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

/// What the managed engine directory holds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineManaged {
    pub installed: bool,
    pub path: String,
    /// The release tag recorded when it was downloaded, e.g. `sf_17.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The engine that would run: ready when its executable is.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub ready: bool,
    /// The executable in use (a downloaded or chosen one), or empty when the bundled engine is.
    pub path: String,
    pub bundled: bool,
    /// The Stockfish KChess downloaded, whether or not it is the one in use.
    pub managed: EngineManaged,
    /// An official native download exists for this platform and architecture.
    pub can_download: bool,
    /// Identity of the executable currently selected, including native file changes; only a
    /// ready engine has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

/// The downloaded Stockfish. `updated` is false when it already was the latest release.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstalled {
    pub path: String,
    pub version: String,
    pub updated: bool,
}

/// A followed player's Lichess ratings, of the categories they have one in.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FollowedRatings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blitz: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rapid: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classical: Option<i64>,
}

/// A player one of the connected accounts follows on Lichess.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowedUser {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub ratings: FollowedRatings,
    /// Connected accounts that follow them.
    pub followed_by: Vec<String>,
    /// Already a friend in KChess (or one of your own accounts).
    pub already_added: bool,
    /// Removed as a friend earlier; not suggested again unless asked.
    pub dismissed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingProblem {
    pub account: String,
    pub message: String,
    /// Connecting the account again grants the missing permission.
    pub needs_reconnect: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FollowingReport {
    pub users: Vec<FollowedUser>,
    pub problems: Vec<FollowingProblem>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageCell {
    pub requests: i64,
    /// Bytes received, after decompression.
    pub bytes_in: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub total: UsageCell,
    /// By the kind's wire name (`UsageKind`), in the order counted.
    pub by_kind: Keyed<UsageCell>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorage {
    pub games: i64,
    /// Approximate size of the stored games.
    pub bytes: i64,
    /// Profile and rating data cached for the account.
    pub cache_bytes: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// Username → downloads from Lichess; the empty key is traffic with no account (login, etc.).
    pub accounts: Keyed<AccountUsage>,
    /// Lower-cased username → what is kept on this computer.
    pub storage: Keyed<AccountStorage>,
    /// When counting started; absent when nothing has been counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<f64>,
    /// Size of the databases on disk.
    pub db_bytes: u64,
}

/// One puzzle, whichever source it came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Puzzle {
    pub id: String,
    /// The position the player starts from: it is the player's move, after the opponent's `lastMove`.
    pub fen: String,
    /// UCI of the opponent's move that led here (for the board highlight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    /// UCI moves, alternating player, opponent, … and ending with the player's move.
    pub solution: Vec<String>,
    #[serde(with = "js_number")]
    pub rating: f64,
    pub themes: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub plays: Option<f64>,
    /// Lichess game the position came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

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

/// Returned instead of data when the account's Lichess login lacks (or lost) the puzzle permission.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NeedsReconnect {
    pub needs_reconnect: bool,
}

/// What a call made as a Lichess account answers: its result, or `{ needsReconnect: true }` when
/// the account has no login or Lichess refused it (it must connect again).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply<T> {
    Done(T),
    NeedsReconnect,
}

impl<T: Serialize> Serialize for Reply<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Reply::Done(value) => value.serialize(serializer),
            Reply::NeedsReconnect => NeedsReconnect {
                needs_reconnect: true,
            }
            .serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Reply<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire<T> {
            Reconnect(NeedsReconnect),
            Done(T),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Reconnect(NeedsReconnect {
                needs_reconnect: true,
            }) => Ok(Reply::NeedsReconnect),
            Wire::Reconnect(_) => Err(serde::de::Error::custom("needsReconnect is only ever true")),
            Wire::Done(value) => Ok(Reply::Done(value)),
        }
    }
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
/// What part of the profile a write changed (`profile:changed`), so other windows refetch it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProfileArea {
    Studies,
    PlayedGames,
    Drills,
    Sessions,
    Tournaments,
    Repertoire,
    LichessGames,
    Accounts,
    DesktopSettings,
    Puzzles,
    VoiceHistory,
    Usage,
    Engine,
    ViewPreferences,
}

/// `alerts:owner`: the connection whose window raises system notifications now (the window
/// focused last), or none. Every other window updates silently (`docs/shared-core.md`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertOwner {
    pub connection: Option<u64>,
}

/// `profile:changed`: a write to the profile, from connection `origin` (`docs/shared-core.md`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChange {
    pub areas: Vec<ProfileArea>,
    /// The connection that wrote it; a window ignores its own writes.
    pub origin: u64,
}

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

/// One recognized word and the recognizer's confidence in it (0–1).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWord {
    pub word: String,
    #[serde(with = "js_number")]
    pub conf: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttemptInput {
    pub source: VoiceSource,
    /// The recognizer's text, as heard.
    pub heard: String,
    /// Average word confidence, 0–1.
    pub confidence: f64,
    pub words: Vec<VoiceWord>,
    pub outcome: VoiceOutcome,
    /// How KChess read it: SAN choices joined by `|`, a square, or `confirm`/`cancel`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<String>,
    /// What the player meant, when known: the square asked for, or the move they went on to play.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// The position it was said in (FEN), so the phrase can be parsed again later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// An earlier attempt at the same move or question that this one repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<i64>,
}

/// A logged phrase: its id and time, then the phrase as saved.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttempt {
    pub id: i64,
    pub at: i64,
    #[serde(flatten)]
    pub input: VoiceAttemptInput,
}

/// A logged phrase's later outcome, and what the player meant once known.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VoiceUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<VoiceOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct BestMoveOptions {
    /// Start from this position instead of the standard one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// Think for this many milliseconds instead of the level's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movetime: Option<u64>,
    /// Chess960 rules: castling is king-takes-rook in UCI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chess960: Option<bool>,
    /// The Lichess key of a variant with rules of its own; Fairy-Stockfish plays it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl From<&kchess_domain::games::computer::SearchOptions> for BestMoveOptions {
    fn from(options: &kchess_domain::games::computer::SearchOptions) -> BestMoveOptions {
        BestMoveOptions {
            fen: options.fen.clone(),
            movetime: options.movetime.map(|ms| ms as u64),
            chess960: options.chess960.then_some(true),
            variant: options.variant.map(|variant| variant.key().to_string()),
        }
    }
}

/// Ask the analysis engine to study one position until stopped or a limit is reached.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct AnalysisRequest {
    pub fen: String,
    /// How many of the best lines to report (MultiPV).
    pub lines: u32,
    /// Keep searching until stopped instead of stopping at a sensible depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infinite: Option<bool>,
    /// Repetition context: root plus moves must produce fen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_fen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<u64>,
    /// The Lichess variant key (`crazyhouse`, `atomic`…) when the position is not standard chess;
    /// those go to Fairy-Stockfish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

/// One engine line. Scores are from White's point of view, like Lichess shows them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineLine {
    /// 1 for the best line, 2 for the next best…
    pub rank: u32,
    pub depth: f64,
    /// Centipawns, when there is no forced mate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    /// Moves to mate: positive when White mates, negative when Black does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// Principal variation as UCI moves.
    pub pv: Vec<String>,
}

/// The payload of `engine:analysis`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUpdate {
    /// The request this belongs to; a frontend drops updates of superseded ones.
    pub id: u64,
    pub fen: String,
    pub depth: f64,
    /// Nodes per second, for the engine's status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nps: Option<f64>,
    pub lines: Vec<EngineLine>,
    /// The search finished (limit reached, or stopped) and no more updates follow.
    pub done: bool,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AnalysisReason>,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/* ── ChatGPT ──────────────────────────────────────────────────────── */

wire_enum!(#[doc = "How hard the model thinks; `auto` leaves it to the model."] AiEffort, AI_EFFORTS {
    Auto = "auto",
    Low = "low",
    Medium = "medium",
    High = "high",
});

union!(#[doc = "Why an AI request came to nothing; each asks something different of the person: `connect`, sign in again (the token was refused for good); `limit`, the allowance ran out; `ineligible`, this plan, workspace or region cannot use it; `unsupported`, the model refuses the effort; `unavailable`, temporary (the credentials stay); `unverified`, the answer named something outside the facts, twice."] AiProblem {
    Connect = "connect",
    Limit = "limit",
    Ineligible = "ineligible",
    Unsupported = "unsupported",
    Unavailable = "unavailable",
    Unverified = "unverified",
});

union!(#[doc = "What an explanation is about: a played move, a hint, a puzzle's solution or a wrong puzzle move."] AiKind {
    Move = "move",
    Nudge = "nudge",
    Solution = "solution",
    Wrong = "wrong",
});

/// The ChatGPT connection, and the model and effort every AI feature uses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiConnection {
    pub connected: bool,
    pub registered: bool,
    /// `null` when no account is signed in.
    pub email: Option<String>,
    /// `null`: the account's first model.
    pub model: Option<String>,
    pub effort: AiEffort,
    /// The chosen model refused the effort, so requests leave it out.
    pub effort_refused: bool,
}

impl Default for AiConnection {
    fn default() -> AiConnection {
        AiConnection {
            connected: false,
            registered: false,
            email: None,
            model: None,
            effort: AiEffort::Auto,
            effort_refused: false,
        }
    }
}

/// A model on offer, as the picker shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiModel {
    pub slug: String,
    pub name: String,
}

/// The models the account may use, or why they could not be listed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum AiModelList {
    Listed { models: Vec<AiModel> },
    Refused { problem: AiProblem },
}

/// What `aiExplain` is asked to explain.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    pub kind: AiKind,
    pub fen: String,
    /// The move played (UCI): `move` and `wrong` need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played: Option<String>,
    /// The line from `fen`: a puzzle's solution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<String>,
    /// The engine's line instead of `played`, and its reply to `played`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub best: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refutation: Vec<String>,
    /// Moves to mate before and after `played`, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate_before: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate_after: Option<f64>,
}

/// An explanation, or why there is none.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum AiExplanation {
    Told { text: String },
    Refused { problem: AiProblem },
}

/* ── Game review ───────────────────────────────────────────────────── */

/// One position of a reviewed game: its score from White's side and the engine's choice there.
/// A checkmated position is `mate: 0` (the side to move has lost).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// The engine's move in this position (UCI), and the line it expects after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pv: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
}

/// Lichess's accuracy figures, as Lichess sent them (`87`, `64.5`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAccuracy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<serde_json::Number>,
}

/// A game's review as stored: the raw scores; labels and accuracy are worked out from them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredReview {
    /// The same moves from the same start share one review.
    pub key: String,
    pub fen: String,
    /// UCI, from `fen`.
    pub moves: Vec<String>,
    pub source: ReviewSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Index 0 is the starting position, index i the position after move i; null until analysed.
    pub evals: Vec<Option<ReviewEval>>,
    /// Depth the local analysis aims for; 0 for Lichess's analysis.
    pub depth: u32,
    pub complete: bool,
    pub updated_at: i64,
    /// The Lichess game it belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// Lichess's own label for each move (index i is move i+1), when Lichess did the analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgments: Option<Vec<Option<Judgment>>>,
    /// Lichess's own accuracy figures, when Lichess did the analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<ReviewAccuracy>,
    /// The Lichess key of a variant game's variant, which Fairy-Stockfish reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// What another release stored beside these, kept as it was.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

/// One player's side of a review. Figures keep the form they were stored in (`100` when read
/// back, `100.0` when just worked out).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSide {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<serde_json::Number>,
    /// Average centipawn loss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acpl: Option<serde_json::Number>,
    pub inaccuracy: u32,
    pub mistake: u32,
    pub blunder: u32,
}

/// What the game list shows for a reviewed game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub key: String,
    pub source: ReviewSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    pub complete: bool,
    pub white: ReviewSide,
    pub black: ReviewSide,
}

/// Review a game: its moves from its position, and the Lichess game it is when it is one.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct ReviewRequest {
    pub fen: String,
    pub moves: Vec<String>,
    /// The Lichess key of a variant game's variant (`crazyhouse`, `atomic`…), which
    /// Fairy-Stockfish reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// A Lichess game: its own analysis is fetched first when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCurrent {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    pub done: usize,
    pub total: usize,
    pub background: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFailure {
    pub key: String,
    pub message: String,
}

/// Progress of the review queue, sent whenever it changes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatus {
    /// The review being worked on, with how many positions are done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<ReviewCurrent>,
    /// Reviews waiting, asked for and automatic.
    pub waiting: usize,
    /// Why automatic reviews are on hold, when they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<ReviewPause>,
    /// The last review asked for that could not be done, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<ReviewFailure>,
}

/// The payload of `review:update`: the review as stored, and its summary.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUpdate {
    pub review: StoredReview,
    pub summary: ReviewSummary,
}

/// Filters of the opening explorer; each database uses the ones it understands.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LookupOptions {
    /// Lichess and player databases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speeds: Option<Vec<ExplorerSpeed>>,
    /// Lichess database: rating groups (each is the lower bound of its band).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratings: Option<Vec<u32>>,
    /// Player database: whose games, and with which colour.
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
    /// Games since this month (`YYYY-MM`; masters use a year).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
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

/// A field that is absent (`None`), null (`Some(None)`) or a value.
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessStudy {
    pub id: String,
    pub name: String,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudySyncRequest {
    pub account: String,
    pub study_id: String,
    pub baseline: String,
    pub pgn: String,
}

/// Where `exportToLichessStudy` added the game: the study's id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StudyExport {
    pub id: String,
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

/* ── Online play, watching and tournaments, as the core answers them ────── */

pub use kchess_domain::games::session::{
    AnalysisSession, ComputerSession, LocalSession, SessionDocument, Sessions,
};
pub use kchess_domain::keyed::Keyed;
pub use kchess_domain::library::{Drill, PlayedGame, Study, StudyChapter, StudyCloud};
pub use kchess_domain::records::challenge::{
    ChallengeDirection, ChallengeInfo, PlayerRef, TimeControl as ChallengeTimeControl,
};
pub use kchess_domain::training::repetition::{RepertoireMisses, RepertoireSchedule};

/// What to play online and how: a public seek, or a challenge to `target`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineOptions {
    /// Minutes on the clock; ignored for correspondence games (`days`).
    pub minutes: u32,
    /// Seconds added per move.
    pub increment: u32,
    /// Correspondence: days per move instead of a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// Standard when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    /// A direct challenge from this position (standard or Chess960 rules).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    pub color: ChallengeColor,
    /// Rated games change the Lichess rating; casual ones do not.
    pub rated: bool,
    /// The player challenged; a public seek without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Which connected account plays; the first connected one without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// What `startOnline` began: a challenge (its id and address), a public seek, or a
/// correspondence game.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondence: Option<bool>,
}

/// The game in progress `resumeOnline` put back on the board, and its account.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Resumed {
    pub id: String,
    pub account: String,
}

/// A game a connected account is playing, real-time or correspondence.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    /// `white` or `black`, as Lichess says (empty when it does not).
    pub color: String,
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    /// Seconds left on your clock (or for your correspondence move), when Lichess says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<Number>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tournament_id: Option<String>,
    /// Whether the Board API can play it (`compat.board`); a game it cannot is watched instead.
    #[serde(default = "playable_by_default")]
    pub playable: bool,
}

fn playable_by_default() -> bool {
    true
}

/// Lichess's view of one user's connection. `signal` runs 1 (poor, lag > 500 ms) to 4 (great,
/// lag < 150 ms).
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<Number>,
    /// The game they are playing, when Lichess says (watchable from the Watch page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    /// Lower-cased username → status; users Lichess did not report are absent.
    pub users: BTreeMap<String, UserPresence>,
    /// Round trip of the status request itself, i.e. this computer's latency to Lichess.
    pub latency_ms: u64,
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

/* ── The library, puzzles and preferences ────────────────────────────── */

/// A number as JavaScript writes it: a whole one without a fraction (`3`, not `3.0`). For numbers
/// the core keeps as written by `JSON.stringify` (stored documents, Lichess's answers).
pub mod js_number {
    use serde::{Deserialize, Deserializer, Serializer};

    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

    pub fn serialize<S: Serializer>(n: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if n.fract() == 0.0 && n.abs() <= MAX_SAFE {
            serializer.serialize_i64(*n as i64)
        } else {
            serializer.serialize_f64(*n)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        f64::deserialize(deserializer)
    }

    /// An optional one; `None` is `null` unless the field is skipped.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(n: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
            match n {
                Some(n) => super::serialize(n, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<f64>, D::Error> {
            Option::<f64>::deserialize(deserializer)
        }
    }
}

/// A study chapter, as Lichess sends it or to save: its name and PGN.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(expecting = "an object")]
pub struct ChapterInput {
    pub name: String,
    pub pgn: String,
}

/// A Lichess study a local one is a copy of.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(expecting = "an object")]
pub struct RemoteStudy {
    pub id: String,
    pub name: String,
}

/// One change to the local study library, by its `op`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StudyCommand {
    /// A study of one chapter, or (with `id`) one chapter's moves of a stored study.
    Save {
        name: String,
        pgn: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
    },
    /// A study of these chapters, new or (with `id`) replacing a stored one.
    SaveChapters {
        name: String,
        chapters: Vec<ChapterInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    AddChapter {
        id: String,
        name: String,
    },
    RenameChapter {
        id: String,
        chapter_id: String,
        name: String,
    },
    DuplicateChapter {
        id: String,
        chapter_id: String,
    },
    RemoveChapter {
        id: String,
        chapter_id: String,
    },
    /// A chapter to its new place.
    MoveChapter {
        id: String,
        chapter_id: String,
        index: u32,
    },
    /// Undo of a removed chapter: it goes back at its place.
    RestoreChapter {
        id: String,
        chapter: StudyChapter,
        index: u32,
    },
    /// The study is a copy of a Lichess study, as downloaded (`baseline`).
    MarkCloud {
        id: String,
        account: String,
        remote_id: String,
        baseline: String,
    },
    /// A Lichess study kept as a local one.
    Offline {
        account: String,
        remote: RemoteStudy,
        chapters: Vec<ChapterInput>,
    },
    Remove {
        id: String,
    },
    /// Undo of a removed study: the study as `remove` answered it.
    Restore {
        study: Value,
    },
    Rename {
        id: String,
        name: String,
    },
    Duplicate {
        id: String,
    },
}

impl StudyCommand {
    /// The stored study the command changes, when it names one.
    pub fn study_id(&self) -> Option<&str> {
        match self {
            StudyCommand::Save { id, .. } | StudyCommand::SaveChapters { id, .. } => id.as_deref(),
            StudyCommand::AddChapter { id, .. }
            | StudyCommand::RenameChapter { id, .. }
            | StudyCommand::DuplicateChapter { id, .. }
            | StudyCommand::RemoveChapter { id, .. }
            | StudyCommand::MoveChapter { id, .. }
            | StudyCommand::RestoreChapter { id, .. }
            | StudyCommand::MarkCloud { id, .. }
            | StudyCommand::Remove { id }
            | StudyCommand::Rename { id, .. }
            | StudyCommand::Duplicate { id } => Some(id),
            StudyCommand::Restore { study } => study.get("id").and_then(Value::as_str),
            StudyCommand::Offline { .. } => None,
        }
    }

    /// The chapter the command changes, when it names one.
    pub fn chapter_id(&self) -> Option<&str> {
        match self {
            StudyCommand::Save { chapter_id, .. } => chapter_id.as_deref(),
            StudyCommand::RenameChapter { chapter_id, .. }
            | StudyCommand::DuplicateChapter { chapter_id, .. }
            | StudyCommand::RemoveChapter { chapter_id, .. }
            | StudyCommand::MoveChapter { chapter_id, .. } => Some(chapter_id),
            StudyCommand::RestoreChapter { chapter, .. } => Some(&chapter.id),
            _ => None,
        }
    }
}

/// What a study command did: the study or chapter it made or kept (`id`), a cloud copy kept
/// apart (`conflict`), the study removed, and the studies it changed (all of them, unless the
/// command was incremental: then only the changed ones, with every study's id in `order`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StudyCommandResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Study>,
    pub studies: Vec<Study>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

/// The local library as saved. Its entries are the stored documents, read with the domain's
/// codecs: studies, played games, mistake drills, sessions by kind, tournaments joined from
/// KChess, the repertoires' missed positions and their review schedule.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub studies: Vec<Study>,
    pub games: Vec<PlayedGame>,
    pub mistakes: Vec<Drill>,
    pub sessions: Sessions,
    pub joined_tournaments: Vec<JoinedTournament>,
    pub repertoire_misses: RepertoireMisses,
    pub repertoire_schedule: RepertoireSchedule,
    /// Why the studies could not be read (they are kept, and shown as none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_error: Option<String>,
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

/// The drills after adding a review's mistakes; `added` counts the new ones.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MistakesAdded {
    pub added: u32,
    pub items: Vec<Drill>,
}

/// A reviewed game with positions in practice, and its game when known: a synced Lichess game
/// (`gameId`, `account`), or one played here (the side named "You" is `player`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MistakeGame {
    /// The review's key.
    pub key: String,
    pub fen: String,
    pub moves: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub player: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(with = "js_number")]
    pub played_at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Accuracy and time use, which the recorded reports leave out, keep the numbers the core
    /// wrote by hand: averages whole when they are, per-game rates always with a fraction.
    #[test]
    fn insights_accuracy_and_time_use_keep_their_bytes() {
        let record = r#"{"total":1,"win":1,"loss":0,"draw":0}"#;
        let text = format!(
            concat!(
                r#"{{"accuracy":{{"games":2,"average":87.5,"acpl":30,"#,
                r#""perGame":{{"inaccuracy":1.0,"mistake":0.5,"blunder":0.0}}}},"#,
                r#""timeUse":{{"games":1,"trouble":{r},"spare":{r},"#,
                r#""phases":[{{"phase":"opening","moves":3,"seconds":2,"share":12.5}},"#,
                r#"{{"phase":"endgame","moves":1,"seconds":0.5}}],"#,
                r#""clockAt":[{{"move":20,"games":1,"player":50,"opponent":41.5}}],"#,
                r#""blunders":{{"games":1,"short":{{"moves":1,"blunders":0}},"#,
                r#""spare":{{"moves":2,"blunders":1}}}}}}}}"#
            ),
            r = record
        );
        #[derive(Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Parts {
            accuracy: AccuracyReport,
            time_use: TimeUseReport,
        }
        let parts: Parts = serde_json::from_str(&text).unwrap();
        assert_eq!(serde_json::to_string(&parts).unwrap(), text);
    }

    /// A joined tournament keeps the bytes the core wrote by hand: its end a whole number.
    #[test]
    fn a_joined_tournament_keeps_its_bytes() {
        for text in [
            r#"[{"system":"arena","id":"abcd","account":"Alice","name":"Hourly","until":1700000000000}]"#,
            r#"[{"system":"swiss","id":"efgh","account":"bob","name":"Weekly","until":1.5}]"#,
        ] {
            let joined: Vec<JoinedTournament> = serde_json::from_str(text).unwrap();
            assert_eq!(serde_json::to_string(&joined).unwrap(), text);
        }
    }

    /// A game saved before `playable` existed is one the Board API can play.
    #[test]
    fn an_ongoing_game_without_playable_is_playable() {
        let game: OngoingGame = serde_json::from_value(serde_json::json!({
            "gameId": "abcd1234",
            "account": "me",
            "opponent": { "name": "them" },
            "color": "white",
            "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
            "isMyTurn": true,
            "variant": "standard",
            "speed": "blitz",
            "rated": false,
        }))
        .unwrap();
        assert!(game.playable);
    }
}
