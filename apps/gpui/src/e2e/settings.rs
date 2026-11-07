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
        .map(|board| board.id.clone())
        .find(|id| *id != app.saved_settings()["boardTheme"])
        .expect("another board");
    let option = format!("board-{other}");
    // The dialog's settings scroll; the board may sit below its fold.
    let dialog = app.find(super::SETTINGS_SHOWN).bounds();
    for _ in 0..20 {
        let shown = app.with_window(|window, _| {
            gpui_kit::test::TestWindowExt::try_find(
                window,
                gpui_kit::ElementId::from(option.clone()),
            )
            .is_some_and(|e| e.visible() && e.bounds().bottom() <= dialog.bottom())
        });
        if shown {
            break;
        }
        app.with_window(|window, cx| {
            use gpui_kit::test::TestWindowExt as _;
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-200.));
            window.scroll("settings-page", gpui_kit::ScrollDelta::Pixels(by), cx);
        });
    }
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

/// Settings opens as a dialog over the page it was opened from, which stays in sight under it,
/// and closing it returns there.
#[gpui_kit::test]
fn settings_opens_over_the_page_and_closes_back_to_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.find("chess-board");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    assert!(app.shows("chess-board"));
    app.click("settings-close");
    app.wait_gone(super::SETTINGS_SHOWN);
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Computer)
    );
}

/// Escape, or a press outside the dialog, closes Settings as its close button does.
#[gpui_kit::test]
fn escape_or_a_press_outside_closes_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.click("settings");
    app.find(super::SETTINGS_SHOWN);
    app.press("escape");
    app.wait_gone(super::SETTINGS_SHOWN);
    app.click("settings");
    let dialog = app.find(super::SETTINGS_SHOWN).bounds();
    // A press inside it keeps it open.
    app.click_point(dialog.center());
    assert!(app.shows(super::SETTINGS_SHOWN));
    let outside = gpui_kit::point(dialog.left() - gpui_kit::px(8.), dialog.center().y);
    app.click_point(outside);
    app.wait_gone(super::SETTINGS_SHOWN);
    app.find("chess-board");
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
fn a_category_opened_in_a_short_window_scrolls_into_its_list(cx: &mut gpui_kit::TestAppContext) {
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
    // Updates, the last category, is scrolled into the dialog's list.
    let updates = app.find("s5-2").bounds();
    let list = app.find("settings-sections").bounds();
    assert!(
        updates.bottom() <= list.bottom(),
        "{updates:?} is under {list:?}"
    );
    // Closed and opened again on it, Settings shows it again.
    app.click("settings-close");
    assert!(!app.shows(super::SETTINGS_SHOWN));
    app.click("settings");
    let updates = app.find("s5-2").bounds();
    assert!(updates.bottom() <= list.bottom());
}

#[gpui_kit::test]
fn moves_can_be_shown_with_piece_symbols(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let symbols = ("notation", 1usize);
    assert_eq!(app.find(("notation", 0usize)).selected(), Some(true));
    app.click(symbols);
    assert_eq!(app.find(symbols).selected(), Some(true));
    app.wait_for_saved("figurineNotation", json!(true));
    assert!(app.cx.update(|cx| crate::notation::figurine(cx)).is_some());
    // Only what is shown changes: a screen reader still hears the move in letters.
    app.click("settings-close");
    app.open("Analysis board");
    app.find("chess-board");
    app.click_square("g1", true);
    app.click_square("f3", true);
    assert_eq!(app.find("tree-g1f3").label(), Some("Nf3"));
}

/// ⌘K finds a category by any row title it shows, so a new row is found as soon as it is added.
#[gpui_kit::test]
fn every_settings_row_is_found_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::settings::{DRAWN_TITLES, SettingsPage, section_keywords, sections};
    let mut app = Harness::start(cx);
    let mut checked = 0;
    for (section, label, _) in sections() {
        app.open_settings(label);
        app.with_window(|_, _| ());
        // Only this category's titles: redraw it on its own.
        DRAWN_TITLES.with(|titles| titles.borrow_mut().clear());
        app.with_window(|_, cx| {
            if let Some(page) =
                crate::shell::view_of::<SettingsPage>(crate::pages::Page::Settings, cx)
            {
                page.update(cx, |_, cx| cx.notify());
            }
        });
        app.with_window(|_, _| ());
        let keywords = section_keywords(section).to_lowercase();
        for title in DRAWN_TITLES.with(|titles| titles.borrow().clone()) {
            assert!(
                keywords.contains(&title.to_lowercase()),
                "{label}: {title:?} is missing from section_keywords"
            );
            checked += 1;
        }
    }
    assert!(checked > 40, "only {checked} row titles were drawn");
}

#[gpui_kit::test]
fn a_row_title_finds_its_category_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("#piece notation");
    super::palette::wait_for_first(&mut app, "Appearance");
    app.press("enter");
    app.find(("notation", 1usize));
}

/// Settings → Updates links to the website, the source and the issue tracker.
#[gpui_kit::test]
fn updates_links_to_the_website_source_and_issues(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Updates");
    for (id, label) in [
        ("about-website", "Website"),
        ("about-source", "Source code"),
        ("about-issues", "Report a problem"),
    ] {
        let link = app.find(id);
        assert_eq!(link.label(), Some(label));
        assert_eq!(link.role(), Some(gpui_kit::Role::Link));
    }
}
