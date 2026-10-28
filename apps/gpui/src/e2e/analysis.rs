//! The analysis board and the board editor.

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

use super::Harness;

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn analysis(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    app.find("chess-board");
    app
}

fn editor(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Board editor");
    app.find("chess-board");
    wait_for_fen(&mut app, START_FEN);
    app
}

/// Type into the labelled input of a dialog once it has slid into place.
fn fill_in_dialog(app: &mut Harness, label: &str, text: &str) {
    let mut last = None;
    app.wait_for(&format!("{label:?} to stop moving"), |window, _| {
        let bounds = window.try_find_by_label(label).map(|input| input.bounds());
        let still = bounds.is_some() && bounds == last;
        last = bounds;
        still
    });
    app.click_label(label);
    app.press("cmd-a");
    app.type_text(text);
}

/// Let animations run their course: a dialog that is closing is gone after this.
fn settle(app: &mut Harness) {
    let mut frames = 0;
    app.wait_for("the window to settle", |_, _| {
        frames += 1;
        frames > 80
    });
}

/// Import a FEN or PGN through the analysis board's dialog, and let it close if it will.
fn import(app: &mut Harness, text: &str) {
    app.click("analysis-import");
    fill_in_dialog(app, "FEN or PGN", text);
    app.click("import-run");
    settle(app);
}

fn selected(app: &mut Harness, path: &str) -> Option<bool> {
    app.find(format!("tree-{path}")).selected()
}

/// Wait until the editor's FEN box (or any input with this label) holds `expected`.
fn wait_for_fen(app: &mut Harness, expected: &str) {
    app.wait_for(&format!("the FEN {expected:?}"), |window, _| {
        window
            .try_find_by_label("FEN")
            .is_some_and(|fen| fen.value() == Some(expected))
    });
}

fn clipboard(app: &mut Harness) -> Option<String> {
    app.cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn moves_played_on_the_board_fill_the_move_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    assert_eq!(selected(&mut app, "e2e4"), Some(true));
    app.drag_square("e7", "e5", true);
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));
    assert_eq!(selected(&mut app, "e2e4"), Some(false));
}

#[gpui_kit::test]
fn typed_moves_are_stepped_through_with_the_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    assert!(!app.enabled("analysis-back") && !app.enabled("analysis-forward"));
    app.click_label("Enter a chess move in SAN or UCI");
    for text in ["e4", "e7e5", "Nf3"] {
        app.type_text(text);
        app.press("enter");
    }
    assert_eq!(selected(&mut app, "e2e4 e7e5 g1f3"), Some(true));
    assert!(!app.enabled("analysis-forward"));

    app.click("analysis-back");
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));
    app.click("analysis-start");
    assert!(!app.enabled("analysis-start"));
    assert_eq!(selected(&mut app, "e2e4"), Some(false));
    app.click("analysis-forward");
    assert_eq!(selected(&mut app, "e2e4"), Some(true));
    app.click("analysis-end");
    assert_eq!(selected(&mut app, "e2e4 e7e5 g1f3"), Some(true));
}

#[gpui_kit::test]
fn the_arrow_keys_step_through_the_moves(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 2. Nf3 *");
    app.click("tree-e2e4 e7e5 g1f3");
    app.press("left");
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));
    app.press("left");
    app.press("left");
    assert!(!app.enabled("analysis-back"));
    app.press("right");
    assert_eq!(selected(&mut app, "e2e4"), Some(true));
    // Down goes to the end of the line, up to the start.
    app.press("down");
    assert_eq!(selected(&mut app, "e2e4 e7e5 g1f3"), Some(true));
    app.press("up");
    assert!(!app.enabled("analysis-back"));
}

#[gpui_kit::test]
fn variations_are_kept_and_a_move_into_one_starts_another(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 *");
    for path in ["e2e4 e7e5 g1f3 b8c6", "e2e4 c7c5", "e2e4 c7c5 g1f3"] {
        app.find(format!("tree-{path}"));
    }
    app.click("tree-e2e4 c7c5");
    assert_eq!(selected(&mut app, "e2e4 c7c5"), Some(true));
    app.click("analysis-back");
    assert_eq!(selected(&mut app, "e2e4"), Some(true));

    // A third reply to 1. e4 becomes another variation; the others stay.
    app.drag_square("d7", "d5", true);
    assert_eq!(selected(&mut app, "e2e4 d7d5"), Some(true));
    app.find("tree-e2e4 e7e5 g1f3 b8c6");
    app.find("tree-e2e4 c7c5 g1f3");
}

