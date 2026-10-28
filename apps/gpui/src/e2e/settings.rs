//! Settings: changes are saved to the profile, shown at once and kept across launches.

use super::Harness;
use serde_json::json;

#[gpui_kit::test]
fn a_switch_is_saved_and_disables_what_depends_on_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    assert_eq!(app.find("sound").checked(), Some(true));
    app.click("sound");
    assert_eq!(app.find("sound").checked(), Some(false));
    app.wait_for_saved("soundEnabled", json!(false));
}

#[gpui_kit::test]
fn a_chosen_board_is_saved_and_marked(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let other = crate::board::looks::boards()
        .iter()
        .map(|board| board.id.to_string())
        .find(|id| *id != app.saved_settings()["boardTheme"])
        .expect("another board");
    let option = format!("board-{other}");
    assert_eq!(app.find(option.clone()).selected(), Some(false));
    app.click(option.clone());
    assert_eq!(app.find(option).selected(), Some(true));
    app.wait_for_saved("boardTheme", json!(other));
}

#[gpui_kit::test]
fn the_theme_choice_is_saved(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let dark = ("appearance", 2usize);
    assert_eq!(app.find(dark).label(), Some("Dark"));
    app.click(dark);
    assert_eq!(app.find(dark).selected(), Some(true));
    app.wait_for_saved("appearance", json!("dark"));
}

#[gpui_kit::test]
fn settings_are_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Sound");
    app.click("sound");
    assert_eq!(app.find("sound").checked(), Some(false));
    // Quit before the 400 ms save: quitting flushes it.
    let mut app = app.restart();
    app.open_settings("Sound");
    assert_eq!(app.find("sound").checked(), Some(false));
}

#[gpui_kit::test]
fn back_returns_to_the_page_open_before(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with Computer");
    app.find("chess-board");
    app.click("settings");
    app.find("2-0");
    assert!(!app.shows("chess-board"));
    // While Settings is open, the sidebar's first item is Back.
    app.click("0-0");
    app.find("chess-board");
}

#[gpui_kit::test]
fn the_settings_shortcut_opens_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("cmd-,");
    app.find("2-0");
}
