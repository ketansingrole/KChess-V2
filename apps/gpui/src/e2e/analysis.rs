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
    app.press("secondary-a");
    app.type_text(text);
}

/// Let animations run their course: a dialog that is closing is gone after this.
pub(super) fn settle(app: &mut Harness) {
    let mut frames = 0;
    app.wait_for("the window to settle", |_, _| {
        frames += 1;
        frames > 80
    });
}

/// Import a FEN or PGN through the analysis board's dialog, and let it close if it will.
pub(super) fn import(app: &mut Harness, text: &str) {
    app.click("analysis-import");
    fill_in_dialog(app, "FEN, PGN or Lichess link", text);
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

/// The move list names the opening the moves up to the shown one reached, as the game pages do,
/// and not once Settings hide opening names.
#[gpui_kit::test]
fn the_move_list_names_the_opening(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.drag_square("e7", "e5", true);
    app.drag_square("g1", "f3", true);
    assert_eq!(
        app.find("analysis-opening").label(),
        Some("C40 King's Knight Opening")
    );
    // Back a move: the opening as it stood then.
    app.press("left");
    assert_eq!(
        app.find("analysis-opening").label(),
        Some("C20 King's Pawn Game")
    );
    app.cx
        .update(|cx| crate::settings::change(cx, |s| s.frontend.show_opening_name = false));
    app.wait_for("the name to go", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.try_find("analysis-opening").is_none()
    });
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
    // End and Home do the same.
    app.press("end");
    assert_eq!(selected(&mut app, "e2e4 e7e5 g1f3"), Some(true));
    app.press("home");
    assert!(!app.enabled("analysis-back"));
}

#[gpui_kit::test]
fn shift_and_the_arrows_go_into_and_out_of_a_variation(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 (1... c5 2. Nf3 d6) 2. Nf3 *");
    app.click("tree-e2e4");
    app.press("shift-right");
    assert_eq!(selected(&mut app, "e2e4 c7c5"), Some(true));
    app.press("right");
    app.press("right");
    assert_eq!(selected(&mut app, "e2e4 c7c5 g1f3 d7d6"), Some(true));
    app.press("shift-left");
    assert_eq!(selected(&mut app, "e2e4"), Some(true));
    // On the main line there is nothing to leave.
    app.press("right");
    app.press("shift-left");
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));
}

