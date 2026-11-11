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

/// Regression: every opening that ever passed through Recent kept its preview board for the
/// session. Only the popular cards and the recent ones shown keep theirs.
#[gpui_kit::test]
fn openings_that_leave_recent_let_their_preview_go(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::openings::{OpeningChoice, OpeningsPage};
    let mut app = openings(cx);
    let lines = [
        "e2e4 e7e5 f2f4",
        "e2e4 e7e5 f2f4 e5f4",
        "e2e4 g7g6",
        "e2e4 b7b6",
        "e2e4 d7d6",
        "e2e4 e7e5 b1c3",
        "f2f4",
        "b2b3",
        "g2g3",
        "e2e4 g8f6",
    ];
    for line in lines {
        let choice = OpeningChoice {
            name: line.to_string(),
            moves: line.split(' ').map(str::to_string).collect(),
        };
        app.cx
            .update(|cx| crate::prefs::set(cx, "kchess:recent-openings", vec![choice]));
        app.find(format!("recent-{line}"));
    }
    let kept = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<OpeningsPage>(cx)
                .expect("the page")
                .read(cx)
                .kept_previews()
        })
    };
    let shown = kept(&mut app);
    assert_eq!(
        shown.len(),
        13,
        "the twelve popular openings and the one recent"
    );
    // The boards shown stay from one frame to the next, not made afresh each time.
    app.cx.update(|cx| {
        crate::shell::view::<OpeningsPage>(cx)
            .expect("the page")
            .update(cx, |_, cx| cx.notify());
    });
    app.settle();
    assert_eq!(kept(&mut app), shown);
}

#[gpui_kit::test]
fn a_popular_opening_is_trained_in_the_repertoire_trainer(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    reveal(&mut app, "train-Sicilian Defense");
    app.click("train-Sicilian Defense");
    assert_eq!(page(&mut app), Some(Page::Practice));
    assert_eq!(
        app.find(super::tab(
            "practice-tabs",
            crate::pages::practice::Tab::Openings
        ))
        .selected(),
        Some(true)
    );
    app.find("restart-line");
}

#[gpui_kit::test]
fn a_line_can_be_drilled_and_saved_as_a_study(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click(("opening-move", 2usize));
    app.click("train-line");
    assert_eq!(
        app.find(super::tab(
            "practice-tabs",
            crate::pages::practice::Tab::Openings
        ))
        .selected(),
        Some(true)
    );

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
            .request_now(kchess_contracts::calls::Library {})
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

/// An open line's buttons are in Quick Pick: its puzzles open from there.
#[gpui_kit::test]
fn an_open_lines_actions_are_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Train this line");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert_eq!(
        &listed[..5],
        [
            "Train this line",
            "Analyse",
            "Train with puzzles",
            "Flip board",
            "All openings"
        ]
    );
    app.type_text("Train with puzzles");
    super::palette::wait_for_first(&mut app, "Train with puzzles");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Puzzles));
}

/// The repertoire trainer's buttons are in Quick Pick: a drilled line is saved from there.
#[gpui_kit::test]
fn a_drilled_line_is_saved_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click("train-line");
    app.find("save-opening");
    app.press("secondary-k");
    app.type_text(">");
    // Zen mode leads, as on every page it applies to.
    super::palette::wait_for_first(&mut app, "Zen mode");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    // Nothing missed yet: there are no mistakes to forget.
    assert_eq!(
        &listed[1..4],
        ["Show move", "Restart line", "Save as study"]
    );
    app.type_text("Save as study");
    super::palette::wait_for_first(&mut app, "Save as study");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    let studies = loop_until_saved(&mut app);
    assert!(studies.contains("King's Gambit"), "{studies}");
}

