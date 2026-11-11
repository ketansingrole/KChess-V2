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

/// A timed game's clock runs beside the board for the side to move, redrawn on its own each
/// second while the rest of the page keeps its last frame.
#[gpui_kit::test]
fn a_timed_games_clock_runs_for_the_side_to_move(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    app.with_window(|window, cx| {
        let local = crate::shell::view::<LocalPage>(cx).expect("the page");
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
    // Any second under five minutes: a loaded machine can draw 4:59 and next 4:57.
    app.wait_for_in_real_time("Black's clock to run down", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(("local-clock", 1usize))
            .and_then(|face| face.label().map(|label| label.contains("4:5")))
            == Some(true)
    });
    assert_eq!(label(&mut app, 0), "White clock, 5:00");
    assert_eq!(app.moves(), ["e4"]);
}

const SCHOLARS_MATE: [&str; 7] = ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"];

#[gpui_kit::test]
fn an_empty_move_list_mentions_the_clock_only_in_a_timed_game(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    use gpui_kit::test::TestWindowExt as _;
    let mut app = board(cx);
    assert_eq!(app.find("moves-empty").label(), Some("No moves yet"));
    app.with_window(|window, cx| {
        let local = crate::shell::view::<LocalPage>(cx).expect("the page");
        local.update(cx, |local, cx| {
            let setup =
                serde_json::json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN });
            let clock = serde_json::json!({
                "white": { "minutes": 5, "increment": 0 },
                "black": { "minutes": 5, "increment": 0 },
            });
            local.replace_game(setup, clock, window, cx);
        });
    });
    app.wait_for("the clock's hint", |window, _| {
        window
            .try_find("moves-empty")
            .and_then(|empty| empty.label().map(str::to_string))
            .as_deref()
            == Some("The first move starts the clock")
    });
}

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

/// Copy PGN writes the game with today's date: it was always "????.??.??", from a rules
/// method that did not exist.
#[gpui_kit::test]
fn copy_pgn_dates_the_game_today(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5"]);
    app.click("copy-pgn");
    let pgn = app
        .cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("a copied PGN");
    let today = chrono::Local::now().format("%Y.%m.%d").to_string();
    assert!(pgn.contains(&format!("[Date \"{today}\"]")), "{pgn}");
    assert!(pgn.contains("1. e4 e5"), "{pgn}");
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
    assert!(!app.shows("back-to-game"));
}

#[gpui_kit::test]
fn the_review_banner_goes_back_to_the_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5", "Nf3"]);
    app.click("skip-back");
    assert_eq!(app.status(), "Reviewing an earlier position");
    app.click("back-to-game");
    assert_eq!(app.status(), "Black to move");
    assert!(!app.shows("back-to-game"));
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
    app.fill_in_dialog("Start from a FEN (optional)", "not-a-fen");
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
    app.fill_in_dialog(
        "Start from a FEN (optional)",
        "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1",
    );
    app.click("start-local");
    app.click("cancel");
    app.wait_for_moves(&["e4"]);

    app.click("local-new");
    app.fill_in_dialog(
        "Start from a FEN (optional)",
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

/// A moment after the window shows, Over the board and the analysis board are made ahead of
/// being opened (`shell::prewarm`): the page then opened is the one made, resuming its game, and
/// neither is shown until it is chosen.
#[gpui_kit::test]
fn a_page_made_ahead_opens_with_its_game(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::analysis::AnalysisPage;
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "c5", "Nf3"]);
    let mut app = app.restart();
    let made = |app: &mut Harness| {
        app.with_window(|_, cx| {
            (
                crate::shell::view::<LocalPage>(cx).map(|page| page.entity_id()),
                crate::shell::view::<AnalysisPage>(cx).is_some(),
            )
        })
    };
    assert_eq!(made(&mut app), (None, false), "made only when asked");
    app.with_window(crate::shell::prewarm);
    let (local, analysis) = made(&mut app);
    assert!(local.is_some() && analysis, "both pages are made");
    assert!(!app.shows("local-new"), "and neither is shown");
    app.open("Over the board");
    app.wait_for_moves(&["e4", "c5", "Nf3"]);
    assert_eq!(
        made(&mut app).0,
        local,
        "the page made ahead is the one opened"
    );
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

