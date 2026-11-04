//! Settings: changes are saved to the profile, shown at once and kept across launches.

use super::Harness;
use gpui_kit::test::TestQueryExt as _;
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
    app.open("Play with computer");
    app.find("chess-board");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    assert!(!app.shows("chess-board"));
    // While Settings is open, the sidebar's first item is Back.
    app.click("0-0");
    app.find("chess-board");
}

#[gpui_kit::test]
fn settings_pressed_again_closes_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    assert!(!app.shows("chess-board"));
    app.click("settings");
    app.find("chess-board");
    assert!(!app.shows(super::SETTINGS_SHOWN));
}

#[gpui_kit::test]
fn the_settings_shortcut_opens_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-,");
    app.find(super::SETTINGS_SHOWN);
}

#[gpui_kit::test]
fn ai_settings_offer_a_chatgpt_plan(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("AI");
    assert_eq!(
        app.find("ai-connect").label(),
        Some("Continue with ChatGPT")
    );
}

#[gpui_kit::test]
fn a_category_opened_in_a_short_window_scrolls_into_the_sidebar(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start_with(cx, |profile| {
        let state = json!({ "x": 0, "y": 0, "width": 1024, "height": 560 });
        std::fs::write(profile.join("window-state.json"), state.to_string())
            .expect("the window state");
    });
    app.with_window(|window, cx| {
        crate::shell::open_settings(crate::pages::settings::Section::Updates, window, cx)
    });
    // Its group is named for its heading.
    let group = app.with_window(|window, _| {
        window
            .try_find_by_label("System")
            .and_then(|group| group.role())
    });
    assert_eq!(group, Some(gpui_kit::Role::Group));
    // Updates, the last category, ends above the footer's Settings.
    let updates = app.find("5-2").bounds();
    let footer = app.find("settings").bounds();
    assert!(
        updates.bottom() <= footer.top(),
        "{updates:?} is under {footer:?}"
    );
    // Closed and opened again on it, Settings shows it again.
    app.click("settings");
    assert!(!app.shows(super::SETTINGS_SHOWN));
    app.click("settings");
    let updates = app.find("5-2").bounds();
    assert!(updates.bottom() <= footer.top());
}