#[gpui_kit::test]
fn an_import_that_cannot_be_read_is_refused(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    // An illegal move, and a side with seventeen pieces.
    import(&mut app, "1. e4 e5 2. Ke5 *");
    assert!(app.shows("import-run"), "the dialog stays for a correction");
    fill_in_dialog(
        &mut app,
        "FEN or PGN",
        "rnbqkbnr/pppppppp/8/8/3q4/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    );
    app.click("import-run");
    settle(&mut app);
    assert!(app.shows("import-run"));
    assert!(!app.shows("tree-e2e4"));

    // A valid position closes it, and play goes on from there: only this king can reach d2.
    fill_in_dialog(&mut app, "FEN or PGN", "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
    app.click("import-run");
    settle(&mut app);
    assert!(!app.shows("import-run"));
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Kd2");
    app.press("enter");
    app.find("tree-e1d2");
}

#[gpui_kit::test]
fn a_comment_and_an_annotation_stay_with_the_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    assert!(!app.shows("glyph-2"), "the start has no move to annotate");
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.click("add-comment");
    app.type_text("Keep this annotation.");
    app.click("glyph-2");
    assert_eq!(
        app.find("comment-e2e4").label(),
        Some("Keep this annotation.")
    );
    assert_eq!(app.find("glyph-2").checked(), Some(true));

    // The same glyph again takes it off; another replaces it.
    app.click("glyph-2");
    assert_eq!(app.find("glyph-2").checked(), Some(false));
    app.click("glyph-1");
    app.click("glyph-4");
    assert_eq!(app.find("glyph-1").checked(), Some(false));
    assert_eq!(app.find("glyph-4").checked(), Some(true));
}

#[gpui_kit::test]
fn the_analysis_with_its_comments_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 2. Nf3 *");
    app.click("tree-e2e4 e7e5");
    app.click("add-comment");
    app.type_text("Symmetrical.");
    app.find("comment-e2e4 e7e5");

    let mut app = app.restart();
    app.open("Analysis board");
    app.find("tree-e2e4 e7e5 g1f3");
    assert_eq!(app.find("comment-e2e4 e7e5").label(), Some("Symmetrical."));
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));
}

#[gpui_kit::test]
fn moves_are_deleted_from_here_and_a_new_analysis_starts_empty(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 2. Nf3 *");
    app.click("tree-e2e4 e7e5");

    app.click("analysis-more");
    app.click_label("Delete from here");
    app.click("cancel");
    app.find("tree-e2e4 e7e5 g1f3");

    app.click("analysis-more");
    app.click_label("Delete from here");
    app.click("ok");
    app.find("tree-e2e4");
    app.wait_for("the deleted moves to go", |window, _| {
        window.try_find("tree-e2e4 e7e5").is_none()
    });
    assert!(!app.shows("tree-e2e4 e7e5 g1f3"));

    app.click("analysis-more");
    app.click_label("New analysis");
    app.wait_for("the board to be empty", |window, _| {
        window.try_find("tree-e2e4").is_none()
    });
}

#[gpui_kit::test]
fn a_variation_can_be_made_the_main_line(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 (1... c5) *");
    app.click("tree-e2e4 c7c5");
    app.click("analysis-more");
    app.click_label("Make main line");
    // The PGN now has c5 as the game's move and e5 as the variation.
    app.click("analysis-more");
    app.click_label("Copy PGN");
    let pgn = clipboard(&mut app).expect("a copied PGN");
    assert!(pgn.contains("1. e4 c5 ( 1... e5 )"), "{pgn}");
}

#[gpui_kit::test]
fn the_tool_under_the_engine_is_chosen_and_remembered(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    assert_eq!(app.find("Moves").selected(), Some(true));
    app.click("Review");
    assert_eq!(app.find("Review").selected(), Some(true));
    assert_eq!(app.find("Moves").selected(), Some(false));
    let mut app = app.restart();
    app.open("Analysis board");
    assert_eq!(app.find("Review").selected(), Some(true));
}

#[gpui_kit::test]
fn the_engine_can_be_switched_off_and_on(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    assert_eq!(app.find("engine-on").checked(), Some(true));
    app.click("engine-on");
    assert_eq!(app.find("engine-on").checked(), Some(false));
    app.click("engine-on");
    assert_eq!(app.find("engine-on").checked(), Some(true));
}