/// Wait for the save dialog and answer it with `name` in `folder`; the path chosen.
fn save_as(app: &mut Harness, folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_new_path() {
        assert!(started.elapsed() < super::TIMEOUT, "no save dialog opened");
        app.tick();
    }
    let path = folder.join(name);
    let chosen = path.clone();
    app.cx.simulate_new_path_selection(move |_| Some(chosen));
    path
}

/// An export cancelled while its save dialog is open, and finished after the next export started,
/// leaves the next one running: its file is offered and written too.
#[gpui_kit::test]
fn a_cancelled_export_leaves_the_next_one_alone(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &SCHOLARS_MATE);
    app.click("local-export");
    app.click_label("Animated GIF of the game");
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_new_path() {
        assert!(started.elapsed() < super::TIMEOUT, "no save dialog opened");
        app.tick();
    }
    app.click_label("Cancel export");
    app.click("local-export");
    app.click_label("PNG of this position");

    let folder = tempfile::tempdir().expect("a folder to save in");
    save_as(&mut app, folder.path(), "game.gif");
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_new_path() {
        assert!(
            started.elapsed() < super::TIMEOUT,
            "no save dialog for the PNG"
        );
        app.tick();
    }
    // The cancelled export finishing left the running one shown as running.
    assert!(
        app.with_window(|window, _| {
            use gpui_kit::test::TestWindowExt as _;
            window.try_find(("local-export", 1usize)).is_some()
        }),
        "the PNG export can still be cancelled"
    );
    let png = save_as(&mut app, folder.path(), "position.png");
    app.wait_for("Export to take clicks again", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("local-export")
            .is_some_and(|button| button.disabled() != Some(true))
    });
    let saved = std::fs::read(&png).expect("the PNG");
    assert_eq!(&saved[..4], b"\x89PNG");
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

/// Every square is a whole number of device pixels and starts on one, at any window size and
/// interface size. A 798px board (Puzzles in a 1936×1048 window on a 96 DPI display) made
/// squares of 99.75px: each piece, rasterized at 100px, was resampled into its square and blurred.
#[gpui_kit::test]
fn the_boards_squares_land_on_whole_pixels(cx: &mut gpui_kit::TestAppContext) {
    use crate::interface_zoom::{Setting, choose};
    use gpui_kit::{px, size};
    let mut app = board(cx);
    for percent in [100, 110, 125] {
        app.with_window(|window, cx| choose(Setting::Percent(percent), window, cx));
        for (width, height) in [(1920., 1017.), (1321., 877.), (1023., 769.), (641., 619.)] {
            app.cx
                .simulate_window_resize(app.window, size(px(width), px(height)));
            for _ in 0..4 {
                app.tick();
            }
            let scale = app.with_window(|window, _| window.scale_factor());
            let board = app.find("chess-board").bounds();
            let square = board.size.width.as_f32() * scale / 8.;
            let left = board.origin.x.as_f32() * scale;
            let top = board.origin.y.as_f32() * scale;
            let whole = |value: f32| (value - value.round()).abs() < 0.01;
            assert!(
                whole(square) && whole(left) && whole(top),
                "at {width}×{height}, {percent}%: squares of {square} device pixels from \
                 ({left}, {top})"
            );
        }
    }
}