#[gpui_kit::test]
fn e_shows_the_explorer_and_the_moves_again(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 *");
    app.click("tree-e2e4");
    app.press("e");
    assert_eq!(app.find("Explorer").selected(), Some(true));
    app.press("e");
    assert_eq!(app.find("Moves").selected(), Some(true));
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
        "FEN, PGN or Lichess link",
        "rnbqkbnr/pppppppp/8/8/3q4/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    );
    app.click("import-run");
    settle(&mut app);
    assert!(app.shows("import-run"));
    assert!(!app.shows("tree-e2e4"));

    // A valid position closes it, and play goes on from there: only this king can reach d2.
    fill_in_dialog(
        &mut app,
        "FEN, PGN or Lichess link",
        "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1",
    );
    app.click("import-run");
    // The dialog closes on the core's answer, which a loaded runner gives after any fixed wait.
    app.wait_gone("import-run");
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

/// ⌘Z and ⇧⌘Z (Ctrl+Z, Ctrl+Y) undo and redo the tree's edits, a comment's typing as one step,
/// and the board's ⋯ menu offers them too. A new document starts with nothing to undo.
#[gpui_kit::test]
fn edits_are_undone_and_redone(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 2. Nf3 (2. Bc4) *");
    let undo = |app: &mut Harness| app.press("secondary-z");
    let redo = |app: &mut Harness| app.press("shift-secondary-z");
    // The import is where the history starts.
    app.click("analysis-more");
    app.click_label("Undo");
    app.find("tree-e2e4 e7e5 g1f3");

    // A comment, typed letter by letter, is one step.
    app.click("tree-e2e4 e7e5");
    app.click("add-comment");
    app.type_text("Symmetrical.");
    app.find("comment-e2e4 e7e5");
    app.click("chess-board");
    undo(&mut app);
    app.wait_gone("comment-e2e4 e7e5");
    redo(&mut app);
    app.find("comment-e2e4 e7e5");

    // A promotion, then a deletion, undone in turn and redone.
    app.click("tree-e2e4 e7e5 f1c4");
    app.click("analysis-more");
    app.click_label("Make main line");
    app.click("analysis-more");
    app.click_label("Delete from here");
    app.wait_gone("tree-e2e4 e7e5 f1c4");
    undo(&mut app);
    app.find("tree-e2e4 e7e5 f1c4");
    assert_eq!(selected(&mut app, "e2e4 e7e5 f1c4"), Some(true));
    assert!(app.with_window(|_, cx| {
        crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
        .is_some_and(|page| page.read(cx).pgn().contains("2. Bc4 ( 2. Nf3 )"))
    }));
    undo(&mut app);
    assert!(app.with_window(|_, cx| {
        crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
        .is_some_and(|page| page.read(cx).pgn().contains("2. Nf3 ( 2. Bc4 )"))
    }));
    redo(&mut app);
    redo(&mut app);
    app.wait_gone("tree-e2e4 e7e5 f1c4");

    // A move played after an undo drops what could be redone.
    undo(&mut app);
    app.find("tree-e2e4 e7e5 f1c4");
    app.click_square("g8", true);
    app.click_square("f6", true);
    app.find("tree-e2e4 e7e5 f1c4 g8f6");
    redo(&mut app);
    app.find("tree-e2e4 e7e5 f1c4");
    undo(&mut app);
    app.wait_gone("tree-e2e4 e7e5 f1c4 g8f6");
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

    // Deleting asks nothing: its notice's Undo brings the moves back.
    app.click("analysis-more");
    app.click_label("Delete from here");
    app.find("tree-e2e4");
    app.wait_for("the deleted moves to go", |window, _| {
        window.try_find("tree-e2e4 e7e5").is_none()
    });
    assert!(!app.shows("tree-e2e4 e7e5 g1f3"));
    app.click("undo-delete-moves");
    app.find("tree-e2e4 e7e5 g1f3");
    assert_eq!(selected(&mut app, "e2e4 e7e5"), Some(true));

    app.click("analysis-more");
    app.click_label("Delete from here");
    app.wait_for("the deleted moves to go", |window, _| {
        window.try_find("tree-e2e4 e7e5").is_none()
    });

    app.click("analysis-more");
    app.click_label("New analysis");
    app.wait_for("the board to be empty", |window, _| {
        window.try_find("tree-e2e4").is_none()
    });
}

/// ⌘N starts a new analysis board, asking first when its moves would be lost.
#[gpui_kit::test]
fn new_starts_an_empty_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 *");
    app.press("secondary-n");
    app.click_label("Replace");
    app.wait_for("the board to be empty", |window, _| {
        window.try_find("tree-e2e4").is_none()
    });
}

/// Backspace (or Delete) on the board deletes from the move shown, as the move's menu does, and
/// ⌘Z brings the moves back.
#[gpui_kit::test]
fn backspace_deletes_from_the_move_shown(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 2. Nf3 *");
    app.click("tree-e2e4 e7e5");
    app.press("backspace");
    app.wait_for("the deleted moves to go", |window, _| {
        window.try_find("tree-e2e4 e7e5").is_none()
    });
    app.find("tree-e2e4");
    app.press("secondary-z");
    app.find("tree-e2e4 e7e5 g1f3");
    app.click("tree-e2e4 e7e5 g1f3");
    app.press("delete");
    app.wait_for("the last move to go", |window, _| {
        window.try_find("tree-e2e4 e7e5 g1f3").is_none()
    });
    app.find("tree-e2e4 e7e5");
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

/// ⌘Z takes back the editor's changes one by one, and ⇧⌘Z redoes them.
#[gpui_kit::test]
fn the_editor_undoes_and_redoes_its_changes(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click("clear-board");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
    app.click(("editor-turn", 1usize));
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 b - - 0 1");
    app.press("secondary-z");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
    app.press("secondary-z");
    wait_for_fen(&mut app, START_FEN);
    // Nothing more to undo: the start stays.
    app.press("secondary-z");
    wait_for_fen(&mut app, START_FEN);
    app.press("shift-secondary-z");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
    // A new change drops what was undone.
    app.click("start-position");
    wait_for_fen(&mut app, START_FEN);
    app.press("shift-secondary-z");
    wait_for_fen(&mut app, START_FEN);
    app.press("secondary-z");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
}

/// F turns the editor's board, as its button does.
#[gpui_kit::test]
fn f_flips_the_editors_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    let orientation = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view_of::<crate::pages::editor::EditorPage>(
                crate::pages::Page::Editor,
                cx,
            )
            .map(|page| page.read(cx).orientation())
        })
    };
    let before = orientation(&mut app).expect("the page");
    app.click(("editor-turn", 0usize));
    app.press("f");
    assert_eq!(orientation(&mut app), Some(!before));
}

