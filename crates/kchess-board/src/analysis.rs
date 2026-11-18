//! Opening a game on the analysis board from any page without naming the page: a page sends what
//! to open (`Open`) and the app's analysis page, which answers through `AnalysisBoard`, opens it,
//! asking first when that would replace unsaved analysis. Also what a page may ask of the board:
//! what it shows, and to save the study it shows now.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gpui_kit::{App, Global, Hsla, Window, rgb};
use kchess_contracts::types::{Drill, LichessGame, MistakeGame, Puzzle, PuzzleWrongMove};
use kchess_domain::variant::Setup;
use shakmaty::Color;

use crate::game::played::{GameOrigin, PlayedGame};
use crate::theme::judgment_color;
use kchess_domain::links::GameLink;

/// A move's classification as a review shows it: its name, its mark and its colour.
pub fn classification_look(classification: &str, dark: bool) -> (&'static str, &'static str, Hsla) {
    let fixed = |hex: u32| -> Hsla { rgb(hex).into() };
    match classification {
        "brilliant" => ("Brilliant", "!!", fixed(0x168c9c)),
        "best" => ("Best", "★", fixed(0x538425)),
        "excellent" => ("Excellent", "✓", fixed(0x658443)),
        "good" => ("Good", "•", fixed(0x697f51)),
        "inaccuracy" => ("Inaccuracy", "?!", judgment_color("inaccuracy", dark)),
        "mistake" => ("Mistake", "?", judgment_color("mistake", dark)),
        _ => ("Blunder", "??", judgment_color("blunder", dark)),
    }
}

/// How a game opens on the board.
#[derive(Clone, Default)]
pub struct Opening {
    /// The move shown.
    pub ply: usize,
    /// How the board faces; as it was when `None`.
    pub orientation: Option<Color>,
    /// Where the game came from.
    pub origin: Option<GameOrigin>,
    /// Review it once open.
    pub review: bool,
}

/// What to open on the analysis board.
pub enum Open {
    /// A game's PGN, as `opening` says; `unreadable` says so when it cannot be read.
    Pgn {
        pgn: String,
        opening: Opening,
        unreadable: bool,
    },
    /// A position, or the start when `None`.
    Position(Option<String>),
    /// Text pasted or dropped: a PGN, a FEN or a list of moves.
    Import(String),
    /// A game played here, reviewed once open when `review`.
    Played {
        game: PlayedGame,
        review: bool,
    },
    /// A position set up in the editor, facing `orientation`.
    Setup {
        setup: Setup,
        orientation: Color,
    },
    /// Moves from the start (an opening's line).
    Line(Vec<String>),
    /// One of the library's studies, or a new one named `name` from the start position.
    Study(String),
    NewStudy(String),
    /// A library study was replaced (its cloud copy kept): shown again if the board shows it, or
    /// the board's unsaved edits to it saved over it.
    StudyChanged(String),
    /// A chapter of a Lichess study.
    LichessChapter {
        study: String,
        chapter: String,
    },
    /// A Lichess game by its link, or exported by its id with its players.
    Linked(GameLink),
    Exported {
        id: String,
        white: String,
        black: String,
        orientation: Color,
    },
    /// An earlier puzzle, with the moves tried that were wrong.
    Puzzle {
        puzzle: Puzzle,
        wrong_moves: Vec<PuzzleWrongMove>,
    },
    /// A reviewed game's positions to practise.
    Practise {
        game: MistakeGame,
        items: Vec<Drill>,
    },
}

/// What the analysis board shows, for a page that sends it on (a Lichess study's chapter).
pub struct Shown {
    pub study_id: String,
    pub pgn: String,
    /// The PGN's `Event` header, empty when it has none.
    pub event: String,
}

/// Saving what the board has not saved yet: whether it was.
pub type Saving = Pin<Box<dyn Future<Output = Result<(), String>>>>;

/// The analysis board as other pages reach it, answered by the app's analysis page.
pub trait AnalysisBoard {
    fn open(&self, open: Open, window: &mut Window, cx: &mut App);
    /// What the board shows, when it has been made.
    fn shown(&self, window: &mut Window, cx: &mut App) -> Option<Shown>;
    /// Save the study it shows now; `None` when it has not been made.
    fn save_study(&self, window: &mut Window, cx: &mut App) -> Option<Saving>;
}

