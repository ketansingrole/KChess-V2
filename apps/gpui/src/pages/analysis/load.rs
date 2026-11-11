//! How other pages put something on the analysis board and show it.

use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::*;

use super::AnalysisPage;
use crate::pages::Page;

/// The analysis board, made now if it has not been shown (so analysis kept from an earlier run
/// counts).
pub fn board(window: &mut Window, cx: &mut App) -> Option<Entity<AnalysisPage>> {
    crate::shell::made::<AnalysisPage>(window, cx)
}

/// Put something else on the analysis board and show it, asking first while the board holds
/// moves no study keeps. `load` says whether the board took it.
pub fn replace(
    window: &mut Window,
    cx: &mut App,
    load: impl Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool + 'static,
) {
    let Some(analysis) = board(window, cx) else {
        return;
    };
    let load = std::rc::Rc::new(load);
    let run = {
        let analysis = analysis.clone();
        move |load: &dyn Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool,
              window: &mut Window,
              cx: &mut App| {
            if analysis.update(cx, |analysis, cx| load(analysis, window, cx)) {
                crate::shell::open(Page::Analysis, window, cx);
            }
        }
    };
    if !analysis.read(cx).unsaved() {
        run(&*load, window, cx);
        return;
    }
    crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let (load, run) = (load.clone(), run.clone());
        dialog
            .title("Replace the unsaved analysis?")
            .description("The analysis board has moves that are not saved as a study")
            .ok_text("Replace")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                run(&*load, window, cx);
                true
            })
    });
}

/// An empty analysis board.
pub fn new_board(window: &mut Window, cx: &mut App) {
    replace(window, cx, |analysis, _, cx| {
        analysis.load(None, cx);
        true
    });
}

/// Practise a reviewed game's positions on the analysis board; another game replaces the
/// board's only after asking, when that would lose unsaved analysis.
pub fn practise_game(
    game: serde_json::Value,
    items: Vec<serde_json::Value>,
    window: &mut Window,
    cx: &mut App,
) {
    let shown = board(window, cx).is_some_and(|analysis| {
        analysis.read(cx).mainline_key() == game["key"].as_str().unwrap_or_default()
    });
    if shown {
        if let Some(analysis) = board(window, cx) {
            analysis.update(cx, |analysis, cx| analysis.practise_game(&game, items, cx));
        }
        crate::shell::open(Page::Analysis, window, cx);
        return;
    }
    replace(window, cx, move |analysis, _, cx| {
        analysis.practise_game(&game, items.clone(), cx)
    });
}

/// Analyse a line from the starting position.
pub fn open_line(moves: Vec<String>, window: &mut Window, cx: &mut App) {
    replace(window, cx, move |analysis, _, cx| {
        analysis.load_line(&moves, cx);
        true
    });
}

/// Study an earlier puzzle on the analysis board, asking first when that would replace unsaved
/// analysis.
pub fn analyse_puzzle(
    puzzle: serde_json::Value,
    wrong_moves: Vec<serde_json::Value>,
    window: &mut Window,
    cx: &mut App,
) {
    replace(window, cx, move |analysis, _, cx| {
        analysis.open_puzzle(&puzzle, &wrong_moves, cx)
    });
}
