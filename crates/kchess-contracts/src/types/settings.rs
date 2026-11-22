//! Settings: the core's, the desktop's and the themes they name.

use super::*;

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

wire_enum!(#[doc = "What closing KChess's last window does: `default` as the platform's apps do (keep running on macOS, quit elsewhere), `background` keeps it running for notifications, `quit` quits."] CloseAction, CLOSE_ACTIONS {
    Default = "default",
    Background = "background",
    Quit = "quit",
});

wire_enum!(#[doc = "Which games are reviewed automatically."] ReviewAuto, REVIEW_AUTO {
    Off = "off",
    Recent = "recent",
    All = "all",
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
    /// Start KChess when the person logs in to the computer: in the background, without a
    /// window, when closing the window keeps it running.
    pub open_at_login: bool,
    /// What closing the last window does.
    pub close_action: CloseAction,
    /// Show KChess's icon and menu in the menu bar (macOS) or the system tray (Windows, Linux).
    /// Off, closing the window quits on Windows and Linux, where nothing else would show it runs.
    pub show_tray_icon: bool,
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
