//! Books: a chess book read beside a board. The book's page on the left, as printed; its
//! diagrams outlined on it, and one clicked set up on the board at the right, read from the
//! picture (`kchess_board::book`). Moves played there are checked against the rules, and
//! Stockfish under the board is off until asked for, so the reader works the position out first.
//! The two sides, and the board and the engine, are split by dividers that keep where they were
//! dragged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, ResizableState, Sizable as _, h_flex, h_resizable, resizable_panel, v_flex,
    v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::book::{self, Book, Cover, DrawnPage};
use kchess_board::picture::{Area, Recognised};
use kchess_contracts::calls;
use kchess_contracts::types::{AnalysisRequest, AnalysisUpdate};
use kchess_domain::variant::{Setup, Variant};
use serde::{Deserialize, Serialize};
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

/// The book open now; none once it is closed.
const BOOK_PATH: &str = "kchess:book-path";
/// The page the book open last was on, before each book kept its own (`BOOKS`).
const BOOK_PAGE: &str = "kchess:book-page";
/// The books opened, the latest first, each with where it was left.
const BOOKS: &str = "kchess:books";
const KEPT_BOOKS: usize = 24;
/// The books the Open menu offers besides choosing a file, and the longest name it shows
/// whole: its items do not cut a longer one short, which would run out of the menu.
/// A cover's width as drawn, and its card's.
const COVER_WIDTH: u32 = 360;
const CARD_WIDTH: Pixels = px(176.);
/// Where the dividers were dragged: the book's and the board's sides, and the board and the
/// engine.
const SPLIT: &str = "kchess:book-split";
const STACK: &str = "kchess:book-stack";
/// Whether Stockfish follows the board.
const ENGINE_ON: &str = "kchess:book-engine";
const LINES: u32 = 3;
/// The toolbar's height, and the room the board leaves around itself in its panel.
/// The diagram buttons under the board, and the gap above them: the board is smaller by both.
const DIAGRAM_GAP: Pixels = px(8.);
const DIAGRAM_ROW: Pixels = px(40.);
const TOOLBAR: Pixels = px(48.);
const BOARD_MARGIN: Pixels = px(16.);
/// The smallest each side and the board's panel may be dragged to.
const BOOK_MIN: Pixels = px(240.);
const SIDE_MIN: Pixels = px(280.);
const ENGINE_MIN: Pixels = px(120.);
/// The room around the book's pages, and between them.
const BOOK_PADDING: Pixels = px(12.);
/// Pages drawn either side of the one read: before it, and the further after it, which is where
/// a reader goes.
const DRAWN_BEFORE: usize = 1;
const DRAWN_AFTER: usize = 2;
/// Pages kept drawn further away, so scrolling back and forth does not draw them again.
const KEPT_AROUND: usize = 4;

/// What a click on the next or previous diagram goes to once its page is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    First,
    Last,
}

/// A book opened before, and where it was left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    path: String,
    /// The page read, and how many it has.
    page: usize,
    #[serde(default)]
    pages: usize,
    /// When it was last opened, in milliseconds since 1970.
    #[serde(default)]
    opened: f64,
    /// The diagram last set up on the board: its page and its place there. Left out when there
    /// is none: the core keeps no null in a preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagram: Option<(usize, usize)>,
}

/// The books opened, the latest first. The book of an earlier release, which kept one alone,
/// is the first.
fn listed(cx: &App) -> Vec<Listed> {
    let books = crate::prefs::get::<Vec<Listed>>(cx, BOOKS, Vec::new());
    if !books.is_empty() {
        return books;
    }
    let path = crate::prefs::get::<String>(cx, BOOK_PATH, String::new());
    if path.is_empty() {
        return books;
    }
    vec![Listed {
        path,
        page: crate::prefs::get::<usize>(cx, BOOK_PAGE, 0),
        pages: 0,
        opened: 0.,
        diagram: None,
    }]
}

fn keep_listed(cx: &mut App, mut books: Vec<Listed>) {
    books.truncate(KEPT_BOOKS);
    crate::prefs::keep(cx, BOOKS, books);
}

