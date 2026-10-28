//! Practice, offline: the board-vision drills and the trainers that need no account.

use super::Harness;
use crate::pages::Page;

fn practice(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Practice");
    app
}

/// Open a tab of the page ("Knight paths" is 2).
fn tab(app: &mut Harness, index: usize) {
    app.click(("practice-tabs", index));
    assert_eq!(app.find(("practice-tabs", index)).selected(), Some(true));
}

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Wait until the element's accessible label is `label`.
fn wait_for_label(app: &mut Harness, id: &'static str, label: &str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("{id} to read {label:?}"), |window, _| {
        window
            .try_find(id)
            .is_some_and(|element| element.label() == Some(label))
    });
}

/// Scroll `list` until `id` is in view.
fn reveal(app: &mut Harness, list: &'static str, id: &'static str) {
    use gpui_kit::test::TestWindowExt as _;
    for _ in 0..30 {
        if app.shows(id) {
            return;
        }
        app.with_window(|window, cx| {
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-300.));
            window.scroll(list, gpui_kit::ScrollDelta::Pixels(by), cx);
        });
    }
    app.find(id);
}

/// Whether a square is light, as a name like "e4" says it.
fn is_light(square: &str) -> bool {
    let bytes = square.as_bytes();
    (bytes[0] - b'a' + bytes[1] - b'1') % 2 == 1
}

#[gpui_kit::test]
fn every_trainer_opens_without_an_account(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    assert_eq!(app.find(("practice-tabs", 0usize)).selected(), Some(true));
    app.find("coordinates-start");
    tab(&mut app, 1);
    app.find("square-colors-start");
    tab(&mut app, 2);
    app.find("knight-start");
    tab(&mut app, 3);
    app.find("card-checkmate-patterns-1");
    tab(&mut app, 4);
    app.find("open-studies");
    tab(&mut app, 5);
    app.find("review-a-game");
}

#[gpui_kit::test]
fn a_coordinates_run_scores_the_clicked_square(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    app.click("coordinates-start");
    let target = app.find("coordinates-target").label().unwrap().to_string();
    assert_eq!(target.len(), 2, "a square is asked for: {target:?}");
    assert_eq!(app.find("run-score").label(), Some("0"));

    // A wrong square scores nothing.
    let wrong = if target == "a1" { "h8" } else { "a1" };
    app.click_square(wrong, true);
    assert_eq!(app.find("run-score").label(), Some("0"));

    let target = app.find("coordinates-target").label().unwrap().to_string();
    app.click_square(&target, true);
    wait_for_label(&mut app, "run-score", "1");

    app.click("coordinates-end");
    assert_eq!(app.status(), "1 correct · 50% accuracy");
    assert!(app.enabled("coordinates-start"));
}

#[gpui_kit::test]
fn a_coordinates_run_can_be_limited_to_some_files_and_ranks(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    app.click("coordinates-limited");
    assert_eq!(app.find("coordinates-limited").checked(), Some(true));
    for file in "bcdefgh".chars() {
        app.click(format!("kchess:coordinates-files-{file}"));
    }
    for rank in "2345678".chars() {
        app.click(format!("kchess:coordinates-ranks-{rank}"));
    }
    app.click("coordinates-start");
    // Only a1 is left, so every question is a1.
    assert_eq!(app.find("coordinates-target").label(), Some("a1"));
    app.click_square("a1", true);
    wait_for_label(&mut app, "run-score", "1");
    assert_eq!(app.find("coordinates-target").label(), Some("a1"));
    app.click("coordinates-end");
    assert_eq!(app.status(), "1 correct · 100% accuracy");
}

#[gpui_kit::test]
fn the_square_colour_drill_scores_keys_and_buttons(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 1);
    app.click("square-colors-start");
    assert_eq!(app.find("square-score").label(), Some("0 correct"));

    // The right key counts.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.press(if is_light(&square) { "l" } else { "d" });
    wait_for_label(&mut app, "square-score", "1 correct");

    // The wrong button does not.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.click(if is_light(&square) {
        "answer-dark"
    } else {
        "answer-light"
    });
    assert_eq!(app.find("square-score").label(), Some("1 correct"));

    // The right button does.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.click(if is_light(&square) {
        "answer-light"
    } else {
        "answer-dark"
    });
    wait_for_label(&mut app, "square-score", "2 correct");
}

#[gpui_kit::test]
fn a_knight_run_asks_for_a_path_and_a_skip_shows_the_shortest(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 2);
    app.click("knight-start");
    let target = app.find("knight-target").label().unwrap().to_string();
    assert!(target.starts_with("Reach "), "{target:?}");
    app.click("knight-skip");
    let notice = app.find("knight-notice").label().unwrap().to_string();
    assert!(
        notice.starts_with("Skipped — the shortest path was"),
        "{notice:?}"
    );
    app.click("knight-end");
    assert_eq!(app.status(), "0 shortest paths · 0 reached");
    assert!(app.enabled("knight-start"));
}

#[gpui_kit::test]
fn endgame_drills_open_and_return_to_the_courses(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    reveal(&mut app, "endgame-courses", "card-drills");
    app.click("card-drills");
    app.find("chess-board");
    app.find("drill-hint");
    assert!(!app.shows("card-drills"));
    app.click("drills-back");
    app.find("endgame-courses");
    assert!(!app.shows("chess-board"));
}

#[gpui_kit::test]
fn a_checkmate_lesson_takes_back_a_wrong_move_and_accepts_the_mate(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    app.click("card-checkmate-patterns-2");
    app.click("training-cp2-balestra");
    app.click("lesson-next");
    assert_eq!(app.status(), "Your turn");
    app.drag_square("d3", "c4", true);
    wait_for_label(&mut app, "status-banner", "Not the move — try again");
    // The refused bishop is back on d3 to make the mate.
    app.drag_square("d3", "b5", true);
    wait_for_label(&mut app, "status-banner", "Checkmate!");
    app.click("lesson-back");
    app.find("card-checkmate-patterns-2");
}

#[gpui_kit::test]
fn an_empty_repertoire_and_empty_mistakes_lead_elsewhere(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 4);
    assert!(app.enabled("browse-openings"));
    app.click("open-studies");
    assert_eq!(page(&mut app), Some(Page::Studies));

    app.open("Practice");
    tab(&mut app, 5);
    assert!(!app.shows("practice-due"));
    app.click("review-a-game");
    assert_eq!(page(&mut app), Some(Page::History));
}
