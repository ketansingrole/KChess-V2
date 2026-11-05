//! Play with computer, against the scripted engine (it always answers `e2e4`, so the computer
//! moves only when it has White).

use super::Harness;

/// The page, once its game takes moves: the board shows at once, but moves wait for the engine to
/// be ready. Clicks before that were ignored, and Windows CI, where each test binary's engine
/// starts cold, failed `a_clicked_move_is_played` with no move played now and then.
fn computer(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.find("chess-board");
    app.wait_for("the game to take moves", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Enter a chess move in SAN or UCI")
            .is_some_and(|entry| entry.disabled() != Some(true))
    });
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
    app.open("Play with computer");
    app.wait_for_moves(&["e4", "e5"]);
}

/// A position whose king and rooks castle from unusual squares is played on as Chess960: its
/// castling is king takes rook, and the game is analysable.
#[gpui_kit::test]
fn a_chess960_position_is_played_from_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.with_window(|window, cx| {
        crate::shell::play_computer_from(
            crate::game::setup_from_fen("1rk4r/pppppppp/8/8/8/8/PPPPPPPP/1RK4R w HBhb - 0 1"),
            window,
            cx,
        )
    });
    app.wait_for_moves(&[]);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("O-O-O");
    app.press("enter");
    app.wait_for_moves(&["O-O-O"]);
    let variant = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::computer::ComputerPage>(
            crate::pages::Page::Computer,
            cx,
        )
        .map(|page| page.read(cx).setup_variant())
    });
    assert_eq!(variant.as_deref(), Some("chess960"));
}

/// Once the game is over, Quick Pick reviews it on the analysis board, as the banner's button does.
#[gpui_kit::test]
fn a_finished_game_is_reviewed_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.click("resign");
    app.click("ok");
    app.find("review-game");
    super::palette::wait_until_closed(&mut app);
    app.press("secondary-k");
    app.type_text(">Review game");
    super::palette::wait_for_first(&mut app, "Review game");
    app.press("enter");
    app.wait_for("the analysis board", |_, cx| {
        crate::shell::current_page(cx) == Some(crate::pages::Page::Analysis)
    });
}

/// Game options start a game in another variant, which the engine plays by its rules: the
/// variant's start, and the computer's move from it.
/// A variant game starts from a FEN read by that variant's rules (a Crazyhouse position with
/// its pockets), and a FEN it refuses keeps the game from starting.
#[gpui_kit::test]
fn game_options_start_a_variant_from_a_position(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestQueryExt as _;
    let mut app = computer(cx);
    app.click("game-options");
    app.click(("start", 2usize));
    let mut last = None;
    app.wait_for("the variant choice to stop moving", |window, _| {
        let bounds = window
            .try_find_by_label("Variant")
            .map(|select| select.bounds());
        let still = bounds.is_some() && bounds == last;
        last = bounds;
        still
    });
    app.click_label("Choose a variant");
    // Crazyhouse is the last of the seven Fairy-Stockfish plays.
    for _ in 0..6 {
        app.press("down");
    }
    app.press("enter");
    let fen = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
    app.click_label("Crazyhouse start");
    app.type_text("not-a-fen");
    assert!(!app.enabled("start-game"));
    app.press("secondary-a");
    app.type_text(fen);
    app.click("start-game");
    app.wait_gone("start-game");
    let setup = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::computer::ComputerPage>(
            crate::pages::Page::Computer,
            cx,
        )
        .map(|page| {
            let page = page.read(cx);
            (page.setup_variant(), page.setup_fen())
        })
    });
    assert_eq!(setup, Some(("crazyhouse".to_string(), fen.to_string())));
}

#[gpui_kit::test]
fn game_options_start_a_variant_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click("game-options");
    app.click(("start", 2usize));
    app.click("start-game");
    app.wait_gone("start-game");
    let variant = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view_of::<crate::pages::computer::ComputerPage>(
                crate::pages::Page::Computer,
                cx,
            )
            .map(|page| page.read(cx).setup_variant())
        })
    };
    // The first variant listed after standard chess and Chess960.
    assert_eq!(variant(&mut app).as_deref(), Some("kingOfTheHill"));
    // The scripted engine answers e2e4, which King of the Hill's rules allow from its start.
    app.click("flip");
    app.wait_for_moves(&["e4"]);

    // A Crazyhouse start (its FEN has pockets, which standard rules refuse) is searched in
    // Crazyhouse: the computer, White, moves.
    app.with_window(|window, cx| {
        let page = crate::shell::view_of::<crate::pages::computer::ComputerPage>(
            crate::pages::Page::Computer,
            cx,
        )
        .expect("the page");
        page.update(cx, |page, cx| {
            page.start_from(
                serde_json::json!({
                    "variant": "crazyhouse",
                    "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1",
                }),
                shakmaty::Color::Black,
                window,
                cx,
            )
        });
    });
    // Starting over asks first.
    app.click("ok");
    app.wait_for_moves(&["e4"]);
    assert_eq!(variant(&mut app).as_deref(), Some("crazyhouse"));
    // The unfinished Crazyhouse game is kept (it once could not be saved).
    let mut app = app.restart();
    app.open("Play with computer");
    app.wait_for_moves(&["e4"]);
    assert_eq!(variant(&mut app).as_deref(), Some("crazyhouse"));
    // The dialog opens on the variant game being played.
    app.click("game-options");
    assert_eq!(app.find(("start", 2usize)).selected(), Some(true));
}
