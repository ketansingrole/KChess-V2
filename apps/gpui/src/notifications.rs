//! Alerts for the player, when Settings allow
//! them: inside the window while KChess is in front (macOS shows no banner for the app in front),
//! a system notification in the background. An alert about the board being looked at is dropped.

use gpui_kit::App;
use kchess_contracts::types::Settings;

use crate::pages::Page;

/// Toasts rise from the bottom right: the top right holds each page's tabs and menus, which a
/// toast there would cover for as long as it shows.
pub fn init(cx: &mut App) {
    gpui_kit::component::Theme::global_mut(cx)
        .notification
        .placement = gpui_kit::Anchor::BottomRight;
    if through_gpui() {
        identify(cx);
        // Not from a test run, which would change the shortcuts of whoever runs it.
        #[cfg(all(windows, not(test)))]
        crate::windows_shortcuts::stamp(APP_ID);
    }
}

/// Whether system notifications go through GPUI, which names KChess as their sender and tells it
/// when one is clicked: on Windows (where the notification library sent them as PowerShell, whose
/// clicks went nowhere), Linux, and macOS from an app bundle. GPUI's macOS notifications need the
/// bundle (`UNUserNotificationCenter` aborts a bare binary), so a bare `cargo run` binary keeps
/// the library, sending as the development app (`sender`).
fn through_gpui() -> bool {
    // Tests post to GPUI's test platform, which keeps them: never to the desktop, where a test
    // run left "Game started" banners from a Terminal icon.
    if cfg!(test) {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        bundled()
    }
    #[cfg(not(target_os = "macos"))]
    {
        cfg!(any(windows, target_os = "linux", target_os = "freebsd"))
    }
}

/// Whether KChess runs from an app bundle, read once before the notification library can
/// replace what `bundleIdentifier` answers.
#[cfg(target_os = "macos")]
fn bundled() -> bool {
    static BUNDLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *BUNDLED.get_or_init(|| {
        objc2_foundation::NSBundle::mainBundle()
            .bundleIdentifier()
            .is_some()
    })
}

/// KChess's identity: on Windows its AppUserModelID, so its notifications are its own and a click
/// on one comes back to it. The installer's shortcuts carry it too (`windows_shortcuts`), so the
/// taskbar keeps one KChess button. On Linux GPUI names the sender from it; macOS names it from
/// the bundle.
pub const APP_ID: &str = "com.kchess.app";

/// Send notifications as KChess, and bring KChess forward on the game a clicked one is about.
pub(crate) fn identify(cx: &mut App) {
    cx.set_app_identity(APP_ID, "KChess");
    cx.on_system_notification_response(|response, cx| {
        open_from(&response.tag, response.action_id.as_deref(), cx)
    });
}

/// A notification tagged `tag` was clicked (`action` the button, or none for its body).
fn open_from(tag: &str, action: Option<&str>, cx: &mut App) {
    if let Some(id) = tag.strip_prefix(CHALLENGE_TAG) {
        return answer_challenge(id, action, cx);
    }
    if let Some(game) = tag.strip_prefix(MOVE_TAG) {
        return answer_move(game, action, cx);
    }
    if let Some(game) = tag.strip_prefix(CONFIRM_TAG) {
        return confirm_move(tag, game, action, cx);
    }
    show_page(game_page(tag), cx);
}

/// KChess comes forward, on `page` when there is one.
fn show_page(page: Option<Page>, cx: &mut App) {
    cx.activate(true);
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, cx| {
            window.activate_window();
            if let Some(page) = page {
                crate::shell::open(page, window, cx);
            }
        });
    }
}

/// A challenge notification's tag: this, then the challenge's id.
const CHALLENGE_TAG: &str = "challenge:";

/// Accept or decline a challenge from its notification: Accept brings KChess forward on Play on
/// Lichess, where the game starts; Decline answers it without; the body opens the challenges.
fn answer_challenge(id: &str, action: Option<&str>, cx: &mut App) {
    let online = crate::online::handle(cx);
    let challenge = online
        .read(cx)
        .challenges
        .iter()
        .find(|challenge| challenge.id == id)
        .cloned();
    match (action, challenge) {
        (Some("decline"), Some(challenge)) => {
            online.update(cx, |online, cx| {
                online.decline(
                    &challenge,
                    kchess_contracts::types::DeclineReason::Generic,
                    cx,
                )
            });
        }
        (Some("accept"), Some(challenge)) => {
            show_page(Some(Page::Online), cx);
            online.update(cx, |online, cx| online.accept(&challenge, cx).detach());
        }
        // Answered or withdrawn meanwhile, or the body: the challenges as they are now.
        _ => show_page(Some(Page::Online), cx),
    }
}

