//! The shapes every frontend exchanges with the core.
//!
//! Conventions:
//! - Field names are camelCase on the wire. `Option` fields are omitted when absent unless a
//!   field says otherwise with its serde attributes.
//! - A string-literal union is a Rust enum. `wire_enum!` also emits the array of wire values
//!   under the constant's name (`APPEARANCES`, ...); `union!` does not.
//! - A flattened parent is `#[serde(flatten)]`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Variant;
use kchess_domain::color::Color;

/// A string-literal union.
macro_rules! union {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant,)+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &[$name] = &[$($name::$variant),+];

            /// Its wire value.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $wire,)+
                }
            }
        }
    };
}

/// A string-literal union with the array of its wire values.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident, $konst:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        union!($(#[$meta])* $name { $($variant = $wire),+ });
        /// The wire values of the union, in declaration order.
        pub const $konst: &[&str] = &[$($wire),+];
    };
}

/* ── Choices and settings ───────────────────────────────────────────── */

wire_enum!(#[doc = "Which color scheme the app uses."] Appearance, APPEARANCES {
    System = "system",
    Light = "light",
    Dark = "dark",
});
wire_enum!(#[doc = "Where board coordinates are shown."] CoordinateMode, COORDINATE_MODES {
    None = "none",
    Inside = "inside",
    Outside = "outside",
});
wire_enum!(#[doc = "Built-in engine strength, from beginner to max."] EngineLevel, ENGINE_LEVELS {
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
wire_enum!(#[doc = "What a desktop notification is about; each kind (except `test`) has its own switch in Settings."] NotificationKind, NOTIFICATION_KINDS {
    OpponentMove = "opponentMove",
    LowTime = "lowTime",
    GameEvents = "gameEvents",
    ComputerMove = "computerMove",
    Challenge = "challenge",
    Test = "test",
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
wire_enum!(#[doc = "Which chat room a message belongs to."] ChatRoom, CHAT_ROOMS {
    Player = "player",
    Spectator = "spectator",
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
wire_enum!(#[doc = "What a training attempt counts as, each move scored by its first try: every move found (`solved`, or `hinted`"] #[doc = "after a hint), `partial` (some found, some wrong), `missed` (none found) or `revealed` (the solution shown"] #[doc = "before any wrong move). Local only: Lichess results stay a win or a loss."] PuzzleAttemptOutcome, PUZZLE_ATTEMPT_OUTCOMES {
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
wire_enum!(#[doc = "Severity of a move mistake in review."] Judgment, JUDGMENTS {
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
wire_enum!(#[doc = "Format for exporting a game."] ExportKind, EXPORT_KINDS {
    Gif = "gif",
    Png = "png",
    Pgn = "pgn",
});
wire_enum!(#[doc = "What a connected account can do about a player on Lichess."] RelationAction, RELATION_ACTIONS {
    Follow = "follow",
    Unfollow = "unfollow",
    Block = "block",
    Unblock = "unblock",
});

union!(#[doc = "Tournament pairing system."] TournamentSystem { Arena = "arena", Swiss = "swiss" });
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
union!(#[doc = "Whether a challenge was received or sent."] ChallengeDirection { In = "in", Out = "out" });
union!(#[doc = "Which online stream a message arrived on."] Lane { Events = "events", Game = "game", Seek = "seek" });
union!(#[doc = "Connection state of the online play session."] OnlinePhase {
    Checking = "checking",
    Connecting = "connecting",
    Connected = "connected",
    Reconnecting = "reconnecting",
    Disconnected = "disconnected",
    AuthRequired = "auth-required",
    Idle = "idle",
});
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
union!(#[doc = "Progress stage of the voice model download."] VoiceModelPhase { Checking = "checking", Downloading = "downloading", Preparing = "preparing" });

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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppTheme {
    pub id: String,
    pub name: String,
    pub light: Option<ThemePalette>,
    pub dark: Option<ThemePalette>,
    /// Loaded from the user's themes folder rather than shipped with the app.
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Streaks {
    pub longest_win: i64,
    pub longest_loss: i64,
    /// Positive for wins in a row, negative for losses.
    pub current: i64,
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

/// One player's side of a review.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSide {
    pub accuracy: Option<f64>,
    /// Average centipawn loss.
    pub acpl: Option<f64>,
    pub inaccuracy: f64,
    pub mistake: f64,
    pub blunder: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineConnection {
    pub session: f64,
    pub account: String,
    pub game_id: String,
    pub lane: Lane,
    pub phase: OnlinePhase,
    pub message: Option<String>,
}

/// A player as a challenge or ongoing game names them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRef {
    pub name: String,
    pub rating: Option<f64>,
    pub title: Option<String>,
    pub provisional: Option<bool>,
    pub online: Option<bool>,
}

/// Time control of a challenge.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LobbyState {
    pub account: String,
    pub phase: OnlinePhase,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: ChatRoom,
}

/// A game a connected account is playing, real-time or correspondence.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    #[serde(with = "kchess_domain::color::by_name")]
    pub color: Color,
    pub fen: String,
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    /// Seconds left on your clock (or for your correspondence move), when Lichess says.
    pub seconds_left: Option<f64>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    pub tournament_id: Option<String>,
    /// Whether the Board API can play it (`compat.board`); a game it cannot is watched instead.
    #[serde(default = "playable_by_default")]
    pub playable: bool,
}

fn playable_by_default() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentClock {
    /// Seconds and seconds per move.
    pub limit: f64,
    pub increment: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewArena {
    /// Empty for a random Grandmaster name.
    pub name: Option<String>,
    pub clock_time: f64,
    pub clock_increment: f64,
    pub minutes: f64,
    /// Minutes from now, unless `startDate` is set.
    pub wait_minutes: Option<f64>,
    pub start_date: Option<f64>,
    pub variant: String,
    pub rated: bool,
    /// An entry code players must type to join.
    pub password: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentPlayer {
    pub name: String,
    pub rating: Option<f64>,
    pub rank: Option<f64>,
}

/// A game between two tournament players, with their current ranks.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentGame {
    pub id: String,
    pub white: TournamentPlayer,
    pub black: TournamentPlayer,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentMe {
    pub rank: Option<f64>,
    pub withdraw: Option<bool>,
    pub game_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentCondition {
    pub condition: String,
    pub verdict: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentVerdicts {
    pub accepted: bool,
    pub list: Vec<TournamentCondition>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentClocks {
    pub white: f64,
    pub black: f64,
}

/// The top game being played now in an arena.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentFeatured {
    #[serde(flatten)]
    pub game: TournamentGame,
    pub fen: String,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
    pub last_move: Option<String>,
    /// Seconds left on each clock when Lichess sent it.
    pub clocks: Option<TournamentClocks>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentPodium {
    pub name: String,
    pub rank: f64,
    pub rating: Option<f64>,
    pub score: Option<f64>,
    pub performance: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentDetail {
    #[serde(flatten)]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TournamentList {
    pub arenas: Vec<TournamentSummary>,
    pub swiss: Vec<TournamentSummary>,
    /// Teams of the account whose Swiss events could not be read, and other notes.
    pub problems: Vec<String>,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchPlayer {
    pub name: String,
    pub title: Option<String>,
    pub rating: Option<f64>,
}

/// Connection health of the TV/game feed, scoped to its watch session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchState {
    pub session: f64,
    pub phase: WatchPhase,
    pub message: Option<String>,
    /// The kind of failure of an `error` phase (`offline`, `refused`, `notFound`…).
    #[serde(default)]
    pub kind: Option<String>,
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
    pub last_move: Option<String>,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
    pub status: Option<String>,
    #[serde(default, with = "kchess_domain::color::option_by_name")]
    pub winner: Option<Color>,
}

/// One position of a game being watched (TV or a game by id).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchFrame {
    /// Which `watch` call it belongs to; frames of an earlier one are stale.
    pub session: f64,
    pub source: WatchSource,
    pub channel: Option<String>,
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    #[serde(with = "kchess_domain::color::by_name")]
    pub orientation: Color,
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
    #[serde(default, with = "kchess_domain::color::option_by_name")]
    pub winner: Option<Color>,
    pub finished: bool,
    /// FEN before the first move, once the move list is known.
    pub start_fen: Option<String>,
    /// UCI moves from `startFen`; absent until the TV feed and Lichess's delayed export line up.
    pub moves: Option<Vec<String>>,
    /// Games this channel showed earlier in this session, newest first.
    pub previous: Option<Vec<TvPastGame>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TvChannel {
    pub key: String,
    pub label: String,
    pub game_id: Option<String>,
    pub player: Option<WatchPlayer>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRoundRef {
    pub id: String,
    pub name: String,
    pub ongoing: bool,
    pub finished: bool,
    pub starts_at: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastTourDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub rounds: Vec<BroadcastRoundRef>,
    pub default_round_id: Option<String>,
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
    pub last_move: Option<String>,
    /// Clocks in seconds, from the last comments of each side.
    pub white_clock: Option<f64>,
    pub black_clock: Option<f64>,
    pub ongoing: bool,
    /// The game as PGN, for the analysis board.
    pub pgn: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastUpdate {
    pub session: f64,
    pub round_id: String,
    /// Games that changed (all of them at first).
    pub games: Vec<BroadcastGame>,
    /// The feed ended (the round is over or the connection dropped).
    pub ended: Option<bool>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineManaged {
    pub installed: bool,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineOptions {
    /// Ignored for correspondence games (`days`).
    pub minutes: f64,
    pub increment: f64,
    /// Correspondence: days per move instead of a clock.
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageCell {
    pub requests: f64,
    /// Bytes received, after decompression.
    pub bytes_in: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub total: UsageCell,
    pub by_kind: BTreeMap<UsageKind, UsageCell>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorage {
    pub games: f64,
    /// Approximate size of the stored games.
    pub bytes: f64,
    /// Profile and rating data cached for the account.
    pub cache_bytes: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    pub signal: Option<f64>,
    /// The game they are playing, when Lichess says (watchable from the Watch page).
    pub playing_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    /// Lower-cased username → status; users Lichess did not report are absent.
    pub users: BTreeMap<String, UserPresence>,
    /// Round trip of the status request itself, i.e. this computer's latency to Lichess.
    pub latency_ms: f64,
}

/// One puzzle, whichever source it came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleGlicko {
    pub rating: Option<f64>,
    pub deviation: Option<f64>,
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
    #[serde(default, with = "kchess_domain::color::option_by_name")]
    pub color: Option<Color>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDraw {
    /// Absent when Lichess has no puzzle left for this theme and difficulty.
    pub puzzle: Option<Puzzle>,
    /// The account's current puzzle rating, when Lichess reports it.
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
    /// Rating change Lichess applied; absent for unrated results.
    pub rating_diff: Option<f64>,
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
    /// When it was played, in ms since the epoch.
    pub date: f64,
    pub win: bool,
    pub puzzle: Puzzle,
}

/// A move the player tried first at one of their moves of a puzzle, instead of the solution's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleWrongMove {
    /// The index in the solution of the move it was played instead of.
    pub index: f64,
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
    pub found: f64,
    pub wrong: f64,
    pub total: f64,
    /// The local calendar day (`YYYY-MM-DD`) it was played on.
    pub day: String,
    /// What was played at each wrong first try.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrong_moves: Option<Vec<PuzzleWrongMove>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleAttempt {
    pub id: f64,
    /// When it was played, in ms since the epoch.
    pub at: f64,
    pub day: String,
    pub puzzle: Puzzle,
    pub mode: PuzzleTrainMode,
    pub retry: bool,
    pub outcome: PuzzleAttemptOutcome,
    pub hinted: bool,
    /// Player moves right at the first try (all of them for a solve), of `total`.
    pub found: f64,
    /// Player moves wrong at the first try.
    pub wrong: f64,
    pub total: f64,
    /// What was played at each wrong first try, when the log kept it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrong_moves: Option<Vec<PuzzleWrongMove>>,
}

/// Attempts by outcome, and player moves found of those tried (found plus each ending wrong move).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleTotals {
    pub attempts: f64,
    pub solved: f64,
    pub hinted: f64,
    pub partial: f64,
    pub missed: f64,
    pub revealed: f64,
    pub moves_found: f64,
    pub moves_tried: f64,
}

/// The local attempt log, summarized. Retries are listed but not counted.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleProgress {
    /// Newest first.
    pub recent: Vec<PuzzleAttempt>,
    /// Consecutive local days with a solved puzzle, ending today (or yesterday until today has one).
    pub day_streak: f64,
    pub best_day_streak: f64,
    pub today: PuzzleTotals,
    pub all: PuzzleTotals,
}

/// Which local day is today, and how many recent attempts to list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleProgressQuery {
    pub today: String,
    pub limit: f64,
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
    pub connection: Option<f64>,
}

/// `profile:changed`: a write to the profile, from connection `origin` (`docs/shared-core.md`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChange {
    pub areas: Vec<ProfileArea>,
    /// The connection that wrote it; a window ignores its own writes.
    pub origin: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDbStatus {
    pub installed: bool,
    pub count: f64,
    /// Approximate size on disk.
    pub bytes: f64,
    pub imported_at: Option<f64>,
    /// True while a download runs; its progress arrives as events.
    pub busy: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalPuzzleQuery {
    pub theme: Option<String>,
    pub min_rating: Option<f64>,
    pub max_rating: Option<f64>,
    pub count: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalLadderQuery {
    pub from: f64,
    pub to: f64,
    pub count: f64,
}

/// A run's score and details, as the game saves it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunInput {
    pub kind: RunKind,
    /// Which flavour of the kind (`3min`, `find`, `kq-vs-k`, …).
    pub variant: String,
    pub score: f64,
    /// Extra numbers and words shown with the score (accuracy, combo, …).
    pub detail: BTreeMap<String, Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    #[serde(flatten)]
    pub input: RunInput,
    pub id: f64,
    pub played_at: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunBest {
    pub score: f64,
    pub played_at: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
    pub conf: f64,
}

/// A finished utterance: its text and each word heard.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRecognized {
    pub text: String,
    pub words: Vec<VoiceWord>,
}

/// What a chunk of audio produced: the words heard so far, or a finished utterance.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceHeard {
    pub partial: Option<String>,
    pub result: Option<VoiceRecognized>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceModelProgress {
    pub phase: VoiceModelPhase,
    pub received: f64,
    pub total: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceModelStatus {
    pub installed: bool,
    pub bytes: f64,
    pub busy: bool,
    pub progress: Option<VoiceModelProgress>,
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
    pub parsed: Option<String>,
    /// What the player meant, when known: the square asked for, or the move they went on to play.
    pub expected: Option<String>,
    /// The position it was said in (FEN), so the phrase can be parsed again later.
    pub fen: Option<String>,
    /// An earlier attempt at the same move or question that this one repeats.
    pub retry_of: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttempt {
    #[serde(flatten)]
    pub input: VoiceAttemptInput,
    pub id: f64,
    pub at: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BestMoveOptions {
    /// Start from this position instead of the standard one.
    pub fen: Option<String>,
    /// Think for this many milliseconds instead of the level's default.
    pub movetime: Option<f64>,
    /// Chess960 rules: castling is king-takes-rook in UCI.
    pub chess960: Option<bool>,
}

/// Ask the analysis engine to study one position until stopped or a limit is reached.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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
    /// The Lichess variant key (`crazyhouse`, `atomic`…) when the position is not standard chess;
    /// those go to Fairy-Stockfish.
    pub variant: Option<String>,
}

/// One engine line. Scores are from White's point of view, like Lichess shows them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUpdate {
    /// The request this belongs to; a frontend drops updates of superseded ones.
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEval {
    pub cp: Option<f64>,
    pub mate: Option<f64>,
    /// The engine's move in this position (UCI), and the line it expects after it.
    pub best: Option<String>,
    pub pv: Option<Vec<String>>,
    pub depth: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAccuracy {
    pub white: Option<f64>,
    pub black: Option<f64>,
}

/// A game's review as stored: the raw scores; labels and accuracy are worked out from them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub key: String,
    pub source: ReviewSource,
    pub engine: Option<String>,
    pub complete: bool,
    pub white: ReviewSide,
    pub black: ReviewSide,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub fen: String,
    pub moves: Vec<String>,
    /// A Lichess game: its own analysis is fetched first when it has one.
    pub game_id: Option<String>,
    pub account: Option<String>,
    /// The Lichess key of a variant game's variant (`crazyhouse`, `atomic`…), which
    /// Fairy-Stockfish reviews.
    pub variant: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCurrent {
    pub key: String,
    pub game_id: Option<String>,
    pub done: f64,
    pub total: f64,
    pub background: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFailure {
    pub key: String,
    pub message: String,
}

/// Progress of the review queue, sent whenever it changes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessStudyChapter {
    pub name: String,
    pub pgn: String,
}

/// A cloud evaluation: lines from White's point of view, like the local engine's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudEval {
    pub fen: String,
    pub depth: f64,
    pub knodes: f64,
    pub lines: Vec<EngineLine>,
}

/// A paragraph, or a section heading, of an opening description.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionBlock {
    pub heading: Option<bool>,
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
    pub fetched_at: f64,
}

/// One month of a line's popularity: of the Lichess games at the position before its last move,
/// the share (0–1) that went on with it, and how many games that was.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PopularityMonth {
    /// `YYYY-MM`.
    pub month: String,
    pub share: f64,
    pub games: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
