//! The yes/no settings, each described once: its Settings row, its Quick Pick entry and the
//! words that find its category.

use kchess_contracts::types::Settings;

use super::Section;

pub(crate) struct Toggle {
    /// The switch's element id.
    pub id: &'static str,
    pub title: &'static str,
    pub hint: Option<&'static str>,
    pub section: Section,
    pub on: fn(&Settings) -> bool,
    pub set: fn(&mut Settings, bool),
    /// Offered in Quick Pick as well.
    pub quick: Option<Quick>,
}

/// How Quick Pick lists a toggle: its id (kept in recent entries), a shorter name where Quick
/// Pick has one, its icon and other words for it.
pub(crate) struct Quick {
    pub id: &'static str,
    pub label: Option<&'static str>,
    pub icon: &'static str,
    pub keywords: &'static str,
    /// A setting of the page in view (Zen mode), offered among its commands too where it applies,
    /// with the key that runs it.
    pub page_key: Option<fn(&gpui_kit::App) -> Option<String>>,
}

impl Toggle {
    pub fn label(&self) -> &'static str {
        self.quick
            .as_ref()
            .and_then(|quick| quick.label)
            .unwrap_or(self.title)
    }
}

/// The toggle with this switch id.
pub(crate) fn toggle(id: &str) -> &'static Toggle {
    TOGGLES
        .iter()
        .find(|toggle| toggle.id == id)
        .unwrap_or_else(|| panic!("no setting toggle {id:?}"))
}

/// The titles of a category's toggles, which find it in Quick Pick.
pub(crate) fn titles(section: Section) -> impl Iterator<Item = &'static str> {
    TOGGLES
        .iter()
        .filter(move |toggle| toggle.section == section)
        .map(|toggle| toggle.title)
}

const fn quick(id: &'static str, icon: &'static str, keywords: &'static str) -> Option<Quick> {
    Some(Quick {
        id,
        label: None,
        icon,
        keywords,
        page_key: None,
    })
}