#[gpui_kit::test]
fn the_board_can_be_flipped_and_still_takes_moves(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    app.click("analysis-flip");
    app.click_square("e2", false);
    app.click_square("e4", false);
    assert_eq!(selected(&mut app, "e2e4"), Some(true));
}

#[gpui_kit::test]
fn a_position_can_be_taken_from_the_analysis_board_to_the_editor(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = analysis(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.click("analysis-more");
    app.click_label("Edit this position");
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
    );
    assert_eq!(app.find(("editor-turn", 1usize)).selected(), Some(true));
}

#[gpui_kit::test]
fn the_editor_starts_from_the_standard_position(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    assert_eq!(app.status(), "Ready to analyse");
    assert!(app.enabled("analyse-position"));
    assert_eq!(app.find(("editor-turn", 0usize)).selected(), Some(true));
    for key in ["K", "Q", "k", "q"] {
        assert_eq!(app.find(format!("castling-{key}")).checked(), Some(true));
    }
}

#[gpui_kit::test]
fn an_empty_board_is_no_position_to_analyse(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click("clear-board");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
    assert_eq!(app.status(), "Not a legal position yet");
    for action in ["analyse-position", "play-stockfish", "play-friend"] {
        assert!(!app.enabled(action), "{action} needs a legal position");
    }
    // Castling needs the king and rook at home.
    assert!(!app.enabled("castling-K"));

    app.click("start-position");
    wait_for_fen(&mut app, START_FEN);
    assert_eq!(app.status(), "Ready to analyse");
}

#[gpui_kit::test]
fn a_position_set_up_with_pieces_is_analysed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click("clear-board");
    for (piece, squares) in [
        ("spare-White-King", &["g1"][..]),
        ("spare-White-Rook", &["a1"]),
        ("spare-Black-King", &["g8"]),
        ("spare-Black-Pawn", &["f7", "g7", "h7"]),
    ] {
        app.click(piece);
        for square in squares {
            app.click_square(square, true);
        }
    }
    wait_for_fen(&mut app, "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1");
    assert_eq!(app.status(), "Ready to analyse");

    app.click("analyse-position");
    app.find("analysis-more");
    app.click("analysis-more");
    app.click_label("Copy FEN");
    assert_eq!(
        clipboard(&mut app).as_deref(),
        Some("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1")
    );
}

#[gpui_kit::test]
fn the_bin_and_a_dragged_piece_remove_and_move_pieces(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click("remove-pieces-White");
    app.click_square("a2", true);
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/8/8/1PPPPPPP/RNBQKBNR w KQkq - 0 1",
    );
    app.click("move-pieces-White");
    app.drag_square("e2", "e4", true);
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/4P3/8/1PPP1PPP/RNBQKBNR w KQkq - 0 1",
    );
}

#[gpui_kit::test]
fn the_side_to_move_and_castling_rights_show_in_the_fen(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click(("editor-turn", 1usize));
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1",
    );
    assert_eq!(app.status(), "Ready to analyse");
    app.click("castling-K");
    app.click("castling-q");
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b Qk - 0 1",
    );
}

#[gpui_kit::test]
fn a_typed_fen_sets_the_board_and_one_that_is_not_legal_is_not_analysed(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = editor(cx);
    app.click_label("FEN");
    app.press("cmd-a");
    app.type_text("bad-fen");
    app.press("enter");
    // Nothing changed behind the refused text.
    assert_eq!(app.status(), "Ready to analyse");

    app.press("cmd-a");
    app.type_text("4k3/8/8/8/8/8/4P3/4K3 b - - 0 1");
    app.press("enter");
    wait_for_fen(&mut app, "4k3/8/8/8/8/8/4P3/4K3 b - - 0 1");
    assert_eq!(app.find(("editor-turn", 1usize)).selected(), Some(true));

    // Seventeen pieces to a side is more than Stockfish will take.
    app.press("cmd-a");
    app.type_text("rnbqkbnr/pppppppp/8/8/3q4/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    app.press("enter");
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/3q4/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    );
    assert_eq!(app.status(), "Not a legal position yet");
    assert!(!app.enabled("analyse-position"));
}

#[gpui_kit::test]
fn the_position_can_be_played_over_the_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click_label("FEN");
    app.press("cmd-a");
    app.type_text("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
    app.press("enter");
    wait_for_fen(&mut app, "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
    app.click("play-friend");
    app.find("local-new");
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Kd2");
    app.press("enter");
    app.wait_for_moves(&["Kd2"]);
}
