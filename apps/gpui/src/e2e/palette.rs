//! Quick Pick (⌘K, ⇧⌘P) and the interface zoom shortcuts.

use super::Harness;
use crate::pages::Page;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use serde_json::json;

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Whether a result with this name is listed.
fn lists(app: &mut Harness, name: &str) -> bool {
    app.with_window(|window, _| !window.find_all_by_label(name).is_empty())
}

/// Wait until a result with this name is listed.
fn wait_until_listed(app: &mut Harness, name: &str) {
    app.wait_for(&format!("{name:?} to be listed"), |window, _| {
        !window.find_all_by_label(name).is_empty()
    });
}

/// Wait until the best result is named `name`.
fn wait_for_first(app: &mut Harness, name: &str) {
    app.wait_for(&format!("{name:?} as the first result"), |window, _| {
        window
            .try_find(("palette-item", 0usize))
            .is_some_and(|item| item.label() == Some(name))
    });
}

/// Wait until the palette is closed.
fn wait_until_closed(app: &mut Harness) {
    app.wait_for("the palette to close", |window, _| {
        window.try_find(("palette-item", 0usize)).is_none()
    });
}

#[gpui_kit::test]
fn quick_pick_opens_with_its_modes_and_closes_on_escape(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert!(!lists(&mut app, "Run a command"));
    app.press("secondary-k");
    for mode in ["Run a command", "Find a player", "Change a setting"] {
        app.click_label(mode);
        // A mode row switches the mode and leaves the palette open.
        app.press("backspace");
    }
    app.press("escape");
    wait_until_closed(&mut app);
}

#[gpui_kit::test]
fn a_press_beside_the_open_box_closes_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("puzzles");
    wait_for_first(&mut app, "Puzzles");
    // The title bar's empty side, left of the box (its icon and padding are 32px): nothing
    // there takes the focus.
    let field = app.with_window(|window, _| {
        window
            .try_find_by_label("Search pages and commands")
            .expect("the search field")
            .bounds()
    });
    app.click_point(gpui_kit::point(
        field.origin.x - gpui_kit::px(52.),
        field.center().y,
    ));
    wait_until_closed(&mut app);
    // It gave up the focus too: typing does not reopen it.
    app.type_text("p");
    assert!(!lists(&mut app, "Puzzles"));
}

#[gpui_kit::test]
fn a_page_is_found_by_name_and_opened(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("puzzles");
    wait_for_first(&mut app, "Puzzles");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Puzzles));
}

#[gpui_kit::test]
fn acronyms_and_several_words_find_pages_and_commands(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("otb");
    wait_for_first(&mut app, "Over the board");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Local));

    app.press("secondary-k");
    app.type_text("stockfish black");
    app.wait_for("a Stockfish game as the first result", |window, _| {
        window.try_find(("palette-item", 0usize)).and_then(|item| {
            item.label()
                .map(|label| label.starts_with("Play Stockfish"))
        }) == Some(true)
    });
}

#[gpui_kit::test]
fn commands_are_run_from_the_command_mode(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.click_label("Run a command");
    wait_until_listed(&mut app, "New analysis board");
    app.type_text(" new analysis");
    wait_for_first(&mut app, "New analysis board");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Analysis));

    // The shortcut opens straight into the commands; Backspace goes back to searching pages.
    app.press("shift-secondary-p");
    wait_until_listed(&mut app, "New analysis board");
    assert!(!lists(&mut app, "Home"));
    app.press("backspace");
    wait_until_listed(&mut app, "Run a command");
}

#[gpui_kit::test]
fn a_setting_is_changed_from_the_palette_and_saved(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.saved_settings()["soundEnabled"], json!(true));
    app.press("secondary-k");
    app.type_text("#sound");
    wait_for_first(&mut app, "Sound");
    app.click(("palette-item", 0usize));
    wait_until_closed(&mut app);
    app.wait_for_saved("soundEnabled", json!(false));
}

#[gpui_kit::test]
fn the_commands_used_last_are_offered_first_and_kept(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    assert!(!lists(&mut app, "New analysis board"));
    app.press("escape");
    wait_until_closed(&mut app);

    app.press("shift-secondary-p");
    app.type_text(" new analysis");
    wait_for_first(&mut app, "New analysis board");
    app.press("enter");
    wait_until_closed(&mut app);

    app.press("secondary-k");
    wait_until_listed(&mut app, "New analysis board");
    app.press("escape");
    wait_until_closed(&mut app);

    // A restart keeps it.
    let mut app = app.restart();
    app.press("secondary-k");
    wait_until_listed(&mut app, "New analysis board");
}

#[gpui_kit::test]
fn the_interface_zoom_steps_with_the_keys_and_is_kept(cx: &mut gpui_kit::TestAppContext) {
    use crate::interface_zoom::{InterfaceZoom, Setting};
    fn setting(app: &mut Harness) -> Setting {
        app.cx.update(|cx| cx.global::<InterfaceZoom>().setting)
    }
    fn saved(app: &Harness) -> serde_json::Value {
        let file = app.profile.as_ref().expect("the profile").path();
        let text = std::fs::read_to_string(file.join("interface-zoom.json")).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_default()
    }
    fn wait_for_setting(app: &mut Harness, wanted: Setting) {
        let mut shown = setting(app);
        app.wait_for(&format!("the zoom to be {wanted:?}"), |_, cx| {
            shown = cx.global::<InterfaceZoom>().setting;
            shown == wanted
        });
    }

    let mut app = Harness::start(cx);
    assert_eq!(setting(&mut app), Setting::Auto);
    let start = app.with_window(|window, cx| crate::interface_zoom::percent(window, cx));

    app.press("secondary-=");
    let larger = crate::interface_zoom::step(start, 1);
    wait_for_setting(&mut app, Setting::Percent(larger));
    assert_eq!(saved(&app)["zoom"], json!(larger));

    app.press("secondary--");
    app.press("secondary--");
    let smaller = crate::interface_zoom::step(start, -1);
    wait_for_setting(&mut app, Setting::Percent(smaller));
    assert_eq!(saved(&app)["zoom"], json!(smaller));

    // The choice is there again after a restart, and ⌘0 goes back to Auto.
    let mut app = app.restart();
    assert_eq!(setting(&mut app), Setting::Percent(smaller));
    app.press("secondary-0");
    wait_for_setting(&mut app, Setting::Auto);
    assert_eq!(saved(&app)["zoom"], json!("auto"));
}