pub(crate) static TOGGLES: [Toggle; 29] = [
    Toggle {
        id: "legal-moves",
        title: "Show possible moves",
        hint: None,
        section: Section::Gameplay,
        on: |s| s.frontend.show_legal_moves,
        set: |s, on| s.frontend.show_legal_moves = on,
        quick: quick("showLegalMoves", "locate", "legal dots hints"),
    },
    Toggle {
        id: "premove",
        title: "Premoves",
        hint: Some("Plays your queued move automatically when legal"),
        section: Section::Gameplay,
        on: |s| s.frontend.premove,
        set: |s, on| s.frontend.premove = on,
        quick: quick("premove", "fast-forward", "queue move"),
    },
    Toggle {
        id: "confirm-move",
        title: "Confirm correspondence moves",
        hint: None,
        section: Section::Gameplay,
        on: |s| s.frontend.confirm_correspondence,
        set: |s, on| s.frontend.confirm_correspondence = on,
        quick: None,
    },
    Toggle {
        id: "opening-names",
        title: "Show opening names",
        hint: None,
        section: Section::Gameplay,
        on: |s| s.frontend.show_opening_name,
        set: |s, on| s.frontend.show_opening_name = on,
        quick: quick("showOpeningName", "book-marked", "eco"),
    },
    Toggle {
        id: "blindfold",
        title: "Blindfold",
        hint: Some("Hide pieces while playing"),
        section: Section::Gameplay,
        on: |s| s.frontend.blindfold,
        set: |s, on| s.frontend.blindfold = on,
        quick: quick("blindfold", "eye-off", "hide pieces"),
    },
    Toggle {
        id: "zen-mode",
        title: "Zen mode",
        hint: Some("Board and game controls only · Z to toggle"),
        section: Section::Gameplay,
        on: |s| s.frontend.zen_mode,
        set: |s, on| s.frontend.zen_mode = on,
        quick: Some(Quick {
            id: "zenMode",
            label: None,
            icon: "minimize",
            keywords: "focus distraction",
            page_key: Some(|cx| crate::menu::keys(&crate::menu::ToggleZen, cx)),
        }),
    },
    Toggle {
        id: "puzzle-next",
        title: "Next puzzle after solving",
        hint: None,
        section: Section::Gameplay,
        on: |s| s.frontend.puzzle_auto_next,
        set: |s, on| s.frontend.puzzle_auto_next = on,
        quick: None,
    },
    Toggle {
        id: "swipe-navigation",
        title: "Two-finger swipe",
        hint: Some("Go back and forward with a horizontal swipe"),
        section: Section::Gestures,
        on: |s| s.frontend.swipe_navigation,
        set: |s, on| s.frontend.swipe_navigation = on,
        quick: quick("swipeNavigation", "hand", "gesture trackpad back forward"),
    },
    Toggle {
        id: "swipe-indicator",
        title: "Swipe feedback",
        hint: Some("Show an edge hint that follows the swipe"),
        section: Section::Gestures,
        on: |s| s.frontend.swipe_indicator,
        set: |s, on| s.frontend.swipe_indicator = on,
        quick: None,
    },
    Toggle {
        id: "receive-challenges",
        title: "Receive challenges",
        hint: None,
        section: Section::Online,
        on: |s| s.core.receive_challenges,
        set: |s, on| s.core.receive_challenges = on,
        quick: None,
    },
    Toggle {
        id: "online-chat",
        title: "Game chat",
        hint: None,
        section: Section::Online,
        on: |s| s.core.online_chat,
        set: |s, on| s.core.online_chat = on,
        quick: None,
    },
    Toggle {
        id: "cloud-eval",
        title: "Lichess cloud evaluation",
        hint: Some("Sends analyzed positions to Lichess. Paused during your online games."),
        section: Section::Analysis,
        on: |s| s.core.cloud_eval,
        set: |s, on| s.core.cloud_eval = on,
        quick: quick("cloudEval", "cloud", "analysis engine"),
    },
    Toggle {
        id: "sound",
        title: "Game sounds",
        hint: None,
        section: Section::Sound,
        on: |s| s.frontend.sound_enabled,
        set: |s, on| s.frontend.sound_enabled = on,
        quick: Some(Quick {
            id: "soundEnabled",
            label: Some("Sound"),
            icon: "volume-2",
            keywords: "mute audio",
            page_key: None,
        }),
    },
    Toggle {
        id: "notifications",
        title: "Desktop notifications",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notifications_enabled,
        set: |s, on| s.frontend.notifications_enabled = on,
        quick: Some(Quick {
            id: "notificationsEnabled",
            label: Some("Notifications"),
            icon: "bell",
            keywords: "alerts",
            page_key: None,
        }),
    },
    Toggle {
        id: "notify-active",
        title: "While I'm using KChess",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_active,
        set: |s, on| s.frontend.notify_active = on,
        quick: None,
    },
    Toggle {
        id: "notify-background",
        title: "While KChess is in the background",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_background,
        set: |s, on| s.frontend.notify_background = on,
        quick: None,
    },
    Toggle {
        id: "notify-opponent",
        title: "Opponent moves",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_opponent_move,
        set: |s, on| s.frontend.notify_opponent_move = on,
        quick: None,
    },
    Toggle {
        id: "notify-low-time",
        title: "Low on time",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_low_time,
        set: |s, on| s.frontend.notify_low_time = on,
        quick: None,
    },
    Toggle {
        id: "notify-games",
        title: "Game start and result",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_game_events,
        set: |s, on| s.frontend.notify_game_events = on,
        quick: None,
    },
    Toggle {
        id: "notify-computer",
        title: "Computer moves",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_computer_move,
        set: |s, on| s.frontend.notify_computer_move = on,
        quick: None,
    },
    Toggle {
        id: "notify-challenges",
        title: "Challenges",
        hint: None,
        section: Section::Notifications,
        on: |s| s.frontend.notify_challenges,
        set: |s, on| s.frontend.notify_challenges = on,
        quick: None,
    },
    Toggle {
        id: "notify-sound",
        title: "System sound",
        hint: Some("For system notifications"),
        section: Section::Notifications,
        on: |s| s.frontend.notify_sound,
        set: |s, on| s.frontend.notify_sound = on,
        quick: None,
    },
    Toggle {
        id: "voice-confirm",
        title: "Confirm spoken moves",
        hint: Some("Say “confirm” before playing the move"),
        section: Section::Voice,
        on: |s| s.frontend.voice_confirm_moves,
        set: |s, on| s.frontend.voice_confirm_moves = on,
        quick: None,
    },
    Toggle {
        id: "voice-push-to-talk",
        title: "Hold to speak",
        hint: Some("Hold Space or the on-screen button"),
        section: Section::Voice,
        on: |s| s.frontend.voice_push_to_talk,
        set: |s, on| s.frontend.voice_push_to_talk = on,
        quick: None,
    },
    Toggle {
        id: "voice-history",
        title: "Keep voice history",
        hint: Some("Saves recognized text on this device; no audio"),
        section: Section::Voice,
        on: |s| s.core.voice_history,
        set: |s, on| s.core.voice_history = on,
        quick: None,
    },
    Toggle {
        id: "review-battery",
        title: "Also on battery power",
        hint: Some("Uses more power; reviews otherwise wait for a charger"),
        section: Section::Engine,
        on: |s| s.core.review_on_battery,
        set: |s, on| s.core.review_on_battery = on,
        quick: None,
    },
    Toggle {
        id: "update-auto-check",
        title: "Automatically check for updates",
        hint: Some("On startup and every six hours"),
        section: Section::Updates,
        on: |s| s.frontend.update_auto_check,
        set: |s, on| s.frontend.update_auto_check = on,
        quick: None,
    },
    Toggle {
        id: "update-auto-download",
        title: "Download updates in the background",
        hint: Some("Turning this off does not cancel an ongoing download"),
        section: Section::Updates,
        on: |s| s.frontend.update_auto_download,
        set: |s, on| s.frontend.update_auto_download = on,
        quick: None,
    },
    Toggle {
        id: "update-install-on-quit",
        title: "Install updates when I quit",
        hint: Some("KChess will not quit or restart automatically"),
        section: Section::Updates,
        on: |s| s.frontend.update_install_on_quit,
        set: |s, on| s.frontend.update_install_on_quit = on,
        quick: None,
    },
];
