//! The notification centre: an alert missed while KChess was in the background waits under the
//! bell in the top bar, which shows a dot until it is read; the panel opens it where it points,
//! and empties when cleared.

use gpui_kit::AppContext as _;
use gpui_kit::test::TestQueryExt as _;
use kchess_contracts::types::{NotificationEntry, NotificationKind};

use super::Harness;
use crate::notifications::{Outcome, centre};
use crate::pages::Page;

fn alerts_on(app: &mut Harness) {
    app.cx.update(|cx| {
        crate::settings::change(cx, |s| {
            s.frontend.notifications_enabled = true;
            s.frontend.notify_background = true;
            s.frontend.notify_active = true;
            s.frontend.notify_game_events = true;
        })
    });
}

fn entries(app: &mut Harness) -> Vec<NotificationEntry> {
    app.cx
        .update(|cx| centre::handle(cx).read(cx).entries().to_vec())
}

/// Wait until the centre holds `count` notifications, as the core answers.
fn wait_for_entries(app: &mut Harness, count: usize) -> Vec<NotificationEntry> {
    app.wait_for(&format!("{count} notifications"), |_, cx| {
        centre::handle(cx).read(cx).entries().len() == count
    });
    entries(app)
}

/// An alert while KChess is in the background: shown by the system, and kept unread.
fn missed(app: &mut Harness, title: &str) {
    gpui_kit::VisualTestContext::from_window(app.window, &app.cx).deactivate_window();
    let outcome = app
        .cx
        .update(|cx| crate::notifications::desktop(NotificationKind::GameEvents, title, "", cx));
    assert_eq!(outcome, Outcome::System);
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window is open");
    app.cx.run_until_parked();
}

#[gpui_kit::test]
fn a_missed_notification_waits_under_the_bell(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    alerts_on(&mut app);
    assert!(!app.shows("notifications-unread"));
    assert!(
        app.with_window(|window, _| window.try_find_by_label("Notifications").is_some()),
        "the bell is named"
    );

    missed(&mut app, "Game over");
    let kept = wait_for_entries(&mut app, 1);
    assert_eq!((kept[0].title.as_str(), kept[0].read), ("Game over", false));
    app.find("notifications-unread");
    assert!(
        app.with_window(|window, _| {
            window
                .try_find_by_label("Notifications, 1 unread")
                .is_some()
        }),
        "the bell says how many are unread"
    );

    // The panel lists it; opening it goes to its game's page and reads it.
    app.click("notifications-button");
    app.find("notifications-panel");
    let row = format!("notification-{}", kept[0].id);
    app.click(gpui_kit::SharedString::from(row.clone()));
    app.wait_for_page(Page::Online);
    app.wait_gone("notifications-unread");
    app.wait_gone("notifications-panel");
    assert!(entries(&mut app).iter().all(|entry| entry.read));

    // Another, then Clear all: the panel says there are none.
    missed(&mut app, "Your opponent left");
    wait_for_entries(&mut app, 2);
    app.find("notifications-unread");
    app.click("notifications-button");
    app.click("notifications-clear");
    app.find("notifications-empty");
    assert!(!app.shows("notifications-clear"));
    wait_for_entries(&mut app, 0);
    app.wait_gone("notifications-unread");
}

/// An alert shown inside the window in front is kept as seen: the bell lights only for what
/// was missed. Each can be dismissed on its own.
#[gpui_kit::test]
fn an_alert_seen_in_the_window_is_kept_read(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    alerts_on(&mut app);
    let outcome = app.cx.update(|cx| {
        crate::notifications::desktop(NotificationKind::GameEvents, "Game started", "", cx)
    });
    assert_eq!(outcome, Outcome::InApp);
    let kept = wait_for_entries(&mut app, 1);
    assert!(kept[0].read);
    assert!(!app.shows("notifications-unread"));

    app.press("shift-secondary-n");
    app.find("notifications-panel");
    app.click(gpui_kit::SharedString::from(format!(
        "notification-dismiss-{}",
        kept[0].id
    )));
    app.find("notifications-empty");
    wait_for_entries(&mut app, 0);
}

/// A challenge gone since its alert is shown as expired (`centre::detail`), and its row offers
/// no Accept: opening it shows the challenges as they stand.
#[gpui_kit::test]
fn an_expired_challenge_offers_nothing_to_accept(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    alerts_on(&mut app);
    app.cx
        .update(|cx| crate::settings::change(cx, |s| s.frontend.notify_challenges = true));
    gpui_kit::VisualTestContext::from_window(app.window, &app.cx).deactivate_window();
    app.cx.update(|cx| {
        crate::notifications::challenge("abcd1234", "Anna challenges you", "5+3 rated", cx)
    });
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window is open");
    let kept = wait_for_entries(&mut app, 1);
    app.click("notifications-button");
    app.find("notifications-panel");
    assert!(app.with_window(|window, _| window.try_find_by_label("Accept").is_none()));
    let standing = app.cx.update(|cx| centre::standing(cx));
    assert!(centre::expired(&kept[0].link, &standing));
    app.click(gpui_kit::SharedString::from(format!(
        "notification-{}",
        kept[0].id
    )));
    app.wait_for_page(Page::Online);
}

/// Another window, or the command line, changes the list: the bell shows it as the core keeps it.
#[gpui_kit::test]
fn a_change_made_elsewhere_reaches_the_bell(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::types::{NotificationInput, ProfileArea};
    let mut app = Harness::start(cx);
    let kept = app.request(kchess_contracts::calls::RecordNotification {
        entry: NotificationInput {
            title: "Your move against Anna".into(),
            ..NotificationInput::default()
        },
    });
    app.changed_elsewhere(&[ProfileArea::Notifications]);
    wait_for_entries(&mut app, 1);
    app.find("notifications-unread");
    app.request(kchess_contracts::calls::MarkNotificationsRead { id: None });
    app.changed_elsewhere(&[ProfileArea::Notifications]);
    app.wait_gone("notifications-unread");
    assert_eq!(entries(&mut app)[0].id, kept[0].id);
}
