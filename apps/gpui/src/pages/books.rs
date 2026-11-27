//! Books: a chess book read beside a board. The book's page on the left, as printed; its
//! diagrams outlined on it, and one clicked set up on the board at the right, read from the
//! picture (`kchess_board::book`). Moves played there are checked against the rules, and
//! Stockfish under the board is off until asked for, so the reader works the position out first.
//! The two sides, and the board and the engine, are split by dividers that keep where they were
//! dragged.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, ResizableState, Sizable as _, h_flex, h_resizable, resizable_panel, v_flex,
    v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::book::{self, Book, DrawnPage};
use kchess_board::picture::{Area, Recognised};
use kchess_contracts::calls;
use kchess_contracts::types::{AnalysisRequest, AnalysisUpdate};
use kchess_domain::variant::{Setup, Variant};
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position as _, Square};

use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard, Circle};
use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::log::ResultExt as _;
use crate::position_engine::PositionEngine;
use crate::theme::{Palette, radii, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

/// The book open last, and the page it was on.
const BOOK_PATH: &str = "kchess:book-path";
const BOOK_PAGE: &str = "kchess:book-page";
/// Where the dividers were dragged: the book's and the board's sides, and the board and the
/// engine.
const SPLIT: &str = "kchess:book-split";
const STACK: &str = "kchess:book-stack";
/// Whether Stockfish follows the board.
const ENGINE_ON: &str = "kchess:book-engine";
const LINES: u32 = 3;
/// The toolbar's height, and the room the board leaves around itself in its panel.
const TOOLBAR: Pixels = px(48.);
const BOARD_MARGIN: Pixels = px(16.);
/// The smallest each side and the board's panel may be dragged to.
const BOOK_MIN: Pixels = px(240.);
const SIDE_MIN: Pixels = px(280.);
const ENGINE_MIN: Pixels = px(120.);

/// What a click on the next or previous diagram goes to once its page is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    First,
    Last,
}

/// The position set up from a diagram, and the moves played from it.
struct Study {
    /// The page and the diagram's place among the page's.
    page: usize,
    diagram: usize,
    start: VariantPosition,
    orientation: Color,
    /// Squares the picture was read with little certainty, ringed until a move is played.
    doubtful: Vec<Square>,
    /// Each move as UCI and SAN, and the position after it.
    moves: Vec<(String, String, VariantPosition)>,
}

impl Study {
    fn position(&self) -> &VariantPosition {
        self.moves
            .last()
            .map_or(&self.start, |(_, _, position)| position)
    }

    fn start_fen(&self) -> String {
        kchess_domain::rules::make_fen(&self.start)
    }
}

pub struct BooksPage {
    focus: FocusHandle,
    book: Option<(PathBuf, Book)>,
    page: usize,
    drawn: Option<DrawnPage>,
    /// Opening a book, drawing a page and reading a diagram: each replaced by the next.
    opening: Option<Task<()>>,
    drawing: Option<Task<()>>,
    reading: Option<Task<()>>,
    then: Option<Then>,
    /// The diagram being read, while it is.
    reading_diagram: Option<(usize, usize)>,
    study: Option<Study>,
    /// A diagram read as a position chess does not allow (no king, a pawn on the back rank):
    /// shown as read, to set right in the board editor.
    unreadable_fen: Option<(String, Color, Vec<Square>)>,
    /// What went wrong last: a book not opened, a diagram not read.
    problem: Option<SharedString>,
    board: Entity<ChessBoard>,
    engine: PositionEngine,
    engine_ready: bool,
    split: Entity<ResizableState>,
    stack: Entity<ResizableState>,
    shown: bool,
    _subscriptions: Vec<Subscription>,
}

fn engine_of(page: &mut BooksPage) -> &mut PositionEngine {
    &mut page.engine
}

/// Engine help may show: no live Lichess game, and none being looked for.
fn assistance(cx: &App) -> bool {
    cx.global::<crate::online::OnlineHandle>()
        .0
        .read(cx)
        .assistance_allowed()
}