/// Over the board, Crazyhouse captures fill the pockets, and a picked pocket piece drops onto a
/// square for the side to move: the move list writes a pawn drop as `@e3`.
#[gpui_kit::test]
fn a_crazyhouse_capture_can_be_dropped_back(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    app.with_window(|window, cx| {
        let local = crate::shell::view::<LocalPage>(cx).expect("the page");
        local.update(cx, |local, cx| {
            let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
            let setup = serde_json::json!({ "variant": "crazyhouse", "fen": fen });
            local.replace_game(setup, serde_json::Value::Null, window, cx);
        });
    });
    type_moves(&mut app, &["e4", "d5", "exd5", "Qxd5"]);
    app.click("pocket-White-Pawn");
    app.click_square("e3", true);
    app.wait_for_moves(&["e4", "d5", "exd5", "Qxd5", "P@e3"]);
    assert_eq!(app.status(), "Black to move");
    // Black's pawn is still in hand; White's was spent.
    assert!(
        app.find("pocket-Black-Pawn")
            .label()
            .is_some_and(|label| label.ends_with(": 1"))
    );
    assert!(
        app.find("pocket-White-Pawn")
            .label()
            .is_some_and(|label| label.ends_with(": 0"))
    );
}

/// A new game starts from a FEN in any variant, read by that variant's rules: a Crazyhouse
/// position's pocket pawn drops at once.
#[gpui_kit::test]
fn a_new_game_starts_a_variant_from_a_position(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.click("local-new");
    app.wait_for("the variant choice to stop moving", {
        use gpui_kit::test::TestQueryExt as _;
        let mut last = None;
        move |window, _| {
            let bounds = window
                .try_find_by_label("Choose a variant")
                .map(|select| select.bounds());
            let still = bounds.is_some() && bounds == last;
            last = bounds;
            still
        }
    });
    app.click_label("Choose a variant");
    // Crazyhouse is the last of the nine.
    for _ in 0..8 {
        app.press("down");
    }
    app.press("enter");
    app.fill_in_dialog(
        "Start from a FEN (optional)",
        "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3",
    );
    app.click("start-local");
    app.click("pocket-White-Pawn");
    app.click_square("e3", true);
    app.wait_for_moves(&["P@e3"]);
    assert_eq!(app.status(), "Black to move");
    // The next new game opens on this one: Crazyhouse, from that position.
    app.click("local-new");
    app.wait_for("the position to come back", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Start from a FEN (optional)")
            .and_then(|input| input.value().map(str::to_string))
            .as_deref()
            == Some("rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3")
    });
}

/// A Chess960 game goes to the analysis board too, its castling (king takes rook) included, and
/// the position is played on from there as Chess960.
#[gpui_kit::test]
fn a_chess960_game_can_be_taken_to_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    app.with_window(|window, cx| {
        let local = crate::shell::view::<LocalPage>(cx).expect("the page");
        local.update(cx, |local, cx| {
            let fen = "1rk4r/pppppppp/8/8/8/8/PPPPPPPP/1RK4R w HBhb - 0 1";
            let setup = serde_json::json!({ "variant": "chess960", "fen": fen });
            local.replace_game(setup, serde_json::Value::Null, window, cx);
        });
    });
    type_moves(&mut app, &["O-O-O", "e5"]);
    app.click("local-analyse");
    app.find("tree-c1b1");
    assert_eq!(app.find("tree-c1b1 e7e5").selected(), Some(true));
}

/// F turns the board, as its flip button does, and the arrows and Home still step the moves.
#[gpui_kit::test]
fn f_flips_the_board(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5"]);
    let orientation = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<LocalPage>(cx)
                .expect("the page")
                .read(cx)
                .orientation(cx)
        })
    };
    let before = orientation(&mut app);
    app.click(("move", 1usize));
    app.press("f");
    assert_eq!(orientation(&mut app), !before);
    app.press("home");
    assert!(!app.enabled("skip-back"));
    app.press("f");
    assert_eq!(orientation(&mut app), before);
    // The move box still takes a click and the next move.
    app.press("end");
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Nf3");
    app.press("enter");
    app.wait_for_moves(&["e4", "e5", "Nf3"]);
}