/// A correspondence game's "your move" notification's tag: this, then `account/id`.
const MOVE_TAG: &str = "move:";
/// The tag of the notification confirming a move typed into it, for the same game.
const CONFIRM_TAG: &str = "confirm:";
/// The text box of a "your move" notification: its button, a tab, its placeholder (the GPUI
/// patch's `input:` actions, `vendor/gpui-pre-windows/KCHESS-PATCH.md`).
const MOVE_INPUT: &str = "input:move";

/// A "your move" notification's buttons: a box to type the move, and Go to board.
fn move_actions() -> Vec<gpui_kit::SystemNotificationAction> {
    vec![
        button(MOVE_INPUT, "Play\tYour move, such as Nf3"),
        button("open", "Go to board"),
    ]
}

/// Alert the player that it is their move in correspondence `game`, which they can answer from
/// the notification where the system takes typed text (Windows, macOS).
pub fn your_move(game: &kchess_contracts::types::OngoingGame, cx: &mut App) -> Outcome {
    let account = &game.account;
    let id = &game.game_id;
    let opponent = &game.opponent.name;
    alert(
        "opponentMove",
        Some((format!("{MOVE_TAG}{account}/{id}"), move_actions())),
        &format!("Your move against {opponent}"),
        &format!("Correspondence game · @{account}"),
        cx,
    )
}

/// Open correspondence game `account/id` on the board, KChess in front.
fn open_game(game: &str, cx: &mut App) {
    show_page(Some(Page::Online), cx);
    if let Some((account, id)) = game.split_once('/') {
        let (account, id) = (account.to_string(), id.to_string());
        crate::online::handle(cx).update(cx, |online, cx| online.open(&account, &id, cx));
    }
}

/// A "your move" notification answered: a typed move is offered back to confirm (or, not
/// legal there, asked for again), all without bringing KChess forward; Go to board or the body
/// open the game.
fn answer_move(game: &str, action: Option<&str>, cx: &mut App) {
    let typed = action
        .and_then(|action| action.strip_prefix(MOVE_INPUT))
        .and_then(|rest| rest.strip_prefix('\n'))
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let Some(text) = typed else {
        return open_game(game, cx);
    };
    let id = game.split_once('/').map_or("", |(_, id)| id);
    let listed = crate::online::handle(cx)
        .read(cx)
        .ongoing
        .iter()
        .find(|listed| listed.game_id == id)
        .cloned();
    let Some(listed) = listed.filter(|listed| listed.is_my_turn) else {
        return post_tagged(
            format!("{MOVE_TAG}{game}"),
            "That game has moved on",
            "Open it to see where it stands",
            vec![button("open", "Go to board")],
            false,
            cx,
        );
    };
    match crate::move_reply::read(&listed, text) {
        Some(reply) => {
            let tag = format!("{CONFIRM_TAG}{game}");
            let (title, body) = (
                format!("Play {} against {}?", reply.san, reply.opponent),
                format!("Correspondence game · @{}", reply.account),
            );
            let play = button("play", &format!("Play {}", reply.san));
            crate::move_reply::hold(&tag, reply, cx);
            post_tagged(
                tag,
                &title,
                &body,
                vec![play, button("cancel", "Cancel")],
                false,
                cx,
            );
        }
        None => post_tagged(
            format!("{MOVE_TAG}{game}"),
            &format!("{text} is not a legal move"),
            &format!("Your move against {}", listed.opponent.name),
            move_actions(),
            false,
            cx,
        ),
    }
}