/// The editor sets up positions in the other variants: Horde's start, read by Horde's rules and
/// analysed as a Horde game; a pasted Crazyhouse FEN brings its variant and keeps its pockets.
#[gpui_kit::test]
fn the_editor_sets_up_a_variant_position(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.click_label("Choose a variant");
    // Horde is sixth after standard chess.
    for _ in 0..6 {
        app.press("down");
    }
    app.press("enter");
    app.click("start-position");
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/PPPPPPPP/PPPPPPPP/PPPPPPPP w kq - 0 1",
    );
    // White has no king, which only Horde allows.
    assert_eq!(app.status(), "Ready to analyse");
    app.click("analyse-position");
    app.wait_for("the Horde game on the analysis board", |_, cx| {
        crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
        .is_some_and(|page| {
            let page = page.read(cx);
            page.header("Variant") == "Horde"
                && page.shown_fen().starts_with("rnbqkbnr/pppppppp/8/1PP2PP1")
        })
    });

    app.open("Board editor");
    let crazyhouse = "rnb1kbnr/ppp1pppp/8/q7/8/2N5/PPPP1PPP/R1BQKBNR[Pp] w KQkq - 2 4";
    app.click_label("FEN");
    app.press("secondary-a");
    app.type_text(crazyhouse);
    app.press("enter");
    // The editor starts the move counters again, as for any position.
    wait_for_fen(
        &mut app,
        "rnb1kbnr/ppp1pppp/8/q7/8/2N5/PPPP1PPP/R1BQKBNR[Pp] w KQkq - 0 1",
    );
    assert_eq!(app.status(), "Ready to analyse");
    let variant = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::editor::EditorPage>(crate::pages::Page::Editor, cx)
            .map(|page| page.read(cx).game_setup())
    });
    assert_eq!(variant.expect("the editor")["variant"], "crazyhouse");
}

/// The spare pieces are buttons a screen reader names, chosen as the tools beside them are.
#[gpui_kit::test]
fn a_spare_piece_is_a_named_button_that_is_chosen(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    let queen = app.find("spare-White-Queen");
    assert_eq!(queen.role(), Some(gpui_kit::Role::Button));
    assert_eq!(queen.label(), Some("White queen"));
    assert_eq!(queen.selected(), Some(false));
    app.click("spare-White-Queen");
    app.wait_for("the queen to be chosen", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("spare-White-Queen")
            .is_some_and(|queen| queen.selected() == Some(true))
    });
    assert_eq!(app.find("spare-Black-Queen").selected(), Some(false));
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

/// A pasted Chess960 position keeps its castling; a king moved off e1 in a standard one loses
/// White's, rather than castling Chess960-style from d1.
#[gpui_kit::test]
fn chess960_castling_is_kept_and_a_moved_king_drops_its_own(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    app.drag_square("e1", "d1", true);
    wait_for_fen(
        &mut app,
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBK1BNR w kq - 0 1",
    );
    // Castling from d1 is allowed in Chess960, and offered, but not ticked for you.
    assert!(app.enabled("castling-Q"));
    assert_eq!(app.find("castling-Q").checked(), Some(false));

    app.click_label("FEN");
    app.press("secondary-a");
    app.type_text("bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1");
    app.press("enter");
    wait_for_fen(
        &mut app,
        "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w KQkq - 0 1",
    );
    for key in ["K", "Q", "k", "q"] {
        assert_eq!(
            app.find(format!("castling-{key}")).checked(),
            Some(true),
            "{key}"
        );
    }
    assert_eq!(app.status(), "Ready to analyse");
}

