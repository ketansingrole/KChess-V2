//! What comes into the app from outside it: pasted text (a Lichess link, moves, a position) and
//! dropped files. Paste outside a text field, Quick Pick and a dropped file all read it here, so
//! they agree on what each one is and where it opens.

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;

use crate::game::{BroadcastLink, GameLink};
use crate::pages::analysis::Opening;
use crate::pages::{ActionInfo, PageCommand};

/// Something pasted that KChess can open.
pub enum Link {
    StudyChapter {
        study: String,
        chapter: String,
    },
    Broadcast(BroadcastLink),
    Tv(String),
    Tournament {
        system: &'static str,
        id: String,
    },
    Profile(String),
    Puzzle(String),
    Game(GameLink),
    /// Numbered moves, with or without PGN headers, the analysis board can play.
    Moves(String),
    /// A legal position's FEN.
    Position(String),
}

/// What `text` is, tried in this order: a Lichess link (a study chapter, a broadcast, a TV
/// channel, a tournament, a profile, a puzzle, a game), numbered moves, then a FEN.
pub fn classify(text: &str) -> Option<Link> {
    if let Some((study, chapter)) = crate::game::study_link(text) {
        return Some(Link::StudyChapter { study, chapter });
    }
    if let Some(link) = crate::game::broadcast_link(text) {
        return Some(Link::Broadcast(link));
    }
    if let Some(channel) = kchess_domain::links::tv_link(text) {
        return Some(Link::Tv(channel));
    }
    if let Some((system, id)) = kchess_domain::links::tournament_link(text) {
        return Some(Link::Tournament { system, id });
    }
    if let Some(name) = crate::game::profile_link(text) {
        return Some(Link::Profile(name));
    }
    if let Some(id) = kchess_domain::links::training_link(text) {
        return Some(Link::Puzzle(id));
    }
    if let Some(link) = crate::game::game_link(text) {
        return Some(Link::Game(link));
    }
    if let Some(pgn) = movetext(text) {
        return Some(Link::Moves(pgn));
    }
    text.contains('/')
        .then(|| kchess_domain::editor::EditorSetup::from_fen(text))
        .flatten()
        .map(|setup| setup.fen())
        .filter(|fen| crate::game::position_problem(fen).is_none())
        .map(Link::Position)
}

/// Numbered moves (`1. e4 e5 2. Nf3`, with or without PGN headers) the analysis board can play.
fn movetext(text: &str) -> Option<String> {
    let numbered = text.split_whitespace().any(|word| word.starts_with("1."));
    (numbered && crate::game::pgn_analysable(text))
        .then(|| crate::game::tree::TreeNode::from_pgn(text))
        .flatten()
        .filter(|tree| !tree.children.is_empty())
        .map(|_| text.to_string())
}

fn command(
    info: ActionInfo,
    detail: Option<String>,
    run: impl Fn(&mut Window, &mut App) + 'static,
) -> PageCommand {
    PageCommand {
        info,
        detail: detail.map(Into::into),
        run: Rc::new(run),
    }
}

impl Link {
    /// What the link can open, first what a paste opens.
    pub fn commands(self) -> Vec<PageCommand> {
        match self {
            Link::StudyChapter { study, chapter } => vec![command(
                ActionInfo::new(
                    format!("pasted:study:{study}:{chapter}"),
                    "Open this study chapter on the analysis board",
                    "microscope",
                    "",
                ),
                None,
                move |window, cx| crate::pages::analysis::open_study(&study, &chapter, window, cx),
            )],
            Link::Broadcast(link) => vec![command(
                ActionInfo::new("pasted:broadcast", "Watch this broadcast", "radio", ""),
                None,
                move |window, cx| {
                    let link = match &link {
                        BroadcastLink::Tour(tour) => BroadcastLink::Tour(tour.clone()),
                        BroadcastLink::Round { round, game } => BroadcastLink::Round {
                            round: round.clone(),
                            game: game.clone(),
                        },
                    };
                    crate::pages::watch::open_broadcast(link, window, cx)
                },
            )],
            Link::Tv(channel) => vec![command(
                ActionInfo::new(
                    format!("pasted:tv:{channel}"),
                    "Watch this TV channel",
                    "tv",
                    "",
                ),
                None,
                move |window, cx| crate::pages::watch::open_channel(channel.clone(), window, cx),
            )],
            Link::Tournament { system, id } => vec![command(
                ActionInfo::new(
                    format!("pasted:tournament:{id}"),
                    "Open this tournament",
                    "trophy",
                    "",
                ),
                None,
                move |window, cx| crate::pages::tournaments::open_event(system, &id, window, cx),
            )],
            Link::Profile(name) => vec![command(
                ActionInfo::new(
                    format!("pasted:player:{name}"),
                    format!("Open {name}’s profile"),
                    "user-search",
                    "",
                ),
                None,
                move |window, cx| crate::pages::players::open_player(&name, window, cx),
            )],
            Link::Puzzle(id) => vec![command(
                ActionInfo::new(
                    format!("pasted:puzzle:{id}"),
                    "Solve this puzzle",
                    "puzzle",
                    "",
                ),
                Some(id.clone()),
                move |window, cx| crate::pages::puzzles::open_puzzle(id.clone(), window, cx),
            )],
            Link::Game(link) => {
                let id = link.id.clone();
                vec![command(
                    ActionInfo::new(
                        format!("pasted:game:{id}"),
                        "Open this game on the analysis board",
                        "microscope",
                        "",
                    ),
                    Some(id),
                    move |window, cx| crate::pages::analysis::open_linked(&link, window, cx),
                )]
            }
            Link::Moves(pgn) => vec![command(
                ActionInfo::new("pasted:moves", "Analyse these moves", "microscope", ""),
                None,
                move |window, cx| {
                    let pgn = pgn.clone();
                    crate::pages::analysis::replace(window, cx, move |analysis, _, cx| {
                        let opening = Opening {
                            ply: usize::MAX,
                            ..Opening::default()
                        };
                        analysis.open_pgn(&pgn, opening, cx)
                    });
                },
            )],
            Link::Position(fen) => {
                let orientation = kchess_domain::rules::fen_turn(&fen);
                let (analysed, edited) = (fen.clone(), fen);
                vec![
                    command(
                        ActionInfo::new(
                            "pasted:analyse",
                            "Analyse this position",
                            "microscope",
                            "",
                        ),
                        None,
                        move |window, cx| {
                            let fen = analysed.clone();
                            crate::pages::analysis::replace(window, cx, move |analysis, _, cx| {
                                analysis.load(Some(&fen), cx);
                                true
                            });
                        },
                    ),
                    command(
                        ActionInfo::new("pasted:edit", "Edit this position", "pencil-ruler", ""),
                        None,
                        move |window, cx| {
                            let setup = crate::game::setup_from_fen(&edited);
                            crate::pages::editor::edit_position(setup, orientation, window, cx)
                        },
                    ),
                ]
            }
        }
    }
}

