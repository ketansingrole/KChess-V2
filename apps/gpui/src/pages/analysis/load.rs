//! How other pages put something on the analysis board and show it.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::*;

use kchess_contracts::calls;
use kchess_contracts::check::{GameId, LichessId, StudyId};

use super::{AnalysisPage, Opening};
use crate::core::AppCore;
use crate::game::played::GameOrigin;
use crate::game::session::SessionSaver;
use crate::pages::Page;

/// The analysis board, made now if it has not been shown (so analysis kept from an earlier run
/// counts).
pub fn board(window: &mut Window, cx: &mut App) -> Option<Entity<AnalysisPage>> {
    crate::shell::made::<AnalysisPage>(window, cx)
}

/// Run `then` with the analysis board once it has read the session it opens with (made now if
/// it has not been shown), so what it shows is what the profile kept. Keys the person has given
/// elsewhere meanwhile stay there (`keeping_keys_since`).
pub fn when_read(
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(Entity<AnalysisPage>, &mut Window, &mut App) + 'static,
) {
    let Some(analysis) = board(window, cx) else {
        return;
    };
    let asked = window.focused(cx);
    analysis.update(cx, |page, cx| {
        SessionSaver::when_read(
            page,
            move |_, window, cx| {
                let analysis = cx.entity();
                window.defer(cx, move |window, cx| {
                    crate::shell::keeping_keys_since(asked, window, cx, |window, cx| {
                        then(analysis, window, cx)
                    })
                });
            },
            window,
            cx,
        );
    });
}

/// Put something else on the analysis board and show it, asking first while the board holds
/// moves no study keeps. `load` says whether the board took it.
pub fn replace(
    window: &mut Window,
    cx: &mut App,
    load: impl Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool + 'static,
) {
    when_read(window, cx, move |analysis, window, cx| {
        replace_read(analysis, window, cx, load)
    });
}

pub fn replace_read(
    analysis: Entity<AnalysisPage>,
    window: &mut Window,
    cx: &mut App,
    load: impl Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>) -> bool + 'static,
) {
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
    game: kchess_contracts::types::MistakeGame,
    items: Vec<kchess_contracts::types::Drill>,
    window: &mut Window,
    cx: &mut App,
) {
    when_read(window, cx, move |analysis, window, cx| {
        if analysis.read(cx).mainline_key() == game.key {
            analysis.update(cx, |analysis, cx| analysis.practise_game(&game, items, cx));
            crate::shell::open(Page::Analysis, window, cx);
            return;
        }
        replace_read(analysis, window, cx, move |analysis, _, cx| {
            analysis.practise_game(&game, items.clone(), cx)
        });
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
    puzzle: kchess_contracts::types::Puzzle,
    wrong_moves: Vec<kchess_contracts::types::PuzzleWrongMove>,
    window: &mut Window,
    cx: &mut App,
) {
    replace(window, cx, move |analysis, _, cx| {
        analysis.open_puzzle(&puzzle, &wrong_moves, cx)
    });
}

/// A finished Lichess game, fetched as PGN (`exportGame`), on the analysis board facing
/// `orientation`; asks first when that replaces unsaved analysis.
pub fn open_exported(
    id: &str,
    white: String,
    black: String,
    orientation: shakmaty::Color,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = GameOrigin {
        white: Some(white),
        black: Some(black),
        ..GameOrigin::lichess(id)
    };
    open_exported_at(id, origin, orientation, 0, window, cx);
}

/// A Lichess game by its link (`crate::game::game_link`), from the side and at the move it names.
pub fn open_linked(link: &crate::game::GameLink, window: &mut Window, cx: &mut App) {
    open_exported_at(
        &link.id,
        GameOrigin::lichess(&link.id),
        link.orientation,
        link.ply.unwrap_or(0),
        window,
        cx,
    );
}

fn open_exported_at(
    id: &str,
    origin: GameOrigin,
    orientation: shakmaty::Color,
    ply: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let export = cx.global::<AppCore>().request(calls::ExportGame {
        id: GameId(id.to_string()),
    });
    let opening = Opening {
        ply,
        orientation: Some(orientation),
        origin: Some(origin),
        review: false,
    };
    let pgn = async move {
        match export.await {
            Ok(pgn) => Ok(pgn),
            Err(cause) => Err(format!("Could not open the game: {}", cause.message)),
        }
    };
    open_fetched(pgn, opening, window, cx);
}

/// A public Lichess study's chapter (its first when `chapter` is empty), from the side the study
/// shows it.
pub fn open_study(study: &str, chapter: &str, window: &mut Window, cx: &mut App) {
    let fetch = cx.global::<AppCore>().request(calls::LichessStudyChapter {
        id: LichessId(study.to_string()),
        chapter: StudyId(chapter.to_string()),
    });
    window
        .spawn(cx, async move |cx| {
            let pgn = match fetch.await {
                Ok(found) => Some(found.pgn),
                Err(cause) => {
                    let message = format!("Could not open the study: {}", cause.message);
                    let _ = cx.update(|window, cx| window.push_notification(message, cx));
                    return;
                }
            };
            let _ = cx.update(|window, cx| {
                let orientation = pgn.as_deref().and_then(crate::game::pgn_orientation);
                let pgn =
                    async move { pgn.ok_or_else(|| "That study could not be read".to_string()) };
                let opening = Opening {
                    orientation,
                    ..Opening::default()
                };
                open_fetched(pgn, opening, window, cx);
            });
        })
        .detach();
}

/// Open a game fetched from Lichess on the analysis board (asking first when the board holds
/// unsaved work), or say why it cannot be opened.
fn open_fetched(
    pgn: impl std::future::Future<Output = Result<String, String>> + 'static,
    opening: Opening,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let pgn = pgn.await;
            let _ = cx.update(|window, cx| match pgn {
                // A variant the analysis board cannot play (Crazyhouse, Atomic…): say so before
                // asking to replace anything.
                Ok(pgn)
                    if !crate::game::pgn_analysable(&pgn)
                        || crate::game::tree::TreeNode::from_pgn(&pgn).is_none() =>
                {
                    window.push_notification("The analysis board cannot open this game", cx)
                }
                Ok(pgn) => {
                    crate::pages::analysis::replace(window, cx, move |analysis, _, cx| {
                        analysis.open_pgn(&pgn, opening.clone(), cx)
                    });
                }
                Err(message) => window.push_notification(message, cx),
            });
        })
        .detach();
}