#[gpui_kit::test]
fn a_typed_fen_sets_the_board_and_one_that_is_not_legal_is_not_analysed(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = editor(cx);
    app.click_label("FEN");
    app.press("secondary-a");
    app.type_text("bad-fen");
    app.press("enter");
    // Nothing changed behind the refused text.
    assert_eq!(app.status(), "Ready to analyse");

    app.press("secondary-a");
    app.type_text("4k3/8/8/8/8/8/4P3/4K3 b - - 0 1");
    app.press("enter");
    wait_for_fen(&mut app, "4k3/8/8/8/8/8/4P3/4K3 b - - 0 1");
    assert_eq!(app.find(("editor-turn", 1usize)).selected(), Some(true));

    // Seventeen pieces to a side is more than Stockfish will take.
    app.press("secondary-a");
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
    app.press("secondary-a");
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

/// Export `label` from the analysis board's Export menu into a file; its bytes.
fn export(app: &mut Harness, label: &str, file: &str) -> Vec<u8> {
    app.click("analysis-export");
    app.click_label(label);
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_new_path() {
        assert!(started.elapsed() < super::TIMEOUT, "no save dialog opened");
        app.tick();
    }
    let folder = tempfile::tempdir().expect("a folder to save in");
    let path = folder.path().join(file);
    let chosen = path.clone();
    app.cx.simulate_new_path_selection(move |_| Some(chosen));
    app.wait_for("Export to take clicks again", |window, _| {
        window
            .try_find("analysis-export")
            .is_some_and(|button| button.disabled() != Some(true))
    });
    std::fs::read(&path).expect("the saved file")
}

#[gpui_kit::test]
fn a_pgn_file_keeps_the_variations_comments_and_headers(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(
        &mut app,
        "[Date \"2001.02.03\"] 1. e4 e5 { The usual reply } (1... c5) 2. Nf3 *",
    );
    app.find("tree-e2e4 c7c5");
    let pgn = String::from_utf8(export(&mut app, "PGN file", "game.pgn")).expect("text");
    assert!(pgn.contains("[Date \"2001.02.03\"]"), "{pgn}");
    assert!(pgn.contains("The usual reply"), "{pgn}");
    assert!(pgn.contains("( 1... c5 )"), "{pgn}");
}

#[gpui_kit::test]
fn a_png_shows_the_position_in_the_variation_shown(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 (1... c5) *");
    app.click("tree-e2e4 c7c5");
    let in_variation = export(&mut app, "PNG of this position", "variation.png");
    // The same position, now on the main line.
    app.click("analysis-more");
    app.click_label("Make main line");
    app.click("tree-e2e4 c7c5");
    let on_main_line = export(&mut app, "PNG of this position", "main.png");
    assert!(in_variation == on_main_line, "the PNG shows 1... c5");
}

/// A Lichess game link pasted into the import fetches that game, opened from the side and at the
/// move the link names.
#[gpui_kit::test]
fn a_lichess_game_link_imports_the_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    app.online();
    import(
        &mut app,
        &format!(
            "https://lichess.org/{}/black#3",
            super::fake_lichess::LINKED_GAME
        ),
    );
    app.wait_gone("import-run");
    app.find("tree-e2e4 e7e5 g1f3 b8c6 f1b5 a7a6");
    assert_eq!(selected(&mut app, "e2e4 e7e5 g1f3"), Some(true));
}

/// The FEN of the position the analysis board shows.
fn shown_fen(app: &mut Harness) -> String {
    app.with_window(|_, cx| {
        let page = crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
        .expect("the analysis board");
        page.read(cx).shown_fen()
    })
}

/// Wait until the engine searches the position shown: a request standard rules refuse (one
/// sent without its variant) shows an error instead.
fn wait_for_search(app: &mut Harness) {
    app.wait_for("the engine to search", |_, cx| {
        crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
        .is_some_and(|page| page.read(cx).engine_text(cx).starts_with("Depth "))
    });
}

