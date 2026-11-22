//! KChess in the background: closing the window keeps it running or quits as chosen, the menu
//! bar or tray menu opens it again and answers what waits, and opening at login writes the
//! system's login entry.

use serde_json::json;

use super::Harness;
use crate::background::{Background, menu_labels, pick};

/// The close choice's pills: Keep running, then Quit.
const KEEP_RUNNING: (&str, usize) = ("close-action", 0);
const QUIT: (&str, usize) = ("close-action", 1);

/// What the core keeps for a close choice: the platform's own is kept as the default.
fn stored(background: bool) -> serde_json::Value {
    let mac = cfg!(target_os = "macos");
    match (background, mac) {
        (true, true) | (false, false) => json!("default"),
        (true, false) => json!("background"),
        (false, true) => json!("quit"),
    }
}

/// Open Settings at the background rows, found from Search as a person would: they sit at the
/// foot of Notifications.
fn open_background_settings(app: &mut Harness) {
    app.press("secondary-k");
    app.type_text("#When you close the window");
    super::palette::wait_for_first(app, "When you close the window");
    app.press("enter");
}

/// Choose what closing the window does, from Settings → Notifications, and close Settings.
fn choose_close(app: &mut Harness, background: bool) {
    open_background_settings(app);
    app.click(if background { KEEP_RUNNING } else { QUIT });
    app.wait_for_saved("closeAction", stored(background));
    app.click("settings-close");
}

fn quit_asked(app: &mut Harness) -> bool {
    app.cx.read(|cx| cx.global::<Background>().quit_asked)
}

fn still_running_shown(app: &mut Harness) -> usize {
    app.cx
        .shown_system_notifications()
        .iter()
        .filter(|shown| shown.title.as_ref() == "KChess is still running")
        .count()
}

/// With Quit chosen, closing the window quits KChess.
#[gpui_kit::test]
fn closing_quits_when_quit_is_chosen(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    choose_close(&mut app, false);
    assert!(!quit_asked(&mut app));
    app.close_window();
    assert!(quit_asked(&mut app), "closing the window quits");
    assert_eq!(still_running_shown(&mut app), 0);
}

/// With Keep running chosen, closing the window leaves KChess running: the core still answers,
/// one notification says where it went (not again on the next close), and the notification
/// and the menu's entries open the window again, on the page they name.
#[gpui_kit::test]
fn closing_keeps_running_and_the_menu_opens_it_again(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    choose_close(&mut app, true);
    app.close_window();
    assert!(!quit_asked(&mut app), "still running");
    assert_eq!(still_running_shown(&mut app), 1);
    // The core still answers with no window open.
    assert_eq!(app.saved_settings()["closeAction"], stored(true));
    let tag = app
        .cx
        .shown_system_notifications()
        .into_iter()
        .find(|shown| shown.title.as_ref() == "KChess is still running")
        .expect("the notification")
        .tag;
    app.cx
        .simulate_system_notification_response(gpui_kit::SystemNotificationResponse {
            tag,
            action_id: None,
        });
    app.adopt_window();

    app.close_window();
    assert!(!quit_asked(&mut app));
    assert_eq!(still_running_shown(&mut app), 1, "told once");
    app.cx.update(|cx| pick("Puzzles", None, cx));
    app.adopt_window();
    app.wait_for_page(crate::pages::Page::Puzzles);

    app.close_window();
    app.cx.update(|cx| pick("Open KChess", None, cx));
    app.adopt_window();
}

/// Without the menu bar or tray icon, nothing shows KChess runs on Windows and Linux, so closing
/// quits there; macOS keeps it in the Dock.
#[gpui_kit::test]
fn without_the_icon_closing_quits_where_nothing_else_shows_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    choose_close(&mut app, true);
    open_background_settings(&mut app);
    app.click("show-tray");
    app.wait_for_saved("showTrayIcon", json!(false));
    assert!(!app.cx.read(crate::background::tray_shown));
    app.click("settings-close");
    app.close_window();
    assert_eq!(quit_asked(&mut app), !cfg!(target_os = "macos"));
}

/// Opening at login writes the system's login entry, which starts KChess quietly, and turning it
/// off removes it.
#[gpui_kit::test]
fn opening_at_login_writes_and_removes_the_login_entry(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let folder = crate::profile::data_dir()
        .expect("the profile")
        .join("autostart");
    let name = crate::background::login_entry().expect("the entry").name;
    let file = kchess_background::autostart::file(&folder, &name);
    open_background_settings(&mut app);
    app.click("open-at-login");
    app.wait_for_saved("openAtLogin", json!(true));
    app.wait_for_app("the login entry", |_| file.exists());
    app.click("open-at-login");
    app.wait_for_saved("openAtLogin", json!(false));
    app.wait_for_app("the login entry to go", |_| !file.exists());
}

/// Do not disturb in the menu turns notifications off and back on, and shows it is on.
#[gpui_kit::test]
fn do_not_disturb_turns_notifications_off(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert!(
        app.cx
            .read(menu_labels)
            .contains(&"Do not disturb".to_string())
    );
    app.cx.update(|cx| pick("Do not disturb", None, cx));
    app.wait_for_saved("notificationsEnabled", json!(false));
    app.wait_for_app("the check mark", |cx| {
        menu_labels(cx).contains(&"[x] Do not disturb".to_string())
    });
    app.cx.update(|cx| pick("Do not disturb", None, cx));
    app.wait_for_saved("notificationsEnabled", json!(true));
}

/// A challenge received in the background shows in the menu with its terms, and Decline there
/// answers it; the menu counts it among the notifications missed, and Notifications opens them.
#[gpui_kit::test]
fn a_challenge_is_answered_from_the_menu(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    gpui_kit::VisualTestContext::from_window(app.window, &app.cx).deactivate_window();
    app.wait_for_app("Play on Lichess in the menu", |cx| {
        menu_labels(cx).contains(&"Play on Lichess".to_string())
    });
    lichess.lock().events.push(json!({
        "type": "challenge",
        "challenge": {
            "id": "Menu0001",
            "status": "created",
            "challenger": { "id": "bob", "name": "Bob", "rating": 1700 },
            "destUser": { "id": "tester", "name": "Tester", "rating": 1500 },
            "variant": { "key": "standard", "name": "Standard" },
            "rated": true,
            "speed": "rapid",
            "timeControl": { "type": "clock", "limit": 600, "increment": 5 },
            "color": "white",
        },
        "compat": { "board": true },
    }));
    app.wait_for_app("the challenge in the menu", |cx| {
        menu_labels(cx).contains(&"Bob (1700) challenges you".to_string())
    });
    let labels = app.cx.read(menu_labels);
    let at = labels
        .iter()
        .position(|label| label == "Bob (1700) challenges you")
        .expect("the challenge");
    assert_eq!(
        labels[at + 1..at + 4],
        [">10+5 · Standard · Rated", ">Accept", ">Decline"]
    );
    app.cx.update(|cx| pick("Decline", Some("Bob"), cx));
    app.wait_for_app("the decline to reach Lichess", |_| {
        lichess
            .lock()
            .writes
            .contains(&"POST /api/challenge/Menu0001/decline".to_string())
    });
    app.wait_for_app("the missed challenge counted", |cx| {
        menu_labels(cx).contains(&"Notifications (1)".to_string())
    });
    app.cx.update(|cx| pick("Notifications (1)", None, cx));
    app.find("notifications-panel");
}