/// The mouse wheel over the board steps through the moves, as on the analysis board: up goes
/// back, down goes on.
#[gpui_kit::test]
fn the_wheel_over_the_board_steps_through_the_moves(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5"]);
    let wheel = |app: &mut Harness, by: f32| {
        app.with_window(|window, cx| {
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(by));
            window.scroll("chess-board", gpui_kit::ScrollDelta::Pixels(by), cx);
        });
        app.cx.run_until_parked();
    };
    wheel(&mut app, 60.);
    wheel(&mut app, 60.);
    assert!(!app.enabled("skip-back"));
    wheel(&mut app, -60.);
    assert!(app.enabled("skip-back"));
    assert!(app.enabled("skip-forward"));
    wheel(&mut app, -60.);
    assert!(!app.enabled("skip-forward"));
}

/// ⌘Z (Ctrl+Z) takes back as the button does once the board has the keys; in the move box it
/// undoes the typing instead.
#[gpui_kit::test]
fn undo_takes_back_outside_the_move_box(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    type_moves(&mut app, &["e4", "e5"]);
    // An empty square takes the keys from the move box.
    app.click_square("a3", true);
    app.press("secondary-z");
    app.wait_for_moves(&["e4"]);
    app.press("secondary-z");
    app.wait_for_moves(&[]);
    app.press("secondary-z");
    assert!(app.moves().is_empty());
}

/// The side to move hears the low-time warning as its clock runs low, before its flag falls.
#[gpui_kit::test]
fn a_low_clock_warns_before_it_runs_out(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::local::LocalPage;
    use crate::sound::{Played, Sound};
    let mut app = board(cx);
    app.cx.update(|cx| cx.set_global(Played::default()));
    let started = std::time::Instant::now();
    app.with_window(|window, cx| {
        let local = crate::shell::view::<LocalPage>(cx).expect("the page");
        local.update(cx, |local, cx| {
            let setup =
                serde_json::json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN });
            // Three seconds each: the warning comes under two.
            let clock = serde_json::json!({
                "white": { "minutes": 0.05, "increment": 0 },
                "black": { "minutes": 0.05, "increment": 0 },
            });
            local.replace_game(setup, clock, window, cx);
            local.play("e2e4", window, cx);
        });
    });
    app.wait_for_in_real_time("the low-time warning", |_, cx| {
        cx.global::<Played>().0.contains(&Sound::LowTime)
    });
    assert!(started.elapsed() < std::time::Duration::from_millis(2_900));
}

/// A dialog a key opened (⌘N here) stands at once, as a command repeated all day should; one a
/// click opened settles in from just above its place. The entrance runs on the test's clock, so
/// it holds still until the test moves the clock.
#[gpui_kit::test]
fn a_dialog_opened_by_a_key_appears_at_once(cx: &mut gpui_kit::TestAppContext) {
    let mut app = board(cx);
    app.cx.update(|cx| cx.set_reduce_motion(false));
    let settle = |app: &mut Harness| {
        app.cx
            .executor()
            .advance_clock(std::time::Duration::from_secs(1));
        app.cx.run_until_parked();
    };

    app.press(if cfg!(target_os = "macos") {
        "cmd-n"
    } else {
        "ctrl-n"
    });
    let opened = app.find("start-local").bounds();
    settle(&mut app);
    assert_eq!(
        app.find("start-local").bounds(),
        opened,
        "at rest when it opened"
    );
    app.click("cancel-local");
    settle(&mut app);
    app.wait_gone("start-local");

    app.click("local-new");
    let opened = app.find("start-local").bounds();
    settle(&mut app);
    let rest = app.find("start-local").bounds();
    assert!(
        rest.origin.y > opened.origin.y && rest.origin.y - opened.origin.y < gpui_kit::px(8.),
        "settles a few pixels down: {opened:?} to {rest:?}"
    );
}