/// The confirmation of a typed move answered: Play sends it once, then says whether it was
/// played; Cancel drops it; the body opens the game.
fn confirm_move(tag: &str, game: &str, action: Option<&str>, cx: &mut App) {
    match action {
        Some("play") => {
            let Some(reply) = crate::move_reply::take(tag, cx) else {
                return;
            };
            let sending = crate::move_reply::send(&reply, cx);
            let tag = tag.to_string();
            cx.spawn(async move |cx| {
                let answer = sending.await;
                cx.update(|cx| {
                    let body = format!("Correspondence game · @{}", reply.account);
                    match answer {
                        Ok(_) => {
                            post_tagged(
                                tag,
                                &format!("Played {} against {}", reply.san, reply.opponent),
                                &body,
                                Vec::new(),
                                false,
                                cx,
                            );
                            crate::online::handle(cx)
                                .update(cx, |online, cx| online.refresh_ongoing(cx));
                        }
                        Err(why) => post_tagged(
                            tag,
                            &format!("{} was not played", reply.san),
                            &why,
                            vec![button("open", "Go to board")],
                            false,
                            cx,
                        ),
                    }
                });
            })
            .detach();
        }
        Some("cancel") => {
            crate::move_reply::take(tag, cx);
            cx.dismiss_system_notification(tag);
        }
        _ => open_game(game, cx),
    }
}

/// A system notification through GPUI, tagged `tag` (its kind, or a challenge's), with `actions`
/// as its buttons.
fn post_tagged(
    tag: String,
    title: &str,
    body: &str,
    actions: Vec<gpui_kit::SystemNotificationAction>,
    sound: bool,
    cx: &mut App,
) {
    cx.show_system_notification(gpui_kit::SystemNotification {
        tag: tag.into(),
        title: title.to_string().into(),
        body: body.to_string().into(),
        actions,
    });
    // Windows shows it silently; the setting's sound is KChess's own.
    if sound {
        crate::sound::play(crate::sound::Sound::GenericNotify, cx);
    }
}

fn button(id: &str, label: &str) -> gpui_kit::SystemNotificationAction {
    gpui_kit::SystemNotificationAction {
        id: id.to_string().into(),
        label: label.to_string().into(),
    }
}

/// A system notification about `kind`, tagged with it; a game's offers Go to board.
pub(crate) fn post(kind: &str, title: &str, body: &str, sound: bool, cx: &mut App) {
    let actions = game_page(kind)
        .map(|_| button("open", "Go to board"))
        .into_iter()
        .collect();
    post_tagged(kind.to_string(), title, body, actions, sound, cx);
}

/// Alert the player to a challenge `id` they received, with Accept and Decline where the system
/// shows buttons.
pub fn challenge(id: &str, title: &str, body: &str, cx: &mut App) -> Outcome {
    let tag = format!("{CHALLENGE_TAG}{id}");
    let actions = vec![button("accept", "Accept"), button("decline", "Decline")];
    alert("challenge", Some((tag, actions)), title, body, cx)
}

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
/// binary (no bundle) the branded development app. Without one the
/// notification library asks macOS for an app named "use_default", which opens a Choose
/// Application dialog. Read once, before that library replaces what `bundleIdentifier` answers
/// for the whole process.
#[cfg(target_os = "macos")]
fn sender() -> String {
    static SENDER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SENDER
        .get_or_init(|| {
            objc2_foundation::NSBundle::mainBundle()
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_else(|| DEV_SENDER.to_string())
        })
        .clone()
}

/// `.dev/KChess.app`'s bundle identifier.
#[cfg(target_os = "macos")]
const DEV_SENDER: &str = "com.kchess.app.dev";

/// Alert the player about `kind` (`notifyDesktop`).
pub fn desktop(kind: &str, title: &str, body: &str, cx: &mut App) -> Outcome {
    alert(kind, None, title, body, cx)
}

/// `desktop`, with a system notification's own tag and buttons in place of its kind's.
fn alert(
    kind: &str,
    tagged: Option<(String, Vec<gpui_kit::SystemNotificationAction>)>,
    title: &str,
    body: &str,
    cx: &mut App,
) -> Outcome {
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
        Outcome::System if through_gpui() => {
            let sound = settings.frontend.notify_sound;
            match tagged {
                Some((tag, actions)) => post_tagged(tag, title, body, actions, sound, cx),
                None => post(kind, title, body, sound, cx),
            }
        }
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
                        kchess_contracts::Level::Warn,
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
        // The notification library answers for the bundle from then on, in the whole process,
        // and any test that sends one gets there first on some runs.
        let _ = notify_rust::set_application("com.apple.Finder");
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