/// The app's analysis board (`cx.set_global(Analysis(Rc::new(…)))`).
#[derive(Clone)]
pub struct Analysis(pub Rc<dyn AnalysisBoard>);

impl Global for Analysis {}

fn board(cx: &App) -> Option<Rc<dyn AnalysisBoard>> {
    cx.try_global::<Analysis>()
        .map(|Analysis(board)| board.clone())
}

/// Open `open` on the analysis board.
pub fn open(open: Open, window: &mut Window, cx: &mut App) {
    if let Some(board) = board(cx) {
        board.open(open, window, cx);
    }
}

/// Open a Lichess game of an account's on the analysis board, from that account's side.
pub fn open_lichess_game(game: LichessGame, window: &mut Window, cx: &mut App) {
    let (white, black) = match game.color {
        Color::White => (game.account, game.opponent),
        Color::Black => (game.opponent, game.account),
    };
    open(
        Open::Exported {
            id: game.id,
            white,
            black,
            orientation: game.color,
        },
        window,
        cx,
    );
}

/// What the analysis board shows, when it has been made.
pub fn shown(window: &mut Window, cx: &mut App) -> Option<Shown> {
    board(cx)?.shown(window, cx)
}

/// Save the study the analysis board shows now; `None` when it has not been made.
pub fn save_study(window: &mut Window, cx: &mut App) -> Option<Saving> {
    board(cx)?.save_study(window, cx)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A board that keeps what it was asked.
    #[derive(Default)]
    struct Kept(RefCell<Vec<String>>);

    impl AnalysisBoard for Kept {
        fn open(&self, open: Open, _: &mut Window, _: &mut App) {
            match open {
                Open::Study(id) => self.0.borrow_mut().push(id),
                Open::Exported {
                    id,
                    white,
                    black,
                    orientation,
                } => self
                    .0
                    .borrow_mut()
                    .push(format!("{id}: {white} v {black}, {orientation:?} below")),
                _ => {}
            }
        }

        fn shown(&self, _: &mut Window, _: &mut App) -> Option<Shown> {
            Some(Shown {
                study_id: "study".into(),
                pgn: "1. e4".into(),
                event: String::new(),
            })
        }

        fn save_study(&self, _: &mut Window, _: &mut App) -> Option<Saving> {
            Some(Box::pin(async { Err("refused".to_string()) }))
        }
    }

    struct Blank;

    impl gpui_kit::Render for Blank {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }

    /// Before the app sets its board nothing opens; then each request reaches it, and what it
    /// shows and saves is its own.
    #[gpui_kit::test]
    fn requests_reach_the_board_the_app_set(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let window = cx.add_window(|_, _| Blank);
        let board = Rc::new(Kept::default());
        let saved = cx
            .update_window(window.into(), |_, window, cx| {
                open(Open::Study("before".into()), window, cx);
                assert!(shown(window, cx).is_none());
                assert!(save_study(window, cx).is_none());
                cx.set_global(Analysis(board.clone()));
                open(Open::Study("after".into()), window, cx);
                for color in ["white", "black"] {
                    let game = serde_json::from_value(serde_json::json!({
                        "id": "g1", "account": "alice", "opponent": "bob", "color": color,
                        "createdAt": 0, "lastMoveAt": 0, "rated": true, "speed": "blitz",
                        "perf": "blitz", "status": "mate", "moves": "e4 e5",
                    }))
                    .expect("a game");
                    open_lichess_game(game, window, cx);
                }
                assert_eq!(
                    shown(window, cx).map(|shown| shown.pgn),
                    Some("1. e4".into())
                );
                save_study(window, cx).expect("a save")
            })
            .unwrap();
        assert_eq!(
            *board.0.borrow(),
            [
                "after",
                "g1: alice v bob, White below",
                "g1: bob v alice, Black below"
            ]
        );
        assert_eq!(
            futures::executor::block_on(saved),
            Err("refused".to_string())
        );
    }
}