/// A move off the line is remembered; Forget mistakes drops it at once and Undo brings it back.
#[gpui_kit::test]
fn forgotten_mistakes_come_back_with_undo(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click("train-line");
    assert_eq!(app.find("forget-mistakes").disabled(), Some(true));
    // The line starts 1. e4; 1. d4 is not in it.
    app.click_square("d2", true);
    app.click_square("d4", true);
    wait_for_status(&mut app, "Not in your repertoire");
    let core = app
        .cx
        .update(|cx| cx.global::<crate::core::AppCore>().bridge.clone());
    let misses = move || {
        core.request_now(kchess_contracts::calls::Library {})
            .map(|library| library["repertoireMisses"].to_string())
            .unwrap_or_default()
    };
    app.wait_for("the miss to be kept", |_, _| misses().contains("rnbqkbnr"));
    app.click("forget-mistakes");
    app.wait_for("the misses to be forgotten", |_, _| {
        !misses().contains("rnbqkbnr")
    });
    assert_eq!(app.find("forget-mistakes").disabled(), Some(true));
    app.click("undo-forget-mistakes");
    app.wait_for("the misses to come back", |_, _| {
        misses().contains("rnbqkbnr")
    });
    app.wait_for("Forget mistakes to be offered again", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("forget-mistakes")
            .is_some_and(|button| button.disabled() != Some(true))
    });
}

/// White's moves of the King's Gambit, 1. e4 e5 2. f4, with Black's reply between them.
fn play_kings_gambit(app: &mut Harness) {
    app.click_square("e2", true);
    app.click_square("e4", true);
    wait_for_status(app, "e4");
    app.wait_for("Black's reply", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("show-move")
            .is_some_and(|show| show.disabled() != Some(true))
    });
    app.click_square("f2", true);
    app.click_square("f4", true);
    wait_for_status(app, "End of this line");
}

/// The panel's line on the schedule, once it reads `text`.
fn wait_for_schedule(app: &mut Harness, text: &str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("the schedule to read {text:?}"), |window, _| {
        window
            .try_find("repertoire-due")
            .is_some_and(|due| due.label() == Some(text))
    });
}

/// A missed line is due again at once and Review due lines drills it; a line finished without a
/// mistake is scheduled a day out, in the core's schedule.
#[gpui_kit::test]
fn missed_lines_are_due_for_review_and_right_ones_wait(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    app.click(("opening-move", 2usize));
    app.click("train-line");
    // Nothing is scheduled in a new profile.
    assert_eq!(app.find("review-due").disabled(), Some(true));
    assert!(!app.shows("repertoire-due"));

    // 1. d4 is not the line: it is missed, so it is due at once.
    app.click_square("d2", true);
    app.click_square("d4", true);
    wait_for_status(&mut app, "Not in your repertoire");
    play_kings_gambit(&mut app);
    wait_for_schedule(&mut app, "1 line due");
    let core = app
        .cx
        .update(|cx| cx.global::<crate::core::AppCore>().bridge.clone());
    let card = move || {
        core.request_now(kchess_contracts::calls::Library {})
            .map(|library| {
                library["repertoireSchedule"]["opening:King's Gambit:white"]["e2e4 e7e5 f2f4"]
                    .clone()
            })
            .unwrap_or_default()
    };
    app.wait_for("the lapse to be kept", |_, _| {
        card()["lapses"].as_f64() == Some(1.)
    });

    // Reviewed right, it waits a day; with nothing else due, the review is done.
    app.click("review-due");
    wait_for_status(&mut app, "Line due for review");
    play_kings_gambit(&mut app);
    wait_for_schedule(&mut app, "Next line due in 1 day");
    app.wait_for("the review to be kept", |_, _| {
        card()["interval"].as_f64() == Some(1.)
    });
    wait_for_status(&mut app, "Due lines reviewed");
    assert_eq!(app.find("review-due").disabled(), Some(true));
}

/// F turns an opening's board, as its button does.
#[gpui_kit::test]
fn f_flips_an_openings_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    let orientation = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::openings::OpeningsPage>(cx)
                .map(|page| page.read(cx).orientation())
        })
    };
    let before = orientation(&mut app).expect("the page");
    app.click("opening-title");
    app.press("f");
    assert_eq!(orientation(&mut app), Some(!before));
}

/// A line opened from the search takes the keys at once: F turns its board, Escape goes back to
/// all openings.
#[gpui_kit::test]
fn escape_goes_back_to_all_openings(cx: &mut gpui_kit::TestAppContext) {
    let mut app = openings(cx);
    open_kieseritzky(&mut app);
    let orientation = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::openings::OpeningsPage>(cx)
                .map(|page| page.read(cx).orientation())
        })
    };
    let before = orientation(&mut app).expect("the page");
    app.press("f");
    assert_eq!(orientation(&mut app), Some(!before));
    app.press("escape");
    app.wait_gone("opening-title");
    app.click_label("Search openings");
}
