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
