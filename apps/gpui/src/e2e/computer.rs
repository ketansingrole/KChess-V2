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
    wait_playable(&mut app);
    app
}

/// Until the page takes moves: the saved game is restored and the engine is ready, which a new
/// game (another variant's engine) waits for again. The page itself is asked, not the move field
/// as last drawn, which can still show the game before.
fn wait_playable(app: &mut Harness) {
    app.wait_for("the game to take moves", |_, cx| {
        crate::shell::view::<crate::pages::computer::ComputerPage>(cx)
            .is_some_and(|page| page.read(cx).takes_moves())
    });
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

/// The board from the keyboard: Tab gives it the keys, Enter picks up the pawn under its cursor,
/// the arrows walk the cursor to e4 and Enter plays the pawn there. A screen reader hears each
/// square's name and whether the pawn can go there.
#[gpui_kit::test]
fn a_move_is_played_from_the_keyboard(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.tab_to_board();
    assert_eq!(app.square_name("e2"), "e2, white pawn");
    assert_eq!(
        app.find(("square", usize::from(shakmaty::Square::E2)))
            .selected(),
        Some(false)
    );
    app.press("enter");
    assert_eq!(
        app.find(("square", usize::from(shakmaty::Square::E2)))
            .selected(),
        Some(true)
    );
    assert_eq!(app.square_name("e4"), "e4, empty, legal move");
    app.press("up");
    app.press("up");
    app.press("enter");
    app.wait_for_moves(&["e4"]);
    assert_eq!(app.square_name("e4"), "e4, white pawn");
    // The board keeps the keys: the arrows still move its cursor, not the game.
    app.press("left");
    assert!(app.enabled("skip-back"));
    // A click gives them back to the page.
    app.click(("move", 0usize));
    app.press("left");
    assert!(!app.enabled("skip-back"));
}

/// After a typed move, a click on the moves gives the arrow keys back to the board: they had
/// stayed with the move box, so ← moved its cursor instead of the game.
#[gpui_kit::test]
fn the_arrows_step_the_moves_after_a_typed_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Nf3");
    app.press("enter");
    app.wait_for_moves(&["Nf3"]);
    app.click(("move", 0usize));
    app.press("left");
    assert!(!app.enabled("skip-back"));
    app.press("end");
    assert!(app.enabled("skip-back"));
}

/// The Export menu copies the game's PGN and the position shown as a FEN.
#[gpui_kit::test]
fn the_export_menu_copies_the_pgn_and_the_fen(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Nf3");
    app.press("enter");
    app.wait_for_moves(&["Nf3"]);
    let copied = |app: &mut Harness, item: &str| {
        app.click("computer-export");
        app.wait_for("the menu", |window, _| {
            use gpui_kit::test::TestQueryExt as _;
            window.try_find_by_label(item).is_some()
        });
        app.click_label(item);
        app.cx.read_from_clipboard().and_then(|item| item.text())
    };
    let pgn = copied(&mut app, "Copy PGN").expect("a copied PGN");
    assert!(pgn.contains("1. Nf3"), "{pgn}");
    assert_eq!(
        copied(&mut app, "Copy FEN").as_deref(),
        Some("rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 1 1")
    );
}

/// Copy outside a text field copies the game's PGN here too, as on the other boards.
#[gpui_kit::test]
fn copy_copies_the_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Nf3");
    app.press("enter");
    app.wait_for_moves(&["Nf3"]);
    app.click("chess-board");
    app.cx
        .write_to_clipboard(gpui_kit::ClipboardItem::new_string("kept".into()));
    app.press("secondary-c");
    app.tick();
    let pgn = app
        .cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(pgn.contains("1. Nf3"), "{pgn}");
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

/// Quick Pick shows ⌘C and ⌘N beside the commands they run here.
#[gpui_kit::test]
fn quick_pick_shows_the_copy_and_new_keys(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    let keys = app.cx.update(|cx| crate::palette::page_command_keys(cx));
    let key = |id: &str| {
        keys.iter()
            .find(|(command, _)| command == id)
            .map(|(_, keys)| keys.as_str())
    };
    let (copy, new) = if cfg!(target_os = "macos") {
        ("⌘ C", "⌘ N")
    } else {
        ("Ctrl C", "Ctrl N")
    };
    assert_eq!(key("computer:copy-pgn"), Some(copy), "{keys:?}");
    assert_eq!(key("computer:new-game"), Some(new), "{keys:?}");
}

/// ⌘N asks the same as New game does.
#[gpui_kit::test]
fn the_new_key_asks_first_then_clears_the_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.press("secondary-n");
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
        crate::pages::computer::play_from(
            crate::game::setup_from_fen("1rk4r/pppppppp/8/8/8/8/PPPPPPPP/1RK4R w HBhb - 0 1"),
            window,
            cx,
        )
    });
    app.wait_for_moves(&[]);
    // Under a loaded full run the move was once typed before the new game took moves, and lost.
    wait_playable(&mut app);
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("O-O-O");
    app.press("enter");
    app.wait_for_moves(&["O-O-O"]);
    let variant = app.cx.update(|cx| {
        crate::shell::view::<crate::pages::computer::ComputerPage>(cx)
            .map(|page| page.read(cx).setup_variant())
    });
    assert_eq!(variant.as_deref(), Some("chess960"));
}

