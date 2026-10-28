//! Play with Computer, against the scripted engine (it always answers `e2e4`, so the computer
//! moves only when it has White).

use super::Harness;

fn computer(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Play with Computer");
    app.find("chess-board");
    app
}

#[gpui_kit::test]
fn a_clicked_move_is_played(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    assert!(!app.enabled("takeback"));
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    assert!(app.enabled("takeback"));
}

#[gpui_kit::test]
fn a_dragged_move_is_played(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.drag_square("d2", "d4", true);
    app.wait_for_moves(&["d4"]);
}

#[gpui_kit::test]
fn a_typed_move_is_played(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Nf3");
    app.press("enter");
    app.wait_for_moves(&["Nf3"]);
}

#[gpui_kit::test]
fn an_illegal_move_is_not_played(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e5", true);
    app.with_window(|_, _| {});
    assert!(app.moves().is_empty());
}

#[gpui_kit::test]
fn the_computer_moves_first_when_you_switch_to_black(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click("flip");
    app.wait_for_moves(&["e4"]);
    // Black at the bottom now.
    app.click_square("e7", false);
    app.click_square("e5", false);
    app.wait_for_moves(&["e4", "e5"]);
}

#[gpui_kit::test]
fn take_back_undoes_your_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.click("takeback");
    app.wait_for_moves(&[]);
    assert!(!app.enabled("takeback"));
}

#[gpui_kit::test]
fn resigning_asks_first_then_ends_the_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);

    app.click("resign");
    app.click("cancel");
    assert!(app.shows("resign"), "cancelling kept the game going");

    app.click("resign");
    app.click("ok");
    let status = app.status();
    assert!(!status.is_empty(), "the result is shown");
    assert!(!app.shows("resign"), "a finished game cannot be resigned");
}

#[gpui_kit::test]
fn new_game_asks_first_then_clears_the_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.click("new-game");
    app.click("ok");
    app.wait_for_moves(&[]);
}

#[gpui_kit::test]
fn moves_can_be_stepped_through(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click("flip");
    app.wait_for_moves(&["e4"]);
    app.click_square("e7", false);
    app.click_square("e5", false);
    app.wait_for_moves(&["e4", "e5"]);
    assert!(!app.enabled("chevron-right"));
    app.click("skip-back");
    assert!(!app.enabled("skip-back"));
    assert_eq!(app.find(("move", 0usize)).selected(), Some(false));
    app.click("chevron-right");
    assert_eq!(app.find(("move", 0usize)).selected(), Some(true));
    app.click("skip-forward");
    assert_eq!(app.find(("move", 1usize)).selected(), Some(true));
}

#[gpui_kit::test]
fn an_unfinished_game_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click("flip");
    app.wait_for_moves(&["e4"]);
    app.click_square("e7", false);
    app.click_square("e5", false);
    app.wait_for_moves(&["e4", "e5"]);
    let mut app = app.restart();
    app.open("Play with Computer");
    app.wait_for_moves(&["e4", "e5"]);
}