/// An Atomic game is played by Atomic's rules: its capture explodes, which standard chess would
/// not show. A variant KChess does not play is refused.
#[gpui_kit::test]
fn a_variant_game_is_played_by_its_own_rules(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "[Variant \"Bughouse\"]\n\n1. e4 d5 2. exd5 *");
    assert!(app.shows("import-run"), "the dialog stays");
    assert!(!app.shows("tree-e2e4"));
    fill_in_dialog(
        &mut app,
        "FEN, PGN or Lichess link",
        "[Variant \"Atomic\"]\n\n1. e4 d5 2. exd5 *",
    );
    app.click("import-run");
    app.wait_gone("import-run");
    app.click("tree-e2e4 d7d5 e4d5");
    // The capturing pawn exploded with the one it took.
    assert_eq!(
        shown_fen(&mut app),
        "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR b KQkq - 0 2"
    );
    // Fairy-Stockfish reviews the game; Lichess's database holds Atomic games too.
    assert!(app.shows("Review"));
    assert!(app.shows("Explorer"));
    wait_for_search(&mut app);
}

/// A Crazyhouse game shows its pockets by the board; a drop from one is played and analysed.
#[gpui_kit::test]
fn a_crazyhouse_drop_is_played_from_the_pocket(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(
        &mut app,
        "[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 *",
    );
    app.wait_gone("import-run");
    app.click("tree-e2e4 d7d5 e4d5 d8d5");
    app.find("pocket-White-Pawn");
    app.find("pocket-Black-Pawn");
    app.click("pocket-White-Pawn");
    app.click_square("e3", true);
    app.find("tree-e2e4 d7d5 e4d5 d8d5 P@e3");
    assert_eq!(selected(&mut app, "e2e4 d7d5 e4d5 d8d5 P@e3"), Some(true));
    // The drop's position goes to the engine by Crazyhouse's rules, which standard rules refuse.
    wait_for_search(&mut app);
    // The game keeps its variant in its PGN.
    app.click("analysis-more");
    app.wait_for("the menu", |window, _| {
        window.try_find_by_label("Copy PGN").is_some()
    });
    app.click_label("Copy PGN");
    let pgn = clipboard(&mut app).expect("a copied PGN");
    assert!(pgn.contains("[Variant \"Crazyhouse\"]"), "{pgn}");
    assert!(pgn.contains("3. P@e3"), "{pgn}");
    // The computer plays on from the drop, in Crazyhouse, as White; the player takes Black.
    app.click("analysis-more");
    app.wait_for("the menu", |window, _| {
        window
            .try_find_by_label("Play Stockfish from here")
            .is_some()
    });
    app.click_label("Play Stockfish from here");
    app.wait_for("a Crazyhouse game from the drop", |_, cx| {
        crate::shell::view_of::<crate::pages::computer::ComputerPage>(
            crate::pages::Page::Computer,
            cx,
        )
        .is_some_and(|page| {
            let page = page.read(cx);
            page.setup_variant() == "crazyhouse"
                && page
                    .setup_fen()
                    .starts_with("rnb1kbnr/ppp1pppp/8/3q4/8/4P3/PPPP1PPP/")
        })
    });
    // The board editor takes the position in Crazyhouse, pockets and all.
    app.open("Analysis board");
    app.click("analysis-more");
    app.wait_for("the menu", |window, _| {
        window.try_find_by_label("Edit this position").is_some()
    });
    app.click_label("Edit this position");
    wait_for_fen(
        &mut app,
        "rnb1kbnr/ppp1pppp/8/3q4/8/4P3/PPPP1PPP/RNBQKBNR[p] b KQkq - 0 1",
    );
}

/// The explorer looks a variant's position up in Lichess's games of that variant, offers only the
/// databases that hold them, and plays a drop it lists.
#[gpui_kit::test]
fn the_explorer_looks_up_a_crazyhouse_position_in_its_variant(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Analysis board");
    app.find("chess-board");
    import(
        &mut app,
        "[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 *",
    );
    app.wait_gone("import-run");
    app.click("tree-e2e4 d7d5 e4d5 d8d5");
    app.click("Explorer");
    app.click("explorer-lookup");
    app.wait_for("the Crazyhouse continuation", |window, _| {
        window.try_find_by_label("Play P@e3").is_some()
    });
    assert!(app.with_window(|window, _| window.try_find_by_label("Masters").is_none()));
    app.click_label("Play P@e3");
    app.click("Moves");
    assert_eq!(selected(&mut app, "e2e4 d7d5 e4d5 d8d5 P@e3"), Some(true));

    // Chess960 is standard chess from another start: the masters' games and the tablebase
    // still answer it.
    import(
        &mut app,
        "[Variant \"Chess960\"]\n[FEN \"bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1\"]\n\n1. e4 *",
    );
    app.wait_gone("import-run");
    app.click("tree-e2e4");
    app.click("Explorer");
    app.wait_for("every database", |window, _| {
        window.try_find_by_label("Masters").is_some()
            && window.try_find_by_label("Tablebase").is_some()
    });
}