/// Until the saved game is restored the board takes no move: on a slow profile (Windows CI) the
/// restore once landed between a player's two clicks and dropped the move.
#[gpui_kit::test]
fn no_move_is_taken_before_the_saved_game_is_restored(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.cx.update(|cx| {
        let page =
            crate::shell::view::<crate::pages::computer::ComputerPage>(cx).expect("the page");
        page.update(cx, |page, _| {
            crate::game::session::Resumes::saver(page).forget()
        });
    });
    app.click_square("e2", true);
    app.click_square("e4", true);
    assert_eq!(app.moves(), Vec::<String>::new());
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
    app.wait_for_page(crate::pages::Page::Analysis);
}

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
    app.click_label("Start from a FEN (optional)");
    app.type_text("not-a-fen");
    assert!(!app.enabled("start-game"));
    app.press("secondary-a");
    app.type_text(fen);
    app.click("start-game");
    app.wait_gone("start-game");
    let setup = app.cx.update(|cx| {
        crate::shell::view::<crate::pages::computer::ComputerPage>(cx).map(|page| {
            let page = page.read(cx);
            (page.setup_variant(), page.setup_fen())
        })
    });
    assert_eq!(setup, Some(("crazyhouse".to_string(), fen.to_string())));

    // The dialog opens with that position; another variant, which refuses it, clears it.
    app.click("game-options");
    let typed = |app: &mut Harness| {
        app.with_window(|window, _| {
            window
                .try_find_by_label("Start from a FEN (optional)")
                .and_then(|input| input.value().map(str::to_string))
        })
    };
    let variant = |window: &mut gpui_kit::Window| {
        window
            .try_find_by_label("Choose a variant")
            .and_then(|select| select.value().map(str::to_string))
    };
    app.wait_for("the dialog", |window, _| {
        variant(window).as_deref() == Some("Crazyhouse")
    });
    assert_eq!(typed(&mut app).as_deref(), Some(fen));
    app.click_label("Choose a variant");
    // Horde is two above Crazyhouse.
    app.press("up");
    app.press("up");
    app.press("enter");
    app.wait_for("the Horde start", |window, _| {
        variant(window).as_deref() == Some("Horde")
    });
    assert!(typed(&mut app).is_none_or(|value| value.is_empty()));
    assert!(app.enabled("start-game"));
/// Game options start a game in another variant, which the engine plays by its rules: the
/// variant's start, and the computer's move from it.
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
            crate::shell::view::<crate::pages::computer::ComputerPage>(cx)
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
        let page =
            crate::shell::view::<crate::pages::computer::ComputerPage>(cx).expect("the page");
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

/// ⌘Z (Ctrl+Z) takes back as the button does, once the page has the keys.
#[gpui_kit::test]
fn undo_takes_back(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.press("secondary-z");
    app.wait_for_moves(&[]);
}

/// The player's clock warns as it runs low, before it runs out, as Lichess's does. (The stand-in
/// engine answers e4 alone: the player, Black to move, plays first so that answer is legal.)
#[gpui_kit::test]
fn a_low_clock_warns_before_it_runs_out(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::computer::ComputerPage;
    use crate::sound::{Played, Sound};
    let mut app = computer(cx);
    app.cx.update(|cx| cx.set_global(Played::default()));
    app.with_window(|window, cx| {
        let page = crate::shell::view::<ComputerPage>(cx).expect("the page");
        let setup = serde_json::json!({
            "variant": "standard",
            "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1",
        });
        // Three seconds: the warning comes under two, once both sides have moved.
        page.update(cx, |page, cx| {
            page.play_with_clock(setup, shakmaty::Color::Black, 0.05, window, cx)
        });
    });
    wait_playable(&mut app);
    app.click_square("e7", false);
    app.click_square("e5", false);
    app.wait_for_moves(&["e5", "e4"]);
    let started = std::time::Instant::now();
    app.wait_for_in_real_time("the low-time warning", |_, cx| {
        cx.global::<Played>().0.contains(&Sound::LowTime)
    });
    assert!(started.elapsed() < std::time::Duration::from_millis(2_900));
}

/// F turns the board on Play with computer, as on every other game page: the page answers the
/// app's Flip board key.
#[gpui_kit::test]
fn f_flips_the_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = computer(cx);
    let orientation = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::computer::ComputerPage>(cx)
                .expect("the page")
                .read(cx)
                .shown_orientation()
        })
    };
    let before = orientation(&mut app);
    app.click("chess-board");
    app.press("f");
    assert_eq!(orientation(&mut app), !before);
    app.press("f");
    assert_eq!(orientation(&mut app), before);
}
