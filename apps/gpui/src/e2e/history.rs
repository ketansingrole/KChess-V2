//! Game history: finished games listed, opened and kept.

use super::Harness;

const SCHOLARS_MATE: [&str; 7] = ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"];

/// Play moves over the board by typing them, from wherever the app is.
fn play_over_the_board(app: &mut Harness, moves: &[&str]) {
    app.open("Over the board");
    for (played, text) in moves.iter().enumerate() {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text(text);
        app.press("enter");
        app.wait_for_moves(&moves[..=played]);
    }
}

/// Game history with the games played over the board listed.
fn board_games(app: &mut Harness) {
    app.open("Game history");
    app.click(("history-source", 1usize));
}

/// A finished game of Scholar's mate, then history over the board.
fn mate_in_history(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    play_over_the_board(&mut app, &SCHOLARS_MATE);
    board_games(&mut app);
    app
}

#[gpui_kit::test]
fn an_empty_history_offers_a_game_to_play(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game history");
    assert_eq!(app.find("history-play").label(), Some("Play the computer"));
    app.click("history-play");
    app.find("takeback");

    app.open("Game history");
    app.click(("history-source", 1usize));
    assert_eq!(
        app.find("history-play").label(),
        Some("Play over the board")
    );
    app.click("history-play");
    app.find("local-new");
}

#[gpui_kit::test]
fn a_finished_board_game_is_listed_and_opens_to_its_moves(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    assert_eq!(
        app.find(("history-row", 0usize)).label(),
        Some("White vs Black")
    );
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    // The game opens at its last move.
    assert_eq!(app.find(("move", 6usize)).selected(), Some(true));
    assert!(!app.enabled("chevron-right"));
}

#[gpui_kit::test]
fn an_open_game_is_stepped_through_and_closed_to_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.click("skip-back");
    assert!(!app.enabled("skip-back"));
    app.click("chevron-right");
    assert_eq!(app.find(("move", 0usize)).selected(), Some(true));
    app.click("skip-forward");
    assert_eq!(app.find(("move", 6usize)).selected(), Some(true));

    app.click("all-games");
    app.find(("history-row", 0usize));
    assert!(!app.shows("all-games"));
}

#[gpui_kit::test]
fn finished_games_are_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let app = mate_in_history(cx);
    let mut app = app.restart();
    board_games(&mut app);
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
}

#[gpui_kit::test]
fn a_saved_game_opens_on_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.click("open-analysis");
    // The whole game is in the tree.
    app.find("tree-e2e4");
    app.find("tree-e2e4 e7e5 d1h5 b8c6 f1c4 g8f6 h5f7");
}

#[gpui_kit::test]
fn a_resigned_computer_game_is_listed_with_the_computer_games(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with Computer");
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.click("resign");
    app.click("ok");
    app.find("new-game");

    app.open("Game history");
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&["e4"]);

    // Over the board has none of it.
    app.click("all-games");
    app.click(("history-source", 1usize));
    app.find("history-play");
    assert!(!app.shows(("history-row", 0usize)));
}

#[gpui_kit::test]
fn the_list_is_narrowed_by_result(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    // A second game, left unfinished after one move.
    app.open("Over the board");
    app.click("local-new");
    app.click("start-local");
    app.wait_for_moves(&[]);
    play_over_the_board(&mut app, &["e4"]);
    app.open("Game history");
    app.find(("history-row", 1usize));

    app.click(("history-filter", 1usize));
    assert_eq!(app.find(("history-filter", 1usize)).selected(), Some(true));
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    app.click("all-games");

    app.click(("history-filter", 4usize));
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&["e4"]);
    app.click("all-games");

    // No game was won by Black, so that filter leads nowhere.
    app.click(("history-filter", 2usize));
    assert_eq!(app.find(("history-filter", 2usize)).selected(), Some(false));
    assert_eq!(app.find(("history-filter", 4usize)).selected(), Some(true));
    app.click(("history-filter", 0usize));
    app.find(("history-row", 1usize));
}

#[gpui_kit::test]
fn deleting_a_game_asks_first_and_is_kept_after_a_restart(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    let actions = "Actions for the game from ";
    let label = app.with_window(|window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .elements()
            .iter()
            .filter_map(|element| element.label())
            .find(|label| label.starts_with(actions))
            .map(str::to_string)
    });
    let label = label.expect("the game's actions");

    app.click_label(&label);
    app.click_label("Delete");
    app.click("delete-cancel");
    app.find(("history-row", 0usize));

    app.click_label(&label);
    app.click_label("Delete");
    app.click("delete-game");
    app.find("history-play");
    assert!(!app.shows(("history-row", 0usize)));

    let mut app = app.restart();
    board_games(&mut app);
    app.find("history-play");
}
