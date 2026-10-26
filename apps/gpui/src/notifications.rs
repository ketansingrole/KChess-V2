//! Alerts for the player (`electron/main/notify.ts`, `utils/gameAlerts.ts`), when Settings allow
//! them: inside the window while KChess is in front (macOS shows no banner for the app in front),
//! a system notification in the background. An alert about the board being looked at is dropped.

use gpui_kit::App;
use kchess_contracts::types::Settings;

use crate::pages::Page;

/// Why an alert was not shown, or how it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    InApp,
    System,
    Disabled,
    CategoryOff,
    WindowState,
    Redundant,
}

/// The Settings switch for each kind; a test follows only the master switch.
fn category(settings: &Settings, kind: &str) -> bool {
    let core = &settings.frontend;
    match kind {
        "opponentMove" => core.notify_opponent_move,
        "lowTime" => core.notify_low_time,
        "gameEvents" => core.notify_game_events,
        "computerMove" => core.notify_computer_move,
        "challenge" => core.notify_challenges,
        _ => true,
    }
}

/// The page that shows the game an alert is about (`GAME_PAGE`).
fn game_page(kind: &str) -> Option<Page> {
    match kind {
        "opponentMove" | "lowTime" | "gameEvents" => Some(Page::Online),
        "computerMove" => Some(Page::Computer),
        _ => None,
    }
}

/// Whether to alert, and how: `away` is KChess in the background, `page` the page shown.
pub fn decide(settings: &Settings, kind: &str, away: bool, page: Page) -> Outcome {
    let core = &settings.frontend;
    if !core.notifications_enabled {
        return Outcome::Disabled;
    }
    if kind != "test" {
        if !category(settings, kind) {
            return Outcome::CategoryOff;
        }
        if !(if away {
            core.notify_background
        } else {
            core.notify_active
        }) {
            return Outcome::WindowState;
        }
        // A game alert repeats what the board shows while the player is looking at it.
        if !away && game_page(kind) == Some(page) {
            return Outcome::Redundant;
        }
    }
    if away {
        Outcome::System
    } else {
        Outcome::InApp
    }
}

/// Who sends a notification on macOS: a packaged app's own bundle, or for a bare `cargo run`
/// binary (no bundle) the branded development app `pnpm run dev` builds. Without one the
/// notification library asks macOS for an app named "use_default", which opens a Choose
/// Application dialog.
#[cfg(target_os = "macos")]
fn sender() -> String {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_else(|| DEV_SENDER.to_string())
}

/// `.dev/KChess.app`'s bundle identifier (`tooling/with-branded-electron.mjs`).
#[cfg(target_os = "macos")]
const DEV_SENDER: &str = "com.kchess.app.dev";

/// Alert the player about `kind` (`notifyDesktop`).
pub fn desktop(kind: &str, title: &str, body: &str, cx: &mut App) -> Outcome {
    let Some(settings) = cx
        .try_global::<crate::settings::SettingsHandle>()
        .map(|settings| settings.0.read(cx).settings.clone())
    else {
        return Outcome::Disabled;
    };
    let away = cx.active_window().is_none();
    let page = crate::shell::current_page(cx).unwrap_or(Page::Dashboard);
    // Another KChess window on this profile raises the alert (`docs/shared-core.md`).
    if !crate::core::alerts_here(cx) {
        return Outcome::WindowState;
    }
    let outcome = decide(&settings, kind, away, page);
    match outcome {
        Outcome::InApp => crate::online::notice(cx, format!("{title}\n{body}")),
        Outcome::System => {
            let (title, body, sound) = (
                title.to_string(),
                body.to_string(),
                settings.frontend.notify_sound,
            );
            // The system call can wait on the notification centre; it stays off the UI thread.
            std::thread::spawn(move || {
                #[cfg(target_os = "macos")]
                {
                    static SENDER: std::sync::Once = std::sync::Once::new();
                    SENDER.call_once(|| {
                        let _ = notify_rust::set_application(&sender());
                    });
                }
                let mut notification = notify_rust::Notification::new();
                notification.summary(&title).body(&body);
                if sound {
                    notification.sound_name("default");
                }
                if let Err(cause) = notification.show() {
                    crate::log::write(
                        kchess_core::Level::Warn,
                        "notify",
                        &format!("Notification failed: {cause}"),
                    );
                }
            });
        }
        _ => {}
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::{Outcome, decide};
    use crate::pages::Page;
    use kchess_contracts::types::Settings;

    fn settings() -> Settings {
        let mut settings: Settings =
            kchess_contracts::defaults::default_settings().expect("the default settings read");
        settings.frontend.notifications_enabled = true;
        settings.frontend.notify_active = true;
        settings.frontend.notify_background = true;
        settings.frontend.notify_low_time = true;
        settings
    }

    /// A bare test binary has no bundle: it sends as the development app, never "use_default".
    #[cfg(target_os = "macos")]
    #[test]
    fn a_binary_without_a_bundle_sends_as_the_development_app() {
        assert_eq!(super::sender(), super::DEV_SENDER);
    }

    #[test]
    fn alerts_follow_the_switches_and_the_window() {
        let mut on = settings();
        assert_eq!(decide(&on, "lowTime", true, Page::Online), Outcome::System);
        assert_eq!(
            decide(&on, "lowTime", false, Page::Dashboard),
            Outcome::InApp
        );
        // The board in front already shows it.
        assert_eq!(
            decide(&on, "lowTime", false, Page::Online),
            Outcome::Redundant
        );
        on.frontend.notify_low_time = false;
        assert_eq!(
            decide(&on, "lowTime", true, Page::Online),
            Outcome::CategoryOff
        );
        // A test needs only the master switch.
        assert_eq!(decide(&on, "test", false, Page::Settings), Outcome::InApp);
        on.frontend.notifications_enabled = false;
        assert_eq!(
            decide(&on, "test", false, Page::Settings),
            Outcome::Disabled
        );
    }
}