/// What a pasted FEN, moves or Lichess link can open, for Quick Pick.
pub fn pasted(text: &str) -> Vec<PageCommand> {
    classify(text).map(Link::commands).unwrap_or_default()
}

/// Paste outside a text field: what the clipboard holds opens as Quick Pick's first suggestion
/// for it would (a game or position on the analysis board, a Lichess link where it belongs).
/// Under a dialog nothing happens; anything else is said in a notice.
pub fn open_pasted(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    crate::slow_calls::call("the clipboard");
    let text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    match pasted(text.trim()).into_iter().next() {
        Some(command) => (command.run)(window, cx),
        None => window.push_notification("Copy a PGN, a FEN or a Lichess link to open it", cx),
    }
}

// Reading a game file, here and from the pages that open one (`kchess_board::export`).
pub use crate::export::{choose_game_file, read_game_file};

/// A PGN or FEN file dropped on the window, from any page: one game or a position opens on the
/// analysis board, as if pasted into its import; several games are offered as a study, in the
/// Studies import; anything else is refused with a notice. On the Database page a PGN file, or a
/// `.zip` or `.7z` of them, is imported into a game database instead.
pub fn open_dropped(paths: &ExternalPaths, window: &mut Window, cx: &mut App) {
    let Some(path) = paths.paths().first().cloned() else {
        return;
    };
    // On the Database page a PGN file of any size, or a zip of them, is imported into a game
    // database.
    let name = path.to_string_lossy().to_ascii_lowercase();
    if crate::shell::current_page(cx) == Some(crate::pages::Page::Database)
        && [".pgn", ".pgn.zst", ".pgn.bz2", ".zip", ".7z"]
            .iter()
            .any(|extension| name.ends_with(extension))
    {
        crate::pages::database::import_dropped(path, cx);
        return;
    }
    // Read off the main thread: a file on a stalled network folder froze every window.
    let read = cx.background_spawn(async move { read_game_file(&path) });
    window
        .spawn(cx, async move |cx| {
            let text = read.await;
            cx.update(|window, cx| open_dropped_text(text, window, cx))
                .ok();
        })
        .detach();
}

/// What `open_dropped` read: opened as a game, a position or a study, or refused.
fn open_dropped_text(text: Option<String>, window: &mut Window, cx: &mut App) {
    let Some(text) = text else {
        window.push_notification(
            gpui_kit::component::notification::Notification::error(
                "Drop a PGN or FEN file under 2 MB",
            )
            .title("Could not open that file"),
            cx,
        );
        return;
    };
    let games = kchess_domain::position::pgn_games(&text).len();
    if games > 1 {
        crate::shell::open(crate::pages::Page::Studies, window, cx);
        crate::pages::studies::StudiesPage::open_import_with(Some(text), window, cx);
        return;
    }
    crate::pages::analysis::replace(window, cx, move |analysis, window, cx| {
        analysis.import_text(text.clone(), window, cx);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::{Link, classify};

    /// Each kind of text is read as what it is, a study's link before a game's.
    #[test]
    fn pasted_text_is_read_as_what_it_is() {
        let kind = |text: &str| match classify(text) {
            Some(Link::StudyChapter { .. }) => "study",
            Some(Link::Broadcast(_)) => "broadcast",
            Some(Link::Tv(_)) => "tv",
            Some(Link::Tournament { .. }) => "tournament",
            Some(Link::Profile(_)) => "profile",
            Some(Link::Puzzle(_)) => "puzzle",
            Some(Link::Game(_)) => "game",
            Some(Link::Moves(_)) => "moves",
            Some(Link::Position(_)) => "position",
            None => "nothing",
        };
        assert_eq!(kind("https://lichess.org/study/abcdEFGH/ijklMNOP"), "study");
        assert_eq!(kind("https://lichess.org/@/someone"), "profile");
        assert_eq!(kind("https://lichess.org/tv/blitz"), "tv");
        assert_eq!(kind("https://lichess.org/abcdEFGH"), "game");
        assert_eq!(kind("1. e4 e5 2. Nf3"), "moves");
        assert_eq!(
            kind("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"),
            "position"
        );
        assert_eq!(kind("hello"), "nothing");
    }
}