impl Focusable for BooksPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BooksPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> BooksPage {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &VariantPosition::Chess(shakmaty::Chess::default()),
                Color::White,
                None,
                None,
            ))
        });
        let split = cx.new(|_| ResizableState::default());
        let stack = cx.new(|_| ResizableState::default());
        let events = cx.global::<AppCore>().events.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(std::slice::from_ref(uci), cx);
                }
            }),
            cx.subscribe_in(
                &events,
                window,
                |this, _, event: &ClientEvent, _, cx| match event {
                    ClientEvent::Event(CoreEvent::Analysis(update)) => {
                        this.engine_update(update, cx)
                    }
                    ClientEvent::Reconnected => {
                        this.engine.retry(engine_of, cx);
                    }
                    _ => {}
                },
            ),
            // The board's size follows the dividers as they are dragged.
            cx.observe(&split, |_, _, cx| cx.notify()),
            cx.observe(&stack, |_, _, cx| cx.notify()),
        ];
        BooksPage {
            focus: cx.focus_handle(),
            book: None,
            page: 0,
            drawn: None,
            opening: None,
            drawing: None,
            reading: None,
            then: None,
            reading_diagram: None,
            study: None,
            unreadable_fen: None,
            problem: None,
            board,
            engine: PositionEngine::default(),
            engine_ready: false,
            split,
            stack,
            shown: false,
            _subscriptions: subscriptions,
        }
    }

    /// The page is shown: the book open last reopens where it was left, and the engine's
    /// readiness is asked for.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = true;
        if self.book.is_none() && self.opening.is_none() {
            let path = crate::prefs::get::<String>(cx, BOOK_PATH, String::new());
            if !path.is_empty() {
                let page = crate::prefs::get::<usize>(cx, BOOK_PAGE, 0);
                self.open(PathBuf::from(path), page, window, cx);
            }
        }
        let status = cx
            .global::<AppCore>()
            .request(calls::EngineStatus { variant: None });
        cx.spawn(async move |this, cx| {
            let ready = status
                .await
                .warn_on_err()
                .is_some_and(|status| status.ready);
            this.update(cx, |this, cx| {
                this.engine_ready = ready;
                this.engine_changed(cx);
            })
            .ok();
        })
        .detach();
    }

    fn detach(&mut self, cx: &mut Context<Self>) {
        self.shown = false;
        self.engine.stop(false, cx);
    }

    /// Ask for a PDF and open it at its first page.
    fn choose_book(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.open(path, 0, window, cx))
                .ok();
        })
        .detach();
    }

    /// Open the book at `path` on page `page` (its first, when it has fewer).
    fn open(&mut self, path: PathBuf, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        let opened = cx.background_spawn({
            let path = path.clone();
            async move { book::open(&path) }
        });
        self.opening = Some(cx.spawn_in(window, async move |this, cx| {
            let opened = opened.await;
            this.update_in(cx, |this, window, cx| {
                this.opening = None;
                match opened {
                    Ok(book) => {
                        crate::prefs::keep(cx, BOOK_PATH, path.to_string_lossy());
                        let page = if page < book.page_count() { page } else { 0 };
                        this.book = Some((path, book));
                        this.problem = None;
                        this.study = None;
                        this.then = None;
                        this.go_to(page, window, cx);
                    }
                    Err(problem) => {
                        crate::log::write(
                            kchess_contracts::Level::Info,
                            "books",
                            &format!("a book was not opened: {problem}"),
                        );
                        this.problem = Some(problem.into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The position on the board, and the page shown, for the tests.
    #[cfg(test)]
    pub(crate) fn board_fen(&self, cx: &App) -> String {
        self.board.read(cx).state().fen.clone()
    }

    #[cfg(test)]
    pub(crate) fn page(&self) -> usize {
        self.page
    }

    #[cfg(test)]
    pub(crate) fn problem(&self) -> Option<SharedString> {
        self.problem.clone()
    }

    fn page_count(&self) -> usize {
        self.book.as_ref().map_or(0, |(_, book)| book.page_count())
    }

    /// Show page `page`, drawn off the main thread.
    fn go_to(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, book)) = &self.book else {
            return;
        };
        if page >= book.page_count() {
            return;
        }
        self.page = page;
        crate::prefs::keep(cx, BOOK_PAGE, page);
        let book = book.clone();
        let drawn = cx.background_spawn(async move { book::draw_page(&book, page) });
        self.drawing = Some(cx.spawn_in(window, async move |this, cx| {
            let drawn = drawn.await;
            this.update_in(cx, |this, window, cx| {
                this.drawing = None;
                if drawn.is_none() {
                    this.problem = Some("This page could not be drawn".into());
                }
                if let Some(old) = std::mem::replace(&mut this.drawn, drawn) {
                    cx.drop_image(old.image, Some(window));
                }
                let diagrams = this.drawn.as_ref().map_or(0, |drawn| drawn.diagrams.len());
                match this.then.take() {
                    Some(Then::First) if diagrams > 0 => this.choose(0, window, cx),
                    Some(Then::Last) if diagrams > 0 => this.choose(diagrams - 1, window, cx),
                    // A page of text alone: on through the book to the next diagram, in the
                    // same direction.
                    Some(then) if this.drawn.is_some() => {
                        let next = match then {
                            Then::First => Some(page + 1).filter(|&next| next < this.page_count()),
                            Then::Last => page.checked_sub(1),
                        };
                        if let Some(next) = next {
                            this.then = Some(then);
                            this.go_to(next, window, cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Set up the page's diagram `index` on the board, read off the main thread.
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drawn) = &self.drawn else {
            return;
        };
        let Some(&area) = drawn.diagrams.get(index) else {
            return;
        };
        let pixels = drawn.pixels.clone();
        let page = drawn.index;
        self.reading_diagram = Some((page, index));
        let read = cx.background_spawn(async move { book::read_diagram(&pixels, area) });
        self.reading = Some(cx.spawn_in(window, async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                this.reading = None;
                this.reading_diagram = None;
                match read {
                    Ok(read) => this.set_up(page, index, &read, cx),
                    Err(refusal) => {
                        crate::log::write(
                            kchess_contracts::Level::Info,
                            "books",
                            &format!("a diagram was not read: {refusal}"),
                        );
                        this.problem = Some("That diagram could not be read".into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The board on a diagram's position as read.
    fn set_up(&mut self, page: usize, diagram: usize, read: &Recognised, cx: &mut Context<Self>) {
        let orientation = if read.white_at_bottom {
            Color::White
        } else {
            Color::Black
        };
        let Some(start) = kchess_domain::rules::setup_start(Variant::Standard, &read.fen) else {
            self.problem =
                Some("That position is not one chess allows: check it in the board editor".into());
            self.study = None;
            let mut state = BoardState::shown(&read.fen, Variant::Standard, orientation, None);
            state.circles = read.doubtful.iter().map(|&at| Circle::yellow(at)).collect();
            self.board
                .update(cx, |board, cx| board.set_state(state, cx));
            self.unreadable_fen = Some((read.fen.clone(), orientation, read.doubtful.clone()));
            return;
        };
        self.problem = None;
        self.unreadable_fen = None;
        self.study = Some(Study {
            page,
            diagram,
            start,
            orientation,
            doubtful: read.doubtful.clone(),
            moves: Vec::new(),
        });
        self.changed(cx);
    }

    /// The diagram after or before the one on the board, on this page or the next or last.
    fn step_diagram(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drawn) = &self.drawn else {
            return;
        };
        let current = self
            .study
            .as_ref()
            .filter(|study| study.page == drawn.index)
            .map(|study| study.diagram);
        let count = drawn.diagrams.len();
        let next = match (current, forward) {
            (Some(at), true) if at + 1 < count => Some(at + 1),
            (Some(at), false) if at > 0 => Some(at - 1),
            (None, true) if count > 0 => Some(0),
            (None, false) if count > 0 => Some(count - 1),
            _ => None,
        };
        if let Some(next) = next {
            return self.choose(next, window, cx);
        }
        // On to the next page with diagrams' first, or the last page's last.
        let page = if forward {
            Some(drawn.index + 1).filter(|&page| page < self.page_count())
        } else {
            drawn.index.checked_sub(1)
        };
        if let Some(page) = page {
            self.then = Some(if forward { Then::First } else { Then::Last });
            self.go_to(page, window, cx);
        }
    }

    /// Play `moves` (UCI) on the board's position, as far as they are legal.
    fn play(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let Some(study) = &mut self.study else {
            return;
        };
        for uci in moves {
            let position = study.position().clone();
            let Some(legal) = kchess_domain::rules::parse_uci(uci)
                .and_then(|uci| kchess_domain::rules::legal_move(&position, uci))
            else {
                break;
            };
            let mut after = position;
            let san = kchess_domain::rules::san_and_play(&mut after, legal);
            study.moves.push((uci.clone(), san, after));
        }
        self.changed(cx);
    }

    fn take_back(&mut self, cx: &mut Context<Self>) {
        if let Some(study) = &mut self.study {
            study.moves.pop();
        }
        self.changed(cx);
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        if let Some(study) = &mut self.study {
            study.moves.clear();
        }
        self.changed(cx);
    }

    /// The other side to move in the diagram: books say which in the text, which is not read.
    fn switch_side(&mut self, cx: &mut Context<Self>) {
        let Some(study) = &mut self.study else {
            return;
        };
        let fen: Vec<String> = kchess_domain::rules::make_fen(&study.start)
            .split_whitespace()
            .enumerate()
            .map(|(field, value)| match (field, value) {
                (1, "w") => "b".into(),
                (1, _) => "w".into(),
                // An en passant square belongs to the side that was to move.
                (3, _) => "-".into(),
                _ => value.into(),
            })
            .collect();
        let fen = fen.join(" ");
        match kchess_domain::rules::setup_start(Variant::Standard, &fen) {
            Some(start) => {
                study.start = start;
                study.moves.clear();
                self.problem = None;
            }
            None => {
                self.problem = Some("The side to move cannot be in check from the other".into());
            }
        }
        self.changed(cx);
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        if let Some(study) = &mut self.study {
            study.orientation = !study.orientation;
        }
        self.changed(cx);
    }

    /// The position on the board, in the board editor: to set right what was misread.
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (fen, orientation) = match (&self.study, &self.unreadable_fen) {
            (Some(study), _) => (
                kchess_domain::rules::make_fen(study.position()),
                study.orientation,
            ),
            (None, Some((fen, orientation, _))) => (fen.clone(), *orientation),
            (None, None) => return,
        };
        crate::pages::editor::edit_position(
            Setup {
                variant: Variant::Standard,
                fen,
            },
            orientation,
            window,
            cx,
        );
    }

    /// The position on the board, with the moves played, on the analysis board.
    fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(study) = &self.study else {
            return;
        };
        let fen = kchess_domain::rules::make_fen(study.position());
        kchess_board::analysis::open(
            kchess_board::analysis::Open::Setup {
                setup: Setup {
                    variant: Variant::Standard,
                    fen,
                },
                orientation: study.orientation,
            },
            window,
            cx,
        );
    }

    fn engine_on(&self, cx: &App) -> bool {
        crate::prefs::get(cx, ENGINE_ON, false)
    }

    fn toggle_engine(&mut self, cx: &mut Context<Self>) {
        let on = self.engine_on(cx);
        crate::prefs::set(cx, ENGINE_ON, !on);
        self.engine_changed(cx);
    }

    /// The game on the board has ended, saying how.
    fn ended(&self) -> Option<&'static str> {
        let position = self.study.as_ref()?.position();
        if position.is_checkmate() {
            Some("Checkmate")
        } else if position.is_stalemate() {
            Some("Stalemate")
        } else if position.is_insufficient_material() {
            Some("Draw")
        } else {
            None
        }
    }

    /// Whether Stockfish follows the board now.
    fn engine_active(&self, cx: &App) -> bool {
        self.shown
            && self.engine_ready
            && self.study.is_some()
            && self.engine_on(cx)
            && assistance(cx)
    }

    /// Search the board's position, or nothing, after anything that may change which.
    fn engine_changed(&mut self, cx: &mut Context<Self>) {
        let request = self
            .study
            .as_ref()
            .filter(|_| self.engine_active(cx) && self.ended().is_none())
            .map(|study| AnalysisRequest {
                fen: kchess_domain::rules::make_fen(study.position()),
                lines: LINES,
                infinite: Some(false),
                root_fen: Some(study.start_fen()),
                moves: Some(study.moves.iter().map(|(uci, _, _)| uci.clone()).collect()),
                client_id: None,
                variant: None,
            });
        self.engine.search(request, engine_of, cx);
        self.redraw_board(cx);
        cx.notify();
    }

    fn engine_update(&mut self, update: &AnalysisUpdate, cx: &mut Context<Self>) {
        if self.engine.receive(update) {
            self.redraw_board(cx);
            cx.notify();
        }
    }

    /// The study changed: the board and the engine follow.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.engine_changed(cx);
    }

    /// The board drawn from the study: its position, movable by the side to move, the last move,
    /// the doubtful squares ringed before a move is played, and the engine's arrows.
    fn redraw_board(&mut self, cx: &mut Context<Self>) {
        let Some(study) = &self.study else {
            return;
        };
        let position = study.position();
        let last = study
            .moves
            .last()
            .and_then(|(uci, _, _)| crate::board::last_squares(uci));
        let mut state = BoardState::of(position, study.orientation, Some(position.turn()), last);
        if study.moves.is_empty() {
            state.circles = study
                .doubtful
                .iter()
                .map(|&at| Circle::yellow(at))
                .collect();
        }
        if self.engine_active(cx) {
            state.arrows = self
                .engine
                .lines()
                .iter()
                .enumerate()
                .filter_map(|(rank, line)| {
                    let uci = line.pv.first()?;
                    let square =
                        |range: std::ops::Range<usize>| uci.get(range)?.parse::<Square>().ok();
                    Some(Arrow::engine(square(0..2)?, square(2..4)?, rank))
                })
                .collect();
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
    }
}

/// Each move's SAN, numbered from the position the moves start in.
fn numbered(start: &VariantPosition, moves: &[(String, String, VariantPosition)]) -> String {
    let mut number = start.fullmoves().get();
    let mut turn = start.turn();
    let mut out = Vec::new();
    for (k, (_, san, _)) in moves.iter().enumerate() {
        match turn {
            Color::White => out.push(format!("{number}. {san}")),
            Color::Black if k == 0 => out.push(format!("{number}… {san}")),
            Color::Black => out.push(san.clone()),
        }
        if turn == Color::Black {
            number += 1;
        }
        turn = !turn;
    }
    out.join(" ")
}

/// The first size saved for a panel, else none: the group shares the room evenly.
/// The second of two panels' size: as laid out, or before that as last left, or `otherwise`.
fn kept(laid_out: &[Pixels], saved: &[f32], otherwise: Pixels) -> Pixels {
    laid_out
        .get(1)
        .copied()
        .filter(|size| *size > px(0.))
        .or_else(|| saved.get(1).copied().filter(|size| *size > 0.).map(px))
        .unwrap_or(otherwise)
}

/// A resizable group's sizes, kept to restore.
fn keep_sizes(key: &'static str, state: &Entity<ResizableState>, cx: &mut App) {
    let sizes: Vec<f32> = state
        .read(cx)
        .sizes()
        .iter()
        .map(|size| size.as_f32())
        .collect();
    crate::prefs::keep(cx, key, sizes);
}

impl BooksPage {
    fn render_toolbar(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .book
            .as_ref()
            .and_then(|(path, _)| path.file_stem())
            .map(|name| SharedString::from(name.to_string_lossy().into_owned()));
        let count = self.page_count();
        let has_book = self.book.is_some();
        h_flex()
            .h(TOOLBAR)
            .flex_none()
            .px(px(12.))
            .gap(px(8.))
            .border_b_1()
            .border_color(palette.border)
            .child(
                Button::new("books-open")
                    .small()
                    .outline()
                    .icon(icon("folder-open"))
                    .label("Open book")
                    .on_click(cx.listener(|this, _, window, cx| this.choose_book(window, cx))),
            )
            .children(title.map(|title| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.highlighted)
                    .child(title)
            }))
            .when(!has_book, |bar| bar.child(div().flex_1()))
            .when(has_book, |bar| {
                bar.child(
                    Button::new("books-previous-page")
                        .small()
                        .ghost()
                        .icon(icon("chevron-left"))
                        .accessibility_label("Previous page")
                        .disabled(self.page == 0)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(page) = this.page.checked_sub(1) {
                                this.go_to(page, window, cx);
                            }
                        })),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(format!("Page {} of {count}", self.page + 1)),
                )
                .child(
                    Button::new("books-next-page")
                        .small()
                        .ghost()
                        .icon(icon("chevron-right"))
                        .accessibility_label("Next page")
                        .disabled(self.page + 1 >= count)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let page = this.page + 1;
                            this.go_to(page, window, cx);
                        })),
                )
                .child(div().w(px(1.)).h(px(20.)).bg(palette.border))
                .child(
                    Button::new("books-previous-diagram")
                        .small()
                        .ghost()
                        .icon(icon("skip-back"))
                        .accessibility_label("Previous diagram")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.step_diagram(false, window, cx)),
                        ),
                )
                .child(
                    Button::new("books-next-diagram")
                        .small()
                        .outline()
                        .icon(icon("skip-forward"))
                        .label("Next diagram")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.step_diagram(true, window, cx)),
                        ),
                )
            })
    }

    /// The book's page at `width`, its diagrams outlined to click; or a way to open a book.
    fn render_book(&self, width: Pixels, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(drawn) = &self.drawn else {
            let busy = self.opening.is_some() || self.drawing.is_some();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .p(px(24.))
                .when(busy, |empty| {
                    empty.child(widgets::hint("Opening…", palette))
                })
                .when(!busy, |empty| {
                    empty
                        .child(widgets::empty_state(
                            icon("book-open"),
                            "Open a chess book",
                            None,
                            palette,
                        ))
                        .child(
                            Button::new("books-open-empty")
                                .primary()
                                .icon(icon("folder-open"))
                                .label("Open PDF")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.choose_book(window, cx)),
                                ),
                        )
                })
                .into_any_element();
        };
        let width = (width - px(24.)).max(px(120.));
        let scale = width.as_f32() / drawn.width() as f32;
        let height = px(drawn.height() as f32 * scale);
        let on_board = self
            .study
            .as_ref()
            .filter(|study| study.page == drawn.index)
            .map(|study| study.diagram);
        let reading = self
            .reading_diagram
            .filter(|(page, _)| *page == drawn.index)
            .map(|(_, diagram)| diagram);
        let outline = |index: usize, area: Area| {
            let at = |value: u32| px(value as f32 * scale);
            let chosen = on_board == Some(index);
            div()
                .id(("book-diagram", index))
                .test_support()
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .absolute()
                .left(at(area.x) - px(3.))
                .top(at(area.y) - px(3.))
                .w(at(area.width) + px(6.))
                .h(at(area.height) + px(6.))
                .rounded(radii::SMALL)
                .border_2()
                .border_color(if chosen {
                    palette.primary
                } else {
                    // Seen against the page's paper, which is white whatever the theme.
                    tint(palette.primary, 60.)
                })
                .when(reading == Some(index), |outline| {
                    outline.bg(tint(palette.primary, 15.))
                })
                .cursor_pointer()
                .hover(|style| {
                    style
                        .border_color(palette.primary)
                        .bg(tint(palette.primary, 10.))
                })
                .aria_label(SharedString::from(format!("Diagram {}", index + 1)))
                .on_click(cx.listener(move |this, _, window, cx| this.choose(index, window, cx)))
        };
        div()
            .id("book-page")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .p(px(12.))
            .bg(palette.bg)
            .child(
                div()
                    .relative()
                    .w(width)
                    .h(height)
                    .border_1()
                    .border_color(palette.border)
                    .child(img(drawn.image.clone()).w(width).h(height))
                    .children(
                        drawn
                            .diagrams
                            .iter()
                            .enumerate()
                            .map(|(index, &area)| outline(index, area)),
                    ),
            )
            .into_any_element()
    }

    /// Stockfish's switch, score and lines; the moves played; what to do with the position.
    fn render_panel(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.engine_active(cx);
        let allowed = self.engine_ready && assistance(cx);
        let on = self.engine_on(cx);
        let ended = self.ended();
        let status: SharedString = if !assistance(cx) {
            "Paused during your game".into()
        } else if !self.engine_ready {
            "Engine unavailable".into()
        } else if let Some(error) = &self.engine.error {
            error.clone().into()
        } else if !on {
            "Off".into()
        } else if self.study.is_none() {
            "Set up a diagram".into()
        } else if let Some(ended) = ended {
            ended.into()
        } else {
            match &self.engine.update {
                None => "Starting…".into(),
                Some(update) => format!("Depth {}", update.depth).into(),
            }
        };
        let score = if active && ended.is_none() {
            crate::game::tree::format_eval(self.engine.best())
        } else {
            "—".into()
        };
        let head = h_flex()
            .gap(px(8.))
            .child(
                Switch::new("books-engine")
                    .large()
                    .checked(on && allowed)
                    .disabled(!allowed)
                    .accessibility_label("Engine analysis")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_engine(cx))),
            )
            .child(
                div()
                    .min_w(px(60.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if active { palette.text } else { palette.dimmed })
                    .child(score),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Stockfish"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .truncate()
                            .text_color(palette.text_muted)
                            .child(status),
                    ),
            );
        let page = cx.entity().downgrade();
        let lines = self.study.as_ref().filter(|_| active).map(|study| {
            crate::engine_view::engine_lines(
                crate::engine_view::EngineLines {
                    fen: &kchess_domain::rules::make_fen(study.position()),
                    variant: Variant::Standard,
                    orientation: study.orientation,
                    lines: self.engine.lines(),
                    pending: LINES as usize,
                    ended: ended.map(|_| "Nothing to analyse"),
                },
                std::rc::Rc::new(move |pv: &[String], at, _: &mut Window, cx: &mut App| {
                    let moves = pv[..(at + 1).min(pv.len())].to_vec();
                    page.update(cx, |page, cx| page.play(&moves, cx)).ok();
                }),
                palette,
                crate::notation::figurine(cx),
            )
        });
        let has_position = self.study.is_some();
        // The side to move now; switched only at the diagram, which starts its moves again.
        let turn = self
            .study
            .as_ref()
            .map(|study| {
                study
                    .moves
                    .last()
                    .map_or(study.start.turn(), |(_, _, position)| position.turn())
            })
            .unwrap_or(Color::White);
        let at_diagram = self
            .study
            .as_ref()
            .is_some_and(|study| study.moves.is_empty());
        let moves = self
            .study
            .as_ref()
            .filter(|study| !study.moves.is_empty())
            .map(|study| numbered(&study.start, &study.moves));
        let position_row = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                Button::new("books-side")
                    .small()
                    .outline()
                    .icon(icon("arrow-left-right"))
                    .label(if turn == Color::White {
                        "White to move"
                    } else {
                        "Black to move"
                    })
                    .tooltip("Switch the side to move")
                    .disabled(!at_diagram)
                    .on_click(cx.listener(|this, _, _, cx| this.switch_side(cx))),
            )
            .child(
                Button::new("books-flip")
                    .small()
                    .ghost()
                    .icon(icon("arrow-up-down"))
                    .accessibility_label("Flip board")
                    .disabled(!has_position)
                    .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
            )
            .child(
                Button::new("books-edit")
                    .small()
                    .ghost()
                    .icon(icon("pencil-ruler"))
                    .label("Edit position")
                    .disabled(!has_position && self.unreadable_fen.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
            )
            .child(
                Button::new("books-analyse")
                    .small()
                    .ghost()
                    .icon(icon("microscope"))
                    .label("Analyse")
                    .disabled(!has_position)
                    .on_click(cx.listener(|this, _, window, cx| this.analyse(window, cx))),
            );
        let doubtful = self
            .study
            .as_ref()
            .filter(|study| study.moves.is_empty() && !study.doubtful.is_empty())
            .is_some();
        widgets::side_panel(palette)
            .id("books-panel")
            .size_full()
            .overflow_y_scroll()
            .child(head)
            .children(lines)
            .child(position_row)
            .when(doubtful, |panel| {
                panel.child(widgets::hint("Check the ringed squares", palette))
            })
            .when_some(moves, |panel, moves| {
                panel.child(
                    v_flex()
                        .gap(px(8.))
                        .child(div().text_size(px(14.)).child(moves))
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(
                                    Button::new("books-take-back")
                                        .small()
                                        .outline()
                                        .icon(icon("undo-2"))
                                        .label("Take back")
                                        .on_click(cx.listener(|this, _, _, cx| this.take_back(cx))),
                                )
                                .child(
                                    Button::new("books-restart")
                                        .small()
                                        .ghost()
                                        .icon(icon("rotate-ccw"))
                                        .label("Back to the diagram")
                                        .on_click(cx.listener(|this, _, _, cx| this.restart(cx))),
                                ),
                        ),
                )
            })
            .when_some(self.problem.clone(), |panel, problem| {
                panel.child(
                    div()
                        .id("books-problem")
                        .test_support()
                        .text_size(px(13.))
                        .text_color(palette.error)
                        .child(problem),
                )
            })
            .when(!has_position && self.book.is_some(), |panel| {
                panel.child(widgets::hint("Click a diagram to set it up", palette))
            })
    }
}