/// A public Lichess study's link imports the chapter it names, or the study's first, in its own
/// variant.
#[gpui_kit::test]
fn a_lichess_study_link_imports_its_chapter(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::{STUDY, STUDY_CHAPTER, VARIANT_STUDY};
    let mut app = analysis(cx);
    app.online();
    import(
        &mut app,
        &format!("https://lichess.org/study/{STUDY}/{STUDY_CHAPTER}"),
    );
    app.wait_gone("import-run");
    app.find("tree-d2d4 d7d5 c2c4 e7e6");

    import(&mut app, &format!("lichess.org/study/{VARIANT_STUDY}"));
    app.wait_gone("import-run");
    app.find("tree-e2e4 d7d5 e4d5 d8d5");
    app.find("pocket-White-Pawn");
}

/// Quick Pick runs the editor's buttons: the board is cleared from it, and the actions an
/// unplayable position cannot take are left out until it is playable again.
#[gpui_kit::test]
fn the_editors_actions_are_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = editor(cx);
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
        settle(app);
    };
    run(&mut app, "Clear board");
    wait_for_fen(&mut app, "8/8/8/8/8/8/8/8 w - - 0 1");
    // No kings: nothing can be played or analysed from here.
    app.press("secondary-k");
    app.type_text(">Analyse this position");
    settle(&mut app);
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..10usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert!(
        !listed.iter().any(|label| label == "Analyse this position"),
        "{listed:?}"
    );
    app.press("escape");
    settle(&mut app);
    run(&mut app, "Starting position");
    wait_for_fen(&mut app, START_FEN);
}

/// Drop `name` holding `text` on the window, and let it open.
fn drop_file(app: &mut Harness, name: &str, text: &str) -> tempfile::TempDir {
    let folder = app.drop_file(name, text.as_bytes());
    settle(app);
    folder
}

/// A PGN file dropped on the window opens on the analysis board, from any page; over unsaved
/// analysis it asks first.
#[gpui_kit::test]
fn a_dropped_pgn_file_opens_on_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    let _file = drop_file(&mut app, "game.pgn", "1. e4 e5 2. Nf3 *");
    app.find("tree-e2e4 e7e5 g1f3");
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Analysis)
    );
    assert!(!app.shows("import-run"));
    // The board now holds unsaved moves: another file asks before replacing them.
    let _file = drop_file(&mut app, "other.PGN", "1. d4 d5 *");
    app.click("ok");
    settle(&mut app);
    app.find("tree-d2d4 d7d5");
    assert!(!app.shows("tree-e2e4"));
    assert!(!app.shows("import-run"));
}

/// A dropped FEN opens that position; a file the import cannot read keeps the dialog open with
/// the reason, and a file that is not a game is refused.
#[gpui_kit::test]
fn a_dropped_file_that_is_no_game_is_refused(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    let fen = "8/8/8/4k3/8/8/4P3/4K3 w - - 0 1";
    let _file = drop_file(&mut app, "ending.fen", fen);
    assert_eq!(shown_fen(&mut app), fen);
    let _file = drop_file(&mut app, "notes.txt", "not a game");
    app.find("import-error");
    app.click("import-cancel");
    settle(&mut app);
    let _file = drop_file(&mut app, "board.png", "png");
    assert!(!app.shows("import-run"));
    // A PGN over the import's 2 MB is refused before it is read, from any page.
    app.open("Puzzles");
    let big = format!("1. e4 e5 *{}", " ".repeat(2 * 1024 * 1024));
    let _file = drop_file(&mut app, "huge.pgn", &big);
    assert!(!app.shows("import-run"));
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Puzzles)
    );
}