/// A book's name: its file's, without `.pdf`.
fn title(path: &str) -> SharedString {
    Path::new(path)
        .file_stem()
        .map_or_else(
            || path.to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
        .into()
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
    /// The page read: the one in the middle of the book's side.
    page: usize,
    /// The book's pages, one after another, drawn as they come near.
    list: ListState,
    drawn: HashMap<usize, DrawnPage>,
    drawing: HashMap<usize, Task<()>>,
    /// Opening a book and reading a diagram: each replaced by the next.
    opening: Option<Task<()>>,
    reading: Option<Task<()>>,
    /// The diagram to set up once its page is drawn: that page's first or last, or on through
    /// the book in that direction when it has none.
    then: Option<(usize, Then)>,
    /// The diagram being read, while it is.
    reading_diagram: Option<(usize, usize)>,
    /// The diagram that was on the board when the book was left, set up again once its page
    /// is drawn.
    restore: Option<(usize, usize)>,
    /// The listed books' covers, or what became of their files; drawn one after another.
    covers: HashMap<String, Cover>,
    covering: Option<Task<()>>,
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
        let board = cx.new(|_| ChessBoard::new(BoardState::start()));
        let split = cx.new(|_| ResizableState::default());
        let stack = cx.new(|_| ResizableState::default());
        let list = ListState::new(0, ListAlignment::Top, px(400.));
        let page = cx.entity().downgrade();
        list.set_scroll_handler(move |_, window, cx| {
            // After the scroll is applied: the list is borrowed while it tells of it.
            page.update(cx, |_, cx| {
                cx.defer_in(window, |this, window, cx| this.scrolled(window, cx))
            })
            .ok();
        });
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
            list,
            drawn: HashMap::new(),
            drawing: HashMap::new(),
            opening: None,
            reading: None,
            then: None,
            reading_diagram: None,
            restore: None,
            covers: HashMap::new(),
            covering: None,
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

    /// The page is shown: the book open when it was left reopens where it was, or the books
    /// opened before are listed; and the engine's readiness is asked for.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = true;
        if self.book.is_none() && self.opening.is_none() {
            let path = crate::prefs::get::<String>(cx, BOOK_PATH, String::new());
            if path.is_empty() {
                self.cover_books(window, cx);
            } else {
                self.open_listed(PathBuf::from(path), window, cx);
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

    /// Ask for a PDF and open it: where it was left when it was opened before.
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
            this.update_in(cx, |this, window, cx| this.open_listed(path, window, cx))
                .ok();
        })
        .detach();
    }

    /// Ask where the file of the listed book at `path` went, and open it from there.
    fn locate(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
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
            let Some(found) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let moved = found.to_string_lossy().into_owned();
                let mut books = listed(cx);
                books.retain(|book| book.path != moved);
                if let Some(book) = books.iter_mut().find(|book| book.path == path) {
                    book.path = moved;
                }
                keep_listed(cx, books);
                this.forget_cover(&path, window, cx);
                this.open_listed(found, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Open the book at `path` where it was left, or at its first page.
    fn open_listed(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let key = path.to_string_lossy();
        let left = listed(cx).into_iter().find(|book| book.path == key);
        let (page, diagram) = left.map_or((0, None), |book| (book.page, book.diagram));
        self.open(path, page, diagram, window, cx);
    }

    /// Close the book: the books opened are listed in its place.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::keep(cx, BOOK_PATH, "");
        self.book = None;
        self.page = 0;
        self.list.reset(0);
        self.release(|_| false, window, cx);
        self.opening = None;
        self.reading = None;
        self.reading_diagram = None;
        self.then = None;
        self.restore = None;
        self.study = None;
        self.unreadable_fen = None;
        self.problem = None;
        let start = BoardState::of(
            &VariantPosition::Chess(shakmaty::Chess::default()),
            Color::White,
            None,
            None,
        );
        self.board
            .update(cx, |board, cx| board.set_state(start, cx));
        self.changed(cx);
        self.cover_books(window, cx);
    }

    /// Take the book at `path` off the list.
    fn forget(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut books = listed(cx);
        books.retain(|book| book.path != path);
        keep_listed(cx, books);
        self.forget_cover(path, window, cx);
        cx.notify();
    }

    fn forget_cover(&mut self, path: &str, window: &mut Window, cx: &mut App) {
        if let Some(Cover::Drawn(image)) = self.covers.remove(path) {
            cx.drop_image(image, Some(window));
        }
    }

    /// Draw the covers of the listed books not drawn yet, one after another off the main
    /// thread, while no book is open; and those drawn before whose files went, as gone.
    fn cover_books(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.book.is_some() || self.covering.is_some() {
            return;
        }
        let (drawn, wanted): (Vec<String>, Vec<String>) = listed(cx)
            .into_iter()
            .map(|book| book.path)
            .partition(|path| matches!(self.covers.get(path), Some(Cover::Drawn(_))));
        self.covering = Some(cx.spawn_in(window, async move |this, cx| {
            let gone = cx
                .background_spawn(async move {
                    drawn
                        .into_iter()
                        .filter(|path| !Path::new(path).is_file())
                        .collect::<Vec<_>>()
                })
                .await;
            let kept = this.update_in(cx, |this, window, cx| {
                for path in gone {
                    this.forget_cover(&path, window, cx);
                    this.covers.insert(path, Cover::Missing);
                }
                cx.notify();
            });
            if kept.is_err() {
                return;
            }
            for path in wanted {
                let drawing = path.clone();
                let cover = cx
                    .background_spawn(async move { book::cover(Path::new(&drawing), COVER_WIDTH) })
                    .await;
                let kept = this.update_in(cx, |this, window, cx| {
                    this.forget_cover(&path, window, cx);
                    this.covers.insert(path, cover);
                    cx.notify();
                });
                if kept.is_err() {
                    return;
                }
            }
            this.update_in(cx, |this, window, cx| {
                this.covering = None;
                // Books listed meanwhile.
                let listed_since = listed(cx)
                    .iter()
                    .any(|book| !this.covers.contains_key(&book.path));
                if listed_since {
                    this.cover_books(window, cx);
                }
            })
            .ok();
        }));
    }

    /// The listed book open now changed where it was left.
    fn left_at(&self, cx: &mut App, change: impl FnOnce(&mut Listed)) {
        let Some((path, _)) = &self.book else {
            return;
        };
        let path = path.to_string_lossy();
        let mut books = listed(cx);
        if let Some(book) = books.iter_mut().find(|book| book.path == path) {
            change(book);
            keep_listed(cx, books);
        }
    }

    /// Open the book at `path` on page `page` (its first, when it has fewer), with `diagram` set
    /// up on the board.
    fn open(
        &mut self,
        path: PathBuf,
        page: usize,
        diagram: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                        let key = path.to_string_lossy().into_owned();
                        crate::prefs::keep(cx, BOOK_PATH, &key);
                        let count = book.page_count();
                        let page = if page < count { page } else { 0 };
                        let mut books = listed(cx);
                        let left = books.iter().position(|book| book.path == key);
                        let diagram = left
                            .and_then(|at| books[at].diagram)
                            .or(diagram)
                            .filter(|(page, _)| *page < count);
                        if let Some(at) = left {
                            books.remove(at);
                        }
                        books.insert(
                            0,
                            Listed {
                                path: key.clone(),
                                page,
                                pages: count,
                                opened: kchess_games::now_ms(),
                                diagram,
                            },
                        );
                        keep_listed(cx, books);
                        // A file found again draws its cover anew.
                        if !matches!(this.covers.get(&key), Some(Cover::Drawn(_))) {
                            this.covers.remove(&key);
                        }
                        this.covering = None;
                        this.list.reset(count);
                        this.release(|_| false, window, cx);
                        this.book = Some((path, book));
                        this.problem = None;
                        this.study = None;
                        this.unreadable_fen = None;
                        this.then = None;
                        this.restore = diagram;
                        this.go_to(page, window, cx);
                    }
                    Err(problem) => {
                        crate::log::write(
                            kchess_contracts::Level::Info,
                            "books",
                            &format!("a book was not opened: {problem}"),
                        );
                        this.problem = Some(problem.into());
                        // What became of its file, on its card.
                        if this.book.is_none() {
                            crate::prefs::keep(cx, BOOK_PATH, "");
                            this.forget_cover(&path.to_string_lossy(), window, cx);
                            this.cover_books(window, cx);
                        }
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

    /// Scroll the book to page `page`.
    fn go_to(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if page >= self.page_count() {
            return;
        }
        self.list.scroll_to(ListOffset {
            item_ix: page,
            offset_in_item: px(0.),
        });
        self.read_on(page, window, cx);
        self.arrived(page, window, cx);
    }

    /// The book was scrolled: the page read is the one across the middle of the book's side.
    fn scrolled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.page_count();
        if count == 0 {
            return;
        }
        let top = self.list.logical_scroll_top();
        let middle = top.offset_in_item + self.list.viewport_bounds().size.height / 2.;
        let mut page = top.item_ix.min(count - 1);
        // A page's height from its last layout, which a scroll does not change.
        if let Some(bounds) = self.list.bounds_for_item(page)
            && middle > bounds.size.height
            && page + 1 < count
        {
            page += 1;
        }
        self.read_on(page, window, cx);
    }

    /// Page `page` is the one read: it and those near it drawn, those far from it let go.
    fn read_on(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if page != self.page {
            self.page = page;
            self.left_at(cx, |book| book.page = page);
            cx.notify();
        }
        let last = self.page_count().saturating_sub(1);
        let wanted = page.saturating_sub(DRAWN_BEFORE)..=(page + DRAWN_AFTER).min(last);
        // The pages a diagram waits for.
        let targets: Vec<usize> = self
            .then
            .map(|(target, _)| target)
            .into_iter()
            .chain(self.restore.map(|(target, _)| target))
            .collect();
        let kept = page.saturating_sub(KEPT_AROUND)..=page + KEPT_AROUND;
        self.release(|at| kept.contains(&at) || targets.contains(&at), window, cx);
        for at in wanted.chain(targets) {
            self.draw(at, window, cx);
        }
    }

    /// Let go of the drawn pages, and those being drawn, that `keep` does not want.
    fn release(&mut self, keep: impl Fn(usize) -> bool, window: &mut Window, cx: &mut App) {
        self.drawing.retain(|&at, _| keep(at));
        let gone: Vec<usize> = self.drawn.keys().copied().filter(|&at| !keep(at)).collect();
        for at in gone {
            if let Some(page) = self.drawn.remove(&at) {
                cx.drop_image(page.image, Some(window));
            }
        }
    }

    /// Draw page `page` off the main thread, unless it is drawn or being drawn.
    fn draw(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.drawn.contains_key(&page) || self.drawing.contains_key(&page) {
            return;
        }
        let Some((_, book)) = &self.book else {
            return;
        };
        let book = book.clone();
        let drawn = cx.background_spawn(async move { book::draw_page(&book, page) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let drawn = drawn.await;
            this.update_in(cx, |this, window, cx| {
                this.drawing.remove(&page);
                match drawn {
                    Some(drawn) => {
                        if let Some(old) = this.drawn.insert(page, drawn) {
                            cx.drop_image(old.image, Some(window));
                        }
                        this.arrived(page, window, cx);
                    }
                    None => {
                        this.problem = Some(format!("Page {} could not be drawn", page + 1).into());
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.drawing.insert(page, task);
    }

    /// Page `page` is drawn: the diagram waiting for it is set up, or, on a page of text alone,
    /// the book is followed on to the next diagram.
    fn arrived(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((at, index)) = self.restore
            && at == page
            && let Some(drawn) = self.drawn.get(&page)
        {
            self.restore = None;
            if index < drawn.diagrams.len() {
                self.choose(page, index, window, cx);
            }
        }
        let Some((target, then)) = self.then else {
            return;
        };
        let Some(drawn) = self.drawn.get(&page).filter(|_| target == page) else {
            return;
        };
        self.then = None;
        let count = drawn.diagrams.len();
        if count > 0 {
            let index = match then {
                Then::First => 0,
                Then::Last => count - 1,
            };
            self.reveal(page, index);
            return self.choose(page, index, window, cx);
        }
        let next = match then {
            Then::First => Some(page + 1).filter(|&next| next < self.page_count()),
            Then::Last => page.checked_sub(1),
        };
        if let Some(next) = next {
            self.then = Some((next, then));
            self.go_to(next, window, cx);
        }
    }

    /// Scroll diagram `index` of page `page` into view, when it is not.
    fn reveal(&self, page: usize, index: usize) {
        let Some(drawn) = self.drawn.get(&page) else {
            return;
        };
        let Some(area) = drawn.diagrams.get(index) else {
            return;
        };
        let viewport = self.list.viewport_bounds();
        let width = viewport.size.width - BOOK_PADDING * 2.;
        if width <= px(0.) {
            return;
        }
        let scale = width / px(drawn.width.max(1) as f32);
        let top = px(area.y as f32) * scale;
        let bottom = px((area.y + area.height) as f32) * scale;
        if let Some(item) = self.list.bounds_for_item(page)
            && item.top() + top >= viewport.top()
            && item.top() + bottom <= viewport.bottom()
        {
            return;
        }
        self.list.scroll_to(ListOffset {
            item_ix: page,
            offset_in_item: (top - BOOK_PADDING * 2.).max(px(0.)),
        });
    }

    /// Set up diagram `index` of page `page` on the board, read off the main thread.
    fn choose(&mut self, page: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, book)) = &self.book else {
            return;
        };
        let Some(drawn) = self.drawn.get(&page) else {
            return;
        };
        let Some(&area) = drawn.diagrams.get(index) else {
            return;
        };
        let (book, drawn) = (book.clone(), drawn.clone());
        self.restore = None;
        self.reading_diagram = Some((page, index));
        let read = cx.background_spawn(async move { book::read_diagram(&book, &drawn, area) });
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
        self.left_at(cx, |book| book.diagram = Some((page, diagram)));
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

    /// The diagram after or before the one on the board, on this page or the next or last: from
    /// the page read, or from the diagram on the board while its page is in view.
    fn step_diagram(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = self.list.viewport_bounds();
        let on_board = self
            .study
            .as_ref()
            .map(|study| (study.page, study.diagram))
            .filter(|&(page, _)| {
                self.list
                    .bounds_for_item(page)
                    .is_some_and(|item| item.intersects(&viewport))
            });
        let page = on_board.map_or(self.page, |(page, _)| page);
        let then = if forward { Then::First } else { Then::Last };
        let Some(drawn) = self.drawn.get(&page) else {
            // Not drawn yet: its first or last diagram once it is.
            self.then = Some((page, then));
            return self.read_on(page, window, cx);
        };
        let count = drawn.diagrams.len();
        let next = match (on_board.map(|(_, diagram)| diagram), forward) {
            (Some(at), true) if at + 1 < count => Some(at + 1),
            (Some(at), false) if at > 0 => Some(at - 1),
            (None, true) if count > 0 => Some(0),
            (None, false) if count > 0 => Some(count - 1),
            _ => None,
        };
        if let Some(next) = next {
            self.reveal(page, next);
            return self.choose(page, next, window, cx);
        }
        // On to the next page with diagrams' first, or the last page's last.
        let page = if forward {
            Some(page + 1).filter(|&page| page < self.page_count())
        } else {
            page.checked_sub(1)
        };
        if let Some(page) = page {
            self.then = Some((page, then));
            self.go_to(page, window, cx);
        }
    }

    /// Play `moves` (UCI) on the board's position, as far as they are legal.
    fn play(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let Some(study) = &mut self.study else {
            // No diagram set up: the board played the move at once, and it goes back.
            self.board.update(cx, |board, cx| board.undo_move(cx));
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

    /// Back to the position after the `ply`th move from the diagram, the moves after it taken back.
    fn back_to(&mut self, ply: usize, cx: &mut Context<Self>) {
        let Some(study) = &mut self.study else {
            return;
        };
        if ply >= study.moves.len() {
            return;
        }
        study.moves.truncate(ply);
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

    /// The position on the board, for the board editor: to set right what was misread.
    fn to_edit(&self) -> Option<(Setup, Color)> {
        let (fen, orientation) = match (&self.study, &self.unreadable_fen) {
            (Some(study), _) => (
                kchess_domain::rules::make_fen(study.position()),
                study.orientation,
            ),
            (None, Some((fen, orientation, _))) => (fen.clone(), *orientation),
            (None, None) => return None,
        };
        let setup = Setup {
            variant: Variant::Standard,
            fen,
        };
        Some((setup, orientation))
    }

    /// The position on the board, with the moves played, for the analysis board.
    fn to_analyse(&self) -> Option<(Setup, Color)> {
        let study = self.study.as_ref()?;
        let setup = Setup {
            variant: Variant::Standard,
            fen: kchess_domain::rules::make_fen(study.position()),
        };
        Some((setup, study.orientation))
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

/// Where the moves from a diagram start, to number them as the book does.
fn first_move(start: &VariantPosition) -> widgets::FirstMove {
    widgets::FirstMove {
        number: usize::try_from(start.fullmoves().get()).unwrap_or(1),
        black: start.turn() == Color::Black,
    }
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

/// The outline of diagram `index` on page `page`.
pub(crate) fn diagram_id(page: usize, index: usize) -> ElementId {
    ElementId::NamedInteger("book-diagram".into(), (page * 1000 + index) as u64)
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
        h_flex()
            .h(TOOLBAR)
            .flex_none()
            .px(px(12.))
            .gap(px(8.))
            .border_b_1()
            .border_color(palette.border)
            // Back to the shelf, which opens another book from its last card.
            .child(
                Button::new("books-close")
                    .small()
                    .ghost()
                    .icon(icon("arrow-left"))
                    .label("Your books")
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .children(title.map(|title| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.highlighted)
                    .child(title)
            }))
    }

    /// The page shown and the ways to the next and the one before, floating at the foot of the
    /// pages they turn, as a PDF reader's are.
    fn render_pager(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.page_count();
        h_flex()
            .absolute()
            .bottom(px(16.))
            .left_0()
            .right_0()
            .justify_center()
            .child(
                h_flex()
                    .id("books-pager")
                    .test_support()
                    .occlude()
                    .gap(px(4.))
                    .p(px(4.))
                    .rounded(radii::BOX)
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.elevated)
                    .shadow_md()
                    .child(
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
                            .px(px(4.))
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
                    ),
            )
    }

    /// The book's pages one after another, its diagrams outlined to click; or the books opened
    /// before and a way to open another.
    fn render_book(&self, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.book.is_none() {
            return self.render_shelf(palette, cx);
        }
        let this = cx.entity().downgrade();
        div()
            .id("book-pages")
            .test_support()
            .relative()
            .size_full()
            .bg(palette.bg)
            .child(
                list(self.list.clone(), move |page, _, cx| {
                    this.upgrade().map_or_else(
                        || div().into_any_element(),
                        |view| view.read(cx).render_page(page, this.clone(), cx),
                    )
                })
                .size_full()
                .p(BOOK_PADDING),
            )
            .child(self.render_pager(palette, cx))
            .into_any_element()
    }

    /// The books opened before, each a card to go on with; or, before any, a way to open one.
    fn render_shelf(&self, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        // Before any book is listed, the page's one Open book; then the shelf's last card.
        let open = Button::new("books-open-empty")
            .primary()
            .icon(icon("folder-open"))
            .label("Open book")
            .on_click(cx.listener(|this, _, window, cx| this.choose_book(window, cx)));
        if self.opening.is_some() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(widgets::hint("Opening…", palette))
                .into_any_element();
        }
        let books = listed(cx);
        if books.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .p(px(24.))
                .child(widgets::empty_state(
                    icon("book-open"),
                    "Open a chess book",
                    None,
                    palette,
                ))
                .child(open)
                .into_any_element();
        }
        let now = kchess_games::now_ms();
        let add = widgets::add_card(
            "books-add",
            "Open book",
            CARD_WIDTH,
            palette,
            cx.listener(|this, _, window, cx| this.choose_book(window, cx)),
        );
        let cards: Vec<AnyElement> = books
            .into_iter()
            .enumerate()
            .map(|(index, listed)| self.render_card(index, listed, now, cx))
            .collect();
        v_flex()
            .id("books-shelf")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .gap(px(16.))
            .child(widgets::heading("Your books", palette))
            .child(
                // Stretched, so the last card is as tall as the books beside it.
                h_flex()
                    .flex_wrap()
                    .items_stretch()
                    .gap(px(12.))
                    .children(cards)
                    .child(add),
            )
            .into_any_element()
    }

    /// A book opened before: its cover, where it was left, and the way back to it, or to its
    /// file when that moved.
    fn render_card(
        &self,
        index: usize,
        listed: Listed,
        now: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = title(&listed.path);
        let cover = self.covers.get(&listed.path);
        let lost = matches!(cover, Some(Cover::Missing | Cover::Unreadable));
        let location = match cover {
            Some(Cover::Missing) => "File not found".to_string(),
            Some(Cover::Unreadable) => "Could not be read".to_string(),
            _ if listed.pages > 0 => format!("Page {} of {}", listed.page + 1, listed.pages),
            _ => format!("Page {}", listed.page + 1),
        };
        let page = cx.entity().downgrade();
        let path = listed.path.clone();
        let action = if lost {
            let (page, path) = (page.clone(), path.clone());
            Button::new(("books-locate", index))
                .outline()
                .small()
                .w_full()
                .icon(icon("search"))
                .label("Locate…")
                .accessibility_label(format!("Locate {name}"))
                .on_click(move |_, window, cx| {
                    page.update(cx, |page, cx| page.locate(path.clone(), window, cx))
                        .ok();
                })
        } else {
            let (page, path) = (page.clone(), path.clone());
            Button::new(("books-continue", index))
                .outline()
                .small()
                .w_full()
                .icon(icon("book-open"))
                .label("Continue")
                .accessibility_label(format!("Continue {name}"))
                .on_click(move |_, window, cx| {
                    page.update(cx, |page, cx| {
                        page.open_listed(PathBuf::from(&path), window, cx)
                    })
                    .ok();
                })
        };
        let menu = {
            let (page, path) = (page.clone(), path.clone());
            Button::new(("books-card-menu", index))
                .ghost()
                .small()
                .icon(icon("ellipsis-vertical"))
                .accessibility_label(format!("Actions for {name}"))
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let (page, path) = (page.clone(), path.clone());
                    menu.item(
                        PopupMenuItem::new("Remove from list")
                            .icon(icon("x"))
                            .on_click(move |_, window, cx| {
                                page.update(cx, |page, cx| page.forget(&path, window, cx))
                                    .ok();
                            }),
                    )
                })
        };
        widgets::StudyCard {
            id: format!("book-{index}"),
            name: name.to_string(),
            picture: match cover {
                Some(Cover::Drawn(image)) => Some(widgets::CardPicture::Image(image.clone())),
                _ => None,
            },
            location,
            description: (listed.opened > 0.)
                .then(|| format!("Opened {}", widgets::time_ago(listed.opened, now))),
            edited: None,
            rows: 2,
            menu: Some(menu.into_any_element()),
            action: action.into_any_element(),
            width: CARD_WIDTH,
            on_open: std::rc::Rc::new(move |window, cx| {
                page.update(cx, |page, cx| {
                    page.open_listed(PathBuf::from(&path), window, cx)
                })
                .ok();
            }),
            found: Vec::new(),
        }
        .into_any_element()
    }

    /// Page `page` as printed, its own shape before it is drawn; its diagrams outlined.
    fn render_page(&self, page: usize, this: WeakEntity<Self>, cx: &App) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let (width, height) = self
            .book
            .as_ref()
            .and_then(|(_, book)| book.page_size(page))
            .filter(|(width, height)| *width > 0. && *height > 0.)
            .unwrap_or((1., 1.4));
        let on_board = self
            .study
            .as_ref()
            .filter(|study| study.page == page)
            .map(|study| study.diagram);
        let reading = self
            .reading_diagram
            .filter(|(at, _)| *at == page)
            .map(|(_, diagram)| diagram);
        let drawn = self.drawn.get(&page);
        let outline = |drawn: &DrawnPage, index: usize, area: Area| {
            let part = |value: u32, of: u32| relative(value as f32 / of.max(1) as f32);
            let chosen = on_board == Some(index);
            let this = this.clone();
            // The diagram's place on the page, and its outline a little outside it.
            div()
                .absolute()
                .left(part(area.x, drawn.width))
                .top(part(area.y, drawn.height))
                .w(part(area.width, drawn.width))
                .h(part(area.height, drawn.height))
                .child(
                    div()
                        .id(diagram_id(page, index))
                        .test_support()
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .absolute()
                        .inset(px(-3.))
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
                        .aria_label(SharedString::from(format!(
                            "Diagram {} on page {}",
                            index + 1,
                            page + 1
                        )))
                        .on_click(move |_, window, cx| {
                            this.update(cx, |this, cx| this.choose(page, index, window, cx))
                                .ok();
                        }),
                )
        };
        div()
            .pb(BOOK_PADDING)
            .child(
                div()
                    .id(("book-page", page))
                    .test_support()
                    .relative()
                    .w_full()
                    .aspect_ratio(width / height)
                    .bg(white())
                    .border_1()
                    .border_color(palette.border)
                    .when_some(drawn, |sheet, drawn| {
                        sheet.child(img(drawn.image.clone()).size_full()).children(
                            drawn
                                .diagrams
                                .iter()
                                .enumerate()
                                .map(|(index, &area)| outline(drawn, index, area)),
                        )
                    }),
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
        let head = crate::engine_view::engine_head(
            crate::engine_view::EngineHead {
                switch: Switch::new("books-engine")
                    .large()
                    .checked(on && allowed)
                    .disabled(!allowed)
                    .accessibility_label("Engine analysis")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_engine(cx)))
                    .into_any_element(),
                score: score.into(),
                live: active,
                busy: active && ended.is_none() && self.engine.update.is_none(),
                status,
                failed: self.engine.error.is_some(),
                after_status: None,
                actions: None,
            },
            palette,
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
        // The moves played from the diagram, as the game pages list them; a click goes back to one.
        let moves = self
            .study
            .as_ref()
            .filter(|study| !study.moves.is_empty())
            .map(|study| {
                let sans: Vec<String> = study.moves.iter().map(|(_, san, _)| san.clone()).collect();
                let page = cx.entity().downgrade();
                widgets::moves_list(
                    "books-moves",
                    widgets::MoveSheet {
                        sans: &sans,
                        shown: sans.len(),
                        first: first_move(&study.start),
                    },
                    SharedString::new_static(""),
                    move |ply, _, cx| {
                        page.update(cx, |page, cx| page.back_to(ply, cx)).ok();
                    },
                    crate::notation::figurine(cx),
                    palette,
                )
            });
        // Leaving for another page takes what it needs first: the switch hides this page,
        // which stops its engine, and must not happen inside an update of this page.
        let page = cx.entity().downgrade();
        let leave = |take: fn(&Self) -> Option<(Setup, Color)>,
                     run: fn((Setup, Color), &mut Window, &mut App)| {
            let page = page.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let taken = page.upgrade().and_then(|page| take(page.read(cx)));
                if let Some(taken) = taken {
                    run(taken, window, cx);
                }
            }
        };
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
                    .on_click(leave(Self::to_edit, |(setup, orientation), window, cx| {
                        crate::pages::editor::edit_position(setup, orientation, window, cx)
                    })),
            )
            .child(
                Button::new("books-analyse")
                    .small()
                    .ghost()
                    .icon(icon("microscope"))
                    .label("Analyse")
                    .disabled(!has_position)
                    .on_click(leave(
                        Self::to_analyse,
                        |(setup, orientation), window, cx| {
                            kchess_board::analysis::open(
                                kchess_board::analysis::Open::Setup { setup, orientation },
                                window,
                                cx,
                            )
                        },
                    )),
            );
        let doubtful = self
            .study
            .as_ref()
            .filter(|study| study.moves.is_empty() && !study.doubtful.is_empty())
            .is_some();
        // Flat in the column the dividers already frame: a card there was a box in a box.
        v_flex()
            .id("books-panel")
            .size_full()
            .min_w_0()
            .p(px(16.))
            .gap(px(12.))
            .overflow_y_scroll()
            // The position first, as it is the board's; then the engine on it.
            .child(position_row)
            .child(head)
            .children(lines)
            .when(doubtful, |panel| {
                panel.child(widgets::hint("Check the ringed squares", palette))
            })
            .when_some(moves, |panel, moves| {
                panel.child(
                    v_flex()
                        .gap(px(8.))
                        .child(div().w_full().child(moves))
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
        let body = (area.height - TOOLBAR).max(px(0.));
        let engine_height = kept(
            self.stack.read(cx).sizes(),
            &crate::prefs::get::<Vec<f32>>(cx, STACK, Vec::new()),
            (body * 0.4).min(body - SIDE_MIN),
        )
        .max(ENGINE_MIN);
        let board_height = (body - engine_height).max(SIDE_MIN);
        // Under the board, the way through the book's diagrams: what it sets up is the board's.
        let board_side =
            (side_width.min(board_height - DIAGRAM_ROW) - BOARD_MARGIN * 2.).max(px(120.));
        let board = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(DIAGRAM_GAP)
            .child(
                div()
                    .w(board_side)
                    .flex_none()
                    .child(crate::board::cached(&self.board)),
            )
            .child(
                h_flex()
                    .w(board_side)
                    .h(DIAGRAM_ROW - DIAGRAM_GAP)
                    .justify_center()
                    .gap(px(8.))
                    // Held open without a book, so the board keeps its size.
                    .when(self.book.is_some(), |row| {
                        row.child(
                            Button::new("books-previous-diagram")
                                .outline()
                                .icon(icon("skip-back"))
                                .label("Previous")
                                .accessibility_label("Previous diagram")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.step_diagram(false, window, cx)
                                })),
                        )
                        .child(
                            Button::new("books-next-diagram")
                                .outline()
                                .icon(icon("skip-forward"))
                                // gpui-lints: allow(icon_per_label) jumps to the next diagram, past pages
                                .label("Next")
                                .accessibility_label("Next diagram")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.step_diagram(true, window, cx)
                                })),
                        )
                    }),
            );
        let book_panel = resizable_panel()
            .size_range(BOOK_MIN..Pixels::MAX)
            .child(self.render_book(&palette, cx));
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
                            .child(div().size_full().child(self.render_panel(&palette, cx))),
                    ),
            );
        // A book's own bar; the shelf opens another from its last card.
        let toolbar = self
            .book
            .is_some()
            .then(|| self.render_toolbar(&palette, cx));
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .children(toolbar)
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

#[cfg(test)]
mod tests {
    use super::Listed;

    /// A book with no diagram set up yet is kept with no `diagram` at all: the core refuses a
    /// preference holding null, which once lost the whole list.
    #[test]
    fn a_book_without_a_diagram_is_kept_without_null() {
        let book = Listed {
            path: "/books/Tactics.pdf".into(),
            page: 3,
            pages: 120,
            opened: 1.,
            diagram: None,
        };
        let kept = serde_json::to_value(vec![book.clone()]).expect("serialised");
        assert_eq!(
            kept,
            serde_json::json!([{"path": "/books/Tactics.pdf", "page": 3, "pages": 120, "opened": 1.0}])
        );
        assert_eq!(
            serde_json::from_value::<Vec<Listed>>(kept).expect("read back"),
            vec![book]
        );
    }
}