impl Render for BooksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        // The page area as the shell measured it, so the first frame is laid out for the window
        // the page opens in; the dividers keep their proportions as the window changes.
        let area = kchess_nav::play_layout::page_area(window).unwrap_or_else(|| {
            let viewport = window.viewport_size();
            size(
                viewport.width,
                viewport.height - crate::shell::TOPBAR_HEIGHT,
            )
        });
        // The side column keeps its width and the engine panel its height (`fixed`), as the
        // dividers left them: the book and the board take what the window adds or loses.
        let side_width = kept(
            self.split.read(cx).sizes(),
            &crate::prefs::get::<Vec<f32>>(cx, SPLIT, Vec::new()),
            (area.width * 0.45).min(area.width - BOOK_MIN),
        )
        .max(SIDE_MIN);
        let book_width = (area.width - side_width).max(BOOK_MIN);
        let body = (area.height - TOOLBAR).max(px(0.));
        let engine_height = kept(
            self.stack.read(cx).sizes(),
            &crate::prefs::get::<Vec<f32>>(cx, STACK, Vec::new()),
            (body * 0.4).min(body - SIDE_MIN),
        )
        .max(ENGINE_MIN);
        let board_height = (body - engine_height).max(SIDE_MIN);
        let board_side = (side_width.min(board_height) - BOARD_MARGIN * 2.).max(px(120.));
        let board = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(board_side)
                    .flex_none()
                    .child(crate::board::cached(&self.board)),
            );
        let book_panel = resizable_panel()
            .size_range(BOOK_MIN..Pixels::MAX)
            .child(self.render_book(book_width, &palette, cx));
        let board_panel = resizable_panel()
            .size_range(SIDE_MIN..Pixels::MAX)
            .child(board);
        let side_panel = resizable_panel()
            .size_range(SIDE_MIN..Pixels::MAX)
            .size(side_width)
            .fixed()
            .child(
                v_resizable("books-stack")
                    .with_state(&self.stack)
                    .on_resize(|state, _, cx| keep_sizes(STACK, state, cx))
                    .child(board_panel)
                    .child(
                        resizable_panel()
                            .size_range(ENGINE_MIN..Pixels::MAX)
                            .size(engine_height)
                            .fixed()
                            .child(
                                div()
                                    .size_full()
                                    .p(px(8.))
                                    .child(self.render_panel(&palette, cx)),
                            ),
                    ),
            );
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .child(self.render_toolbar(&palette, cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("books-split")
                        .with_state(&self.split)
                        .on_resize(|state, _, cx| keep_sizes(SPLIT, state, cx))
                        .child(book_panel)
                        .child(side_panel),
                ),
            )
    }
}

impl crate::pages::PageView for BooksPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Books,
        label: "Books",
        short: "Books",
        icon: "book-open",
        group: Some(crate::pages::NavGroup::Train),
        keywords: "pdf book diagram read study exercises",
    };

    type Action = std::convert::Infallible;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        BooksPage::new(window, cx)
    }

    fn shown(this: &Entity<Self>, _again: bool, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.attach(window, cx));
    }

    fn hidden(this: &Entity<Self>, _window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.detach(cx));
    }
}
