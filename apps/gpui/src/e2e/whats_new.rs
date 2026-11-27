//! What's new: the notes of an update on its first start, and every release's notes on request.

use super::Harness;
use crate::whats_new::SEEN_KEY;

/// A fresh profile has nothing to catch up on; a profile last shown an older release gets that
/// update's notes on its next start, once.
#[gpui_kit::test]
fn an_update_shows_its_notes_once(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert!(!app.shows("whats-new"), "a first start shows no notes");
    let current = crate::updates::current_version();
    let seen: Option<String> = app.cx.update(|cx| crate::prefs::get(cx, SEEN_KEY, None));
    assert_eq!(seen.as_deref(), Some(current.as_str()));
    // As if the last start had been an older release.
    app.cx
        .update(|cx| crate::prefs::set(cx, SEEN_KEY, "2026.1.0".to_string()));
    let mut app = app.restart();
    app.find("whats-new");
    app.press("escape");
    app.wait_gone("whats-new");
    let mut app = app.restart();
    assert!(!app.shows("whats-new"), "the notes show once");
}

/// The notes after an update say how to stop them: the box turns them off, and the next update
/// starts with none, though it still counts as shown. Settings → Updates turns them back on, on
/// every installation: its switch was first left out of the page.
#[gpui_kit::test]
fn the_notes_after_an_update_can_be_turned_off(cx: &mut gpui_kit::TestAppContext) {
    let app = Harness::start(cx);
    app.cx
        .update(|cx| crate::prefs::set(cx, SEEN_KEY, "2026.1.0".to_string()));
    let mut app = app.restart();
    app.find("whats-new");
    app.click("whats-new-skip");
    app.wait_for_saved("updateShowNotes", serde_json::json!(false));
    app.click("whats-new-close");
    app.wait_gone("whats-new");
    app.cx
        .update(|cx| crate::prefs::set(cx, SEEN_KEY, "2026.1.0".to_string()));
    let mut app = app.restart();
    assert!(!app.shows("whats-new"), "the notes stay off");
    let seen: Option<String> = app.cx.update(|cx| crate::prefs::get(cx, SEEN_KEY, None));
    assert_eq!(seen, Some(crate::updates::current_version()));
    // Settings → Updates turns them back on, though this build cannot update itself and hides
    // the updater's own switches.
    app.open_settings("Updates");
    assert!(!app.shows("update-auto-check"));
    app.click("update-show-notes");
    app.wait_for_saved("updateShowNotes", serde_json::json!(true));
}

/// Notes read before installing marked the update's version read; when that install failed, the
/// app started as the older release, which showed nothing and moved the mark back, so the next
/// successful install showed the same notes again.
#[gpui_kit::test]
fn notes_read_before_a_failed_install_stay_read(cx: &mut gpui_kit::TestAppContext) {
    let app = Harness::start(cx);
    let ahead = "2099.1.0".to_string();
    app.cx
        .update(|cx| crate::prefs::set(cx, SEEN_KEY, ahead.clone()));
    let mut app = app.restart();
    assert!(!app.shows("whats-new"));
    let seen: Option<String> = app.cx.update(|cx| crate::prefs::get(cx, SEEN_KEY, None));
    assert_eq!(seen, Some(ahead), "the mark only moves forward");
}

/// Every release's notes open from the account menu, Settings → Updates and Quick Pick, and Escape
/// closes them.
#[gpui_kit::test]
fn the_notes_open_from_settings_and_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.click("account-menu");
    app.click_label("What's new");
    app.find("whats-new");
    app.press("escape");
    app.wait_gone("whats-new");
    app.open_settings("Updates");
    app.click("whats-new-open");
    app.find("whats-new");
    app.press("escape");
    app.wait_gone("whats-new");
    app.press("secondary-k");
    app.type_text("what's new");
    super::palette::wait_for_first(&mut app, "What's new");
    app.press("enter");
    app.find("whats-new");
}

/// In a short window the notes run past the bottom of the list, and the wheel scrolls to them.
#[gpui_kit::test]
fn the_notes_scroll_in_a_short_window(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    let mut app = Harness::start(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(900.), px(480.)));
    app.settle();
    app.click("account-menu");
    app.click_label("What's new");
    let list = app.find("whats-new").bounds();
    // The longest notes alone are taller than a short window's list. Not the newest: a
    // development build shows the unreleased notes first, which may be one line.
    let longest = crate::whats_new::releases()
        .iter()
        .enumerate()
        .max_by_key(|(_, release)| release.items.len())
        .map(|(index, _)| index)
        .expect("there are notes");
    // Drawn, if below the window's bottom edge while the notes before it are long: measured
    // without waiting for it to be in view.
    let release = |app: &mut Harness| {
        use gpui_kit::test::TestWindowExt as _;
        let mut bounds = None;
        app.wait_for("the longest release drawn", |window, _| {
            bounds = window
                .try_find(("whats-new-release", longest))
                .map(|release| release.bounds());
            bounds.is_some()
        });
        bounds.expect("drawn")
    };
    let first = release(&mut app);
    assert!(
        first.size.height > list.size.height,
        "the longest release {first:?} keeps its height past the list {list:?}, to scroll to"
    );
    app.scroll_point(list.center(), px(-300.));
    let scrolled = release(&mut app);
    assert!(
        scrolled.top() < first.top(),
        "the wheel scrolls the notes: {first:?} then {scrolled:?}"
    );
}
