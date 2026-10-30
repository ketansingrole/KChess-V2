//! Over the board: two players at one board, and the chess clock.

use super::Harness;

fn board(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Over the board");
    app.find("chess-board");
    app
}

/// Type each move into the move box, as the players take turns at it.
fn type_moves(app: &mut Harness, moves: &[&str]) {
    for (played, text) in moves.iter().enumerate() {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text(text);
        app.press("enter");
        app.wait_for_moves(&moves[..=played]);
    }
}

/// Type into the labelled input of the dialog once it has slid into place.
fn fill_in_dialog(app: &mut Harness, label: &str, text: &str) {
    use gpui_kit::test::TestQueryExt as _;
    let mut last = None;
    app.wait_for(&format!("{label:?} to stop moving"), |window, _| {
        let bounds = window.try_find_by_label(label).map(|input| input.bounds());
        let still = bounds.is_some() && bounds == last;
        last = bounds;
        still
    });
    app.click_label(label);
    app.type_text(text);
}

/// A timed game's clock runs beside the board for the side to move, redrawn on its own each
/// second while the rest of the page keeps its last frame.
#[gpui_kit::test]
fn a_timed_games_clock_runs_for_the_side_to_move(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::Page;
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    app.with_window(|window, cx| {
        let local = crate::shell::view_of::<LocalPage>(Page::Local, cx).expect("the page");
        local.update(cx, |local, cx| {
            let setup =
                serde_json::json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN });
            let clock = serde_json::json!({
                "white": { "minutes": 5, "increment": 0 },
                "black": { "minutes": 5, "increment": 0 },
            });
            local.replace_game(setup, clock, window, cx);
            local.play("e2e4", window, cx);
        });
    });
    let label = |app: &mut Harness, face: usize| {
        app.find(("local-clock", face))
            .label()
            .map(str::to_string)
            .unwrap_or_default()
    };
    app.wait_for("Black's clock to run down", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(("local-clock", 1usize))
            .and_then(|face| face.label().map(|label| label.contains("4:58")))
            == Some(true)
    });
    assert_eq!(label(&mut app, 0), "White clock, 5:00");
    assert_eq!(app.moves(), ["e4"]);
}

const SCHOLARS_MATE: [&str; 7] = ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"];

#[gpui_kit::test]
fn the_players_take_turns_at_one_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    assert_eq!(app.status(), "White to move");
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    assert_eq!(app.status(), "Black to move");
    // The board does not turn: Black plays from the top.
    app.drag_square("e7", "e5", true);
    app.wait_for_moves(&["e4", "e5"]);
    assert_eq!(app.status(), "White to move");
}

#[gpui_kit::test]
fn typed_moves_play_a_game_to_checkmate(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &SCHOLARS_MATE);
    assert_eq!(app.status(), "White wins");
    assert!(
        !app.shows("local-resign") && !app.shows("local-draw"),
        "a finished game cannot be resigned or drawn"
    );
}

#[gpui_kit::test]
fn an_illegal_typed_move_is_refused(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Ke3");
    app.press("enter");
    assert!(app.moves().is_empty());
    assert_eq!(app.status(), "White to move");
    // The refused text stays in the box to correct.
    app.press("secondary-a");
    app.type_text("e4");
    app.press("enter");
    app.wait_for_moves(&["e4"]);
}

#[gpui_kit::test]
fn take_back_undoes_the_last_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    assert!(!app.enabled("local-takeback"));
    type_moves(&mut app, &["e4", "e5"]);
    app.click("local-takeback");
    app.wait_for_moves(&["e4"]);
    assert_eq!(app.status(), "Black to move");
    app.click("local-takeback");
    app.wait_for_moves(&[]);
    assert!(!app.enabled("local-takeback"));
}

#[gpui_kit::test]
fn an_earlier_position_can_be_reviewed_but_not_played_on(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5", "Nf3"]);
    app.click("chevron-left");
    assert_eq!(app.status(), "Reviewing an earlier position");
    assert_eq!(app.find(("move", 1usize)).selected(), Some(true));
    app.click("skip-back");
    assert!(!app.enabled("skip-back"));
    app.click("skip-forward");
    assert_eq!(app.status(), "Black to move");
    assert!(!app.enabled("chevron-right"));
}

#[gpui_kit::test]
fn resigning_asks_first_then_the_other_side_wins(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    assert!(
        !app.shows("local-resign"),
        "nothing to resign before a move"
    );
    type_moves(&mut app, &["e4"]);
    app.click("local-resign");
    app.click("cancel");
    assert_eq!(app.status(), "Black to move");

    // It is Black's turn, so Black resigns.
    app.click("local-resign");
    app.click("ok");
    assert_eq!(app.status(), "White wins");
    assert!(!app.shows("local-resign"));
}

#[gpui_kit::test]
fn a_draw_by_agreement_ends_the_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["d4", "d5"]);
    app.click("local-draw");
    assert_eq!(app.status(), "Draw");
}

