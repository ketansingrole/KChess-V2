//! Openings, browsed and drilled offline: the names come with the app, only the explorer's
//! statistics need a Lichess account.

use super::Harness;
use crate::pages::Page;

fn openings(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Openings");
    app
}

const KIESERITZKY: &str = "King's Gambit Accepted: Kieseritzky Gambit";

/// Search for the Kieseritzky Gambit and open it.
fn open_kieseritzky(app: &mut Harness) {
    app.click_label("Search openings");
    app.type_text("kieseritzky");
    app.click(format!("result-{KIESERITZKY}"));
    assert_eq!(app.find("opening-title").label(), Some(KIESERITZKY));
}

/// Scroll the list of openings, down and then back up, until `id` is in view.
fn reveal(app: &mut Harness, id: impl Into<gpui_kit::ElementId>) {
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    for step in (0..40).map(|_| -300.).chain((0..80).map(|_| 300.)) {
        if app.shows(id.clone()) {
            return;
        }
        app.with_window(|window, cx| {
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(step));
            window.scroll("openings", gpui_kit::ScrollDelta::Pixels(by), cx);
        });
    }
    app.find(id);
}

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Wait until the status banner's title begins with `title`.
fn wait_for_status(app: &mut Harness, title: &str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("a status beginning {title:?}"), |window, _| {
        window
            .try_find("status-banner")
            .is_some_and(|banner| banner.label().is_some_and(|label| label.starts_with(title)))
    });
}

#[gpui_kit::test]
fn the_families_are_listed_under_their_first_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    assert_eq!(app.find(("openings-group", 0usize)).selected(), Some(true));
    reveal(&mut app, "toggle-King's Gambit");
    assert!(!app.shows("open-Slav Defense"));

    // A family's variations open and close.
    assert!(!app.shows("open-King's Gambit Accepted"));
    app.click("toggle-King's Gambit");
    app.find("open-King's Gambit Accepted");
    app.click("toggle-King's Gambit");
    assert!(!app.shows("open-King's Gambit Accepted"));

    reveal(&mut app, ("openings-group", 1usize));
    app.click(("openings-group", 1usize));
    reveal(&mut app, "open-Slav Defense");
    assert!(!app.shows("toggle-King's Gambit"));
}

#[gpui_kit::test]
fn a_search_finds_an_opening_and_its_moves_can_be_stepped_back(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    // Without an account the line's moves are there to step through.
    assert_eq!(app.find(("opening-move", 0usize)).label(), Some("e4"));
    assert_eq!(app.find(("opening-move", 8usize)).label(), Some("Ne5"));
    app.click(("opening-move", 2usize));
    assert_eq!(app.find("opening-title").label(), Some("King's Gambit"));
    assert!(!app.shows(("opening-move", 3usize)));
}

#[gpui_kit::test]
fn the_openings_just_seen_are_listed_and_kept(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    assert!(!app.shows(format!("recent-{KIESERITZKY}")));
    open_kieseritzky(&mut app);
    app.click(("opening-move", 2usize));
    app.click("all-openings");
    app.find("recent-King's Gambit");
    app.find(format!("recent-{KIESERITZKY}"));

    // A recent opening opens again, also after the app has been restarted.
    let mut app = app.restart();
    app.open("Openings");
    app.find("recent-King's Gambit");
    app.click(format!("recent-{KIESERITZKY}"));
    assert_eq!(app.find("opening-title").label(), Some(KIESERITZKY));
}

#[gpui_kit::test]
fn a_popular_opening_is_trained_in_the_repertoire_trainer(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    reveal(&mut app, "train-Sicilian Defense");
    app.click("train-Sicilian Defense");
    assert_eq!(page(&mut app), Some(Page::Practice));
    assert_eq!(app.find(("practice-tabs", 4usize)).selected(), Some(true));
    app.find("restart-line");
}

#[gpui_kit::test]
fn a_line_can_be_drilled_and_saved_as_a_study(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click(("opening-move", 2usize));
    app.click("train-line");
    assert_eq!(app.find(("practice-tabs", 4usize)).selected(), Some(true));

    // White plays the line's moves; Black answers from it.
    app.click_square("e2", true);
    app.click_square("e4", true);
    wait_for_status(&mut app, "e4");
    app.wait_for("Black's reply", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("show-move")
            .is_some_and(|show| show.disabled() != Some(true))
    });
    app.click_square("f2", true);
    app.click_square("f4", true);
    wait_for_status(&mut app, "End of this line");

    app.click("save-opening");
    let studies = loop_until_saved(&mut app);
    assert!(studies.contains("King's Gambit"), "{studies}");
}

/// The library as the core has it, once the study was saved to it.
fn loop_until_saved(app: &mut Harness) -> String {
    let core = app
        .cx
        .update(|cx| cx.global::<crate::core::AppCore>().bridge.clone());
    let mut shown = String::new();
    app.wait_for("the study to be saved", |_, _| {
        shown = core
            .call_now("library", Vec::new())
            .map(|library| library.to_string())
            .unwrap_or_default();
        shown.contains("King's Gambit")
    });
    shown
}

#[gpui_kit::test]
fn an_opening_can_be_analysed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click("analyse-line");
    assert_eq!(page(&mut app), Some(Page::Analysis));
}