/// The import's Open file… reads a PGN file from disk, as if it were pasted.
#[gpui_kit::test]
fn the_import_opens_a_pgn_file(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("game.pgn");
    std::fs::write(&path, "1. c4 e5 *").expect("the file");
    app.click("analysis-import");
    app.click("import-file");
    let started = std::time::Instant::now();
    while !app.cx.did_prompt_for_paths() {
        assert!(started.elapsed() < super::TIMEOUT, "no open dialog");
        app.tick();
    }
    app.cx
        .simulate_path_prompt_response(move |_| Some(vec![path]));
    settle(&mut app);
    app.find("tree-c2c4 e7e5");
    assert!(!app.shows("import-run"));
}

/// A file dragged over the window shows where it will go, and only a file from outside the app:
/// the zone is hidden before the drag, while it is over the window, and after it leaves.
#[gpui_kit::test]
fn a_dragged_file_shows_where_it_will_open(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::InputEvent as _;
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    assert!(!app.shows("drop-zone"));
    let position = app.with_window(|window, _| {
        let size = window.viewport_size();
        gpui_kit::point(size.width * 0.6, size.height * 0.5)
    });
    let paths = gpui_kit::ExternalPaths(std::iter::once("game.pgn".into()).collect());
    app.with_window(|window, cx| {
        window.dispatch_event(
            gpui_kit::FileDropEvent::Entered { position, paths }.to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::FileDropEvent::Pending { position }.to_platform_input(),
            cx,
        );
    });
    settle(&mut app);
    assert!(app.shows("drop-zone"));
    app.with_window(|window, cx| {
        window.dispatch_event(gpui_kit::FileDropEvent::Exited.to_platform_input(), cx)
    });
    settle(&mut app);
    assert!(!app.shows("drop-zone"));
}

/// X shows the opponent's threat, as on Lichess: the engine searches the position with the other
/// side to move, Space does not play the threat's move, and X again goes back. In check there is
/// no threat to show.
#[gpui_kit::test]
fn x_shows_the_threat(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    let page = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
                crate::pages::Page::Analysis,
                cx,
            )
            .expect("the analysis board")
        })
    };
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.find("tree-e2e4");
    wait_for_search(&mut app);
    app.click("chess-board");
    app.press("x");
    let analysis = page(&mut app);
    let threat = app
        .cx
        .update(|cx| analysis.read(cx).threat_shown())
        .expect("a threat to search");
    // White to move again, en passant forgotten.
    assert_eq!(
        threat,
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1"
    );
    app.wait_for("the threat to be searched", |_, cx| {
        analysis
            .read(cx)
            .engine_text(cx)
            .starts_with("Threat · Depth ")
    });
    // Space plays the best move of the position shown, never the threat's.
    app.press("space");
    settle(&mut app);
    assert!(!app.shows("tree-e2e4 e7e5"));
    assert_eq!(
        app.cx.update(|cx| analysis.read(cx).shown_fen()),
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
    );
    app.press("x");
    assert_eq!(app.cx.update(|cx| analysis.read(cx).threat_shown()), None);
    wait_for_search(&mut app);

    // In check the side to move cannot pass: no threat, the ordinary search.
    import(
        &mut app,
        "rnbqkbnr/ppppp1pp/8/5p1Q/4P3/8/PPPP1PPP/RNB1KBNR b KQkq - 1 2",
    );
    app.click("chess-board");
    app.press("x");
    assert_eq!(app.cx.update(|cx| analysis.read(cx).threat_shown()), None);
    wait_for_search(&mut app);
}

/// Copy line copies the moves to the one shown, without the side lines or what follows.
#[gpui_kit::test]
fn the_line_to_a_move_is_copied(cx: &mut gpui_kit::TestAppContext) {
    let mut app = analysis(cx);
    import(&mut app, "1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 *");
    app.click("tree-e2e4 e7e5 g1f3");
    app.click("analysis-more");
    app.click_label("Copy line");
    let copied = app
        .cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(copied.contains("1. e4 e5 2. Nf3 *"), "{copied}");
    assert!(
        !copied.contains("c5") && !copied.contains("Nc6"),
        "{copied}"
    );

    // Quick Pick copies the line to the move shown, as the menu does.
    app.click("tree-e2e4 e7e5");
    app.press("secondary-k");
    app.type_text("Copy line");
    super::palette::wait_for_first(&mut app, "Copy line");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    let copied = app
        .cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(copied.contains("1. e4 e5 *"), "{copied}");
}