#[gpui_kit::test]
fn a_new_game_needs_a_valid_start_and_asks_before_replacing_one(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4"]);
    app.click("local-new");
    fill_in_dialog(&mut app, "Standard start", "not-a-fen");
    app.wait_for("Start to be refused", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("start-local")
            .is_some_and(|start| start.disabled() == Some(true))
    });
    app.click("cancel-local");
    app.wait_for_moves(&["e4"]);

    // Kd2 is legal only from this position, not from the standard start.
    app.click("local-new");
    fill_in_dialog(
        &mut app,
        "Standard start",
        "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1",
    );
    app.click("start-local");
    app.click("cancel");
    app.wait_for_moves(&["e4"]);

    app.click("local-new");
    fill_in_dialog(
        &mut app,
        "Standard start",
        "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1",
    );
    app.click("start-local");
    app.click("ok");
    app.wait_for_moves(&[]);
    app.wait_for("the dialog to close", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.try_find("ok").is_none()
    });
    type_moves(&mut app, &["Kd2"]);
}

#[gpui_kit::test]
fn a_game_in_progress_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "c5", "Nf3"]);
    let mut app = app.restart();
    app.open("Over the board");
    app.wait_for_moves(&["e4", "c5", "Nf3"]);
    assert_eq!(app.status(), "Black to move");
}

#[gpui_kit::test]
fn the_game_can_be_taken_to_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    assert!(!app.enabled("local-analyse"));
    type_moves(&mut app, &["e4", "e5"]);
    app.click("local-analyse");
    // The moves are on the analysis board's tree, the second one under the first.
    app.find("tree-e2e4");
    assert_eq!(app.find("tree-e2e4 e7e5").selected(), Some(true));
}

#[gpui_kit::test]
fn the_game_is_exported_as_an_animated_gif(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    assert!(
        !app.enabled("local-export"),
        "an empty game has nothing to export"
    );
    type_moves(&mut app, &SCHOLARS_MATE);
    app.click("local-export");
    app.click_label("Animated GIF of the game");

    // The save dialog opens once the frames are drawn; the GIF goes where it is pointed.
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_new_path() {
        assert!(started.elapsed() < super::TIMEOUT, "no save dialog opened");
        app.tick();
    }
    let folder = tempfile::tempdir().expect("a folder to save in");
    let path = folder.path().join("game.gif");
    let chosen = path.clone();
    app.cx.simulate_new_path_selection(move |_| Some(chosen));
    // The export is over once Export takes clicks again, the file written by then.
    app.wait_for("Export to take clicks again", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("local-export")
            .is_some_and(|button| button.disabled() != Some(true))
    });
    let gif = std::fs::read(&path).expect("the saved GIF");
    assert_eq!(&gif[..6], b"GIF89a");
}

#[gpui_kit::test]
fn the_clock_runs_after_a_player_presses_their_side(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.click(("local-mode", 1usize));
    let start = |app: &mut Harness| app.find("clock-start").label().map(str::to_string);
    assert_eq!(start(&mut app).as_deref(), Some("Start"));
    assert!(
        app.find(("clock-face", 1usize))
            .label()
            .unwrap()
            .contains("5:00")
    );

    app.click(("clock-face", 1usize));
    assert_eq!(start(&mut app).as_deref(), Some("Pause"));
    // The other player's time is the one running down.
    app.wait_for("Player 2's clock to run down", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(("clock-face", 2usize))
            .and_then(|face| face.label().map(|label| label.contains("4:5")))
            == Some(true)
    });
    assert!(
        app.find(("clock-face", 1usize))
            .label()
            .unwrap()
            .contains("5:00")
    );

    app.click("clock-start");
    assert_eq!(start(&mut app).as_deref(), Some("Start"));
}

#[gpui_kit::test]
fn the_clock_pauses_with_a_key_and_resets(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.click(("local-mode", 1usize));
    app.click(("clock-face", 2usize));
    assert_eq!(app.find("clock-start").label(), Some("Pause"));
    app.press("p");
    assert_eq!(app.find("clock-start").label(), Some("Start"));

    app.click("clock-reset");
    for index in [1usize, 2] {
        assert!(
            app.find(("clock-face", index))
                .label()
                .unwrap()
                .contains("5:00"),
            "Player {index} is back at the set time"
        );
    }
}

#[gpui_kit::test]
fn a_clock_face_can_be_turned_for_the_player_across_the_table(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.click(("local-mode", 1usize));
    app.click(("clock-rotate", 1usize));
    assert_eq!(
        app.find(("clock-rotate", 1usize)).label(),
        Some("Rotate Player 1 clock, currently 90 degrees")
    );
    assert_eq!(
        app.find(("clock-rotate", 2usize)).label(),
        Some("Rotate Player 2 clock, currently 0 degrees")
    );
}
