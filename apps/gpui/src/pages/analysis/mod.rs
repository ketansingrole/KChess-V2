//! The analysis board: a move tree explored on the
//! board, Stockfish's lines and arrows for the position shown, Lichess's cloud evaluation, and the
//! session the Electron app keeps (`sessions.analysis`), so either app opens it as it was left.

mod notes;
mod practice;
mod review;

pub use review::classification_look;

use crate::voice::moves::{Access, Action, BoardBar, Reply, Turn, VoiceMoves, board_bar};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::Duration;

use futures::FutureExt as _;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position as _, Square};

use crate::assets::icon;
use crate::board::{Arrow, Badge, BoardEvent, BoardState, ChessBoard, Circle};
use crate::core::{AppCore, BoxedSave, CoreEvent};
use crate::engine_view::EVAL_BAR;
use crate::explorer::{ExplorerEvent, PositionExplorer};
use crate::export::{Exporter, GameExport};
use crate::game::tree::{
    TreeNode, analysis_context, child_path, comment_text, format_eval, move_glyph, move_number,
    moves_of, path_of, pv_san,
};
use crate::game::{STANDARD_FEN, rules, view};
use crate::online::OnlineHandle;
use crate::pages::study_bar::{STUDY_BAR, StudyBar};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::reviews::ReviewHandle;
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::studies::StudyHandle;
use crate::theme::{Palette, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

/// Why the board left its study: another window changed the chapter it was saving to.
const STALE_COPY: &str = "This study was changed in another KChess window. Your board is kept as an unsaved copy; save it under a new name or reopen the study.";
const CONFLICT: &str = "Recovered session differs from the saved study. It is kept as an unsaved copy; save it under a new name or reopen the saved study.";

/// A study write's answer, which the quit and the page's own handling both wait on.
type StudySave =
    futures::future::Shared<std::pin::Pin<Box<dyn Future<Output = Result<Value, String>>>>>;

/// Evaluations kept per position, so stepping back and forth shows them at once.
const CACHE_SIZE: usize = 600;
const SAVE_DELAY: Duration = Duration::from_millis(150);
/// Stepping quickly through a game should not send every position on the way to Lichess.
const CLOUD_DELAY: Duration = Duration::from_millis(300);
/// The evaluation bar's gap to the board; without the bar the board keeps its place.
const EVAL_GAP: Pixels = px(8.);
const ENGINE_ON: &str = "kchess:analysis-engine";
const ENGINE_LINES: &str = "kchess:analysis-lines";
const INFINITE: &str = "kchess:analysis-infinite";
const ARROWS: &str = "kchess:analysis-arrows";
const TOOL: &str = "kchess:analysis-tool";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Moves,
    Review,
    Explorer,
}

impl Tool {
    fn key(self) -> &'static str {
        match self {
            Tool::Moves => "moves",
            Tool::Review => "review",
            Tool::Explorer => "explorer",
        }
    }

    fn of(key: &str) -> Tool {
        match key {
            "review" => Tool::Review,
            "explorer" => Tool::Explorer,
            _ => Tool::Moves,
        }
    }
}

fn now_square(uci: &str, range: std::ops::Range<usize>) -> Option<Square> {
    uci.get(range)?.parse().ok()
}

fn position_of(fen: &str) -> Option<VariantPosition> {
    kchess_domain::rules::setup_start("standard", fen)
}

/// Checkmate, stalemate or bare kings end the analysis: there is nothing to search.
fn game_over(position: Option<&VariantPosition>) -> Option<&'static str> {
    let position = position?;
    if position.is_checkmate() {
        Some("Checkmate")
    } else if position.is_stalemate() {
        Some("Stalemate")
    } else if position.is_insufficient_material() {
        Some("Insufficient material")
    } else {
        None
    }
}

pub struct AnalysisPage {
    root: TreeNode,
    path: String,
    orientation: Color,
    /// The saved study the board is showing, and its chapter; edits are saved back to it.
    study: String,
    chapter: String,
    /// The PGN last written to the open study, so opening one does not count as an edit.
    study_pgn: String,
    /// Why the study could not be saved (or a recovered session that differs from it).
    study_error: Option<String>,
    study_save: Option<Task<()>>,
    /// Bumped whenever another document takes the board, so a late save-as does not link it.
    navigation: u64,
    study_bar: Entity<StudyBar>,
    /// The main line's game review, and practising its mistakes.
    review: review::ReviewState,
    practice: Option<practice::Practice>,
    /// The engine made way for a review of this game.
    paused_for_review: bool,
    /// The comment and annotation of the move shown.
    notes: notes::Notes,
    /// The move list's scrolling, brought to the move shown after it changes.
    moves_scroll: ScrollHandle,
    reveal_move: std::rc::Rc<std::cell::Cell<bool>>,
    /// The Explorer tool, and whether it was shown at the last render.
    explorer: Entity<PositionExplorer>,
    explorer_open: bool,
    exporter: Entity<Exporter>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    lines_choice: Entity<SelectState<Vec<SharedString>>>,
    import: Entity<TextareaState>,
    import_error: Option<SharedString>,
    /// Shown while the page is open: the engine follows the board.
    attached: bool,
    engine_ready: bool,
    engine_identity: Option<String>,
    evaluations: HashMap<String, Value>,
    /// The cache's keys, oldest first.
    order: VecDeque<String>,
    live: Option<Value>,
    engine_error: Option<String>,
    engine_busy: bool,
    /// The position the running search is for, and the request's ids.
    wanted: String,
    client_id: u64,
    latest_id: f64,
    cloud: Option<Value>,
    cloud_busy: bool,
    cloud_error: Option<String>,
    cloud_request: u64,
    cloud_task: Option<Task<()>>,
    save: Option<Task<()>>,
    wheel: f32,
    focus: FocusHandle,
    /// Spoken moves, and “back”, “next”, “best move”, “flip board”.
    voice: VoiceMoves,
    /// Moves tried instead of a studied puzzle's solution, waiting for their reviews to be
    /// labelled (`usePuzzleAnalysis().label`).
    tried_labels: Vec<Subscription>,
    /// The tried moves already labelled, by path.
    tried_done: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AnalysisPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AnalysisPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> AnalysisPage {
        let library = cx
            .global::<AppCore>()
            .bridge
            .call_now("library", Vec::new())
            .unwrap_or(Value::Null);
        let session = library["sessions"]["analysis"].clone();
        let mistakes = library["mistakes"].as_array().cloned().unwrap_or_default();
        let root = session["pgn"]
            .as_str()
            .and_then(TreeNode::from_pgn)
            .unwrap_or_else(|| TreeNode::new(None));
        let path = root.existing_path(session["path"].as_str().unwrap_or_default());
        let orientation = if session["orientation"] == "black" {
            Color::Black
        } else {
            Color::White
        };
        // A recovered session that differs from its study stays an unsaved copy; it never
        // overwrites the library.
        let library = cx.global::<StudyHandle>().0.clone();
        let linked = library
            .read(cx)
            .study(session["study"].as_str().unwrap_or_default())
            .cloned();
        let chapter_of = |study: &Value, id: &str| {
            study["chapters"]
                .as_array()
                .and_then(|chapters| chapters.iter().find(|chapter| chapter["id"] == json!(id)))
                .cloned()
        };
        let linked_pgn = linked.as_ref().map(|study| {
            chapter_of(study, session["chapter"].as_str().unwrap_or_default())
                .and_then(|chapter| chapter["pgn"].as_str().map(str::to_string))
                .unwrap_or_else(|| study["pgn"].as_str().unwrap_or_default().to_string())
        });
        let conflict = session["pgn"].is_string()
            && linked_pgn
                .as_deref()
                .and_then(TreeNode::from_pgn)
                .is_some_and(|tree| tree.to_pgn() != root.to_pgn());
        let study = if conflict || linked.is_none() {
            String::new()
        } else {
            session["study"].as_str().unwrap_or_default().to_string()
        };
        let chapter = session["chapter"]
            .as_str()
            .filter(|chapter| !chapter.is_empty())
            .map(str::to_string)
            .or_else(|| {
                linked
                    .as_ref()
                    .and_then(|study| study["chapters"][0]["id"].as_str().map(str::to_string))
            })
            .unwrap_or_default();
        let study_pgn = if study.is_empty() {
            String::new()
        } else {
            linked_pgn.unwrap_or_default()
        };
        if !study.is_empty() {
            library.read(cx).track(&study, &chapter);
        }
        let study_error = conflict.then(|| CONFLICT.to_string());
        let page = cx.weak_entity();
        let study_bar = cx.new(|cx| StudyBar::new(page, window, cx));
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &position_of(STANDARD_FEN).unwrap_or_default(),
                orientation,
                None,
                None,
            ))
        });
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
        let lines = prefs_lines(cx);
        let lines_choice = cx.new(|cx| {
            SelectState::new(
                (1..=5)
                    .map(|count| {
                        SharedString::from(format!(
                            "{count} {}",
                            if count == 1 { "line" } else { "lines" }
                        ))
                    })
                    .collect(),
                Some(IndexPath::new(lines - 1)),
                window,
                cx,
            )
        });
        let import = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
        });
        let settings = cx.global::<SettingsHandle>().0.clone();
        let online = cx.global::<OnlineHandle>().0.clone();
        let events = cx.global::<AppCore>().events.clone();
        let notes_field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(2)
                .placeholder("What's the idea here?")
        });
        let explorer = cx.new(|cx| PositionExplorer::new(false, window, cx));
        let subscriptions = vec![
            cx.subscribe(&explorer, |this, _, event: &ExplorerEvent, cx| {
                let ExplorerEvent::Play(uci) = event;
                this.play(uci, cx);
            }),
            cx.subscribe(
                &notes_field,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.write_comment(cx),
                    InputEvent::Focus => this.notes.editing = true,
                    InputEvent::Blur => {
                        this.notes.editing = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, cx);
                }
            }),
            cx.subscribe(
                &crate::voice::handle(cx),
                |this, _, event: &crate::voice::VoiceEvent, cx| this.voice_heard(event, cx),
            ),
            cx.observe(&crate::voice::handle(cx), |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
            cx.subscribe(
                &lines_choice,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(2, |index| index.row);
                    crate::prefs::set(cx, ENGINE_LINES, row + 1);
                    this.engine_changed(cx);
                },
            ),
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.ask_cloud(cx);
                this.redraw(cx);
            }),
            cx.observe(&online, |this, _, cx| this.engine_changed(cx)),
            // Reviews streaming in, and the review queue: labels follow, and the engine makes way
            // while this game is reviewed.
            cx.observe(&cx.global::<ReviewHandle>().0.clone(), |this, _, cx| {
                this.follow_review(cx);
                this.check_practice(cx);
                let reviewing = this.reviewing(cx).is_some();
                if reviewing != this.paused_for_review {
                    this.paused_for_review = reviewing;
                    this.analyse(cx);
                }
                this.redraw(cx);
            }),
            // A study removed from the library (here or on the Studies page) gets no more edits.
            cx.observe(&library, |this, library, cx| {
                if !this.study.is_empty() && library.read(cx).study(&this.study).is_none() {
                    this.study.clear();
                    this.schedule_save(cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
                if event.name == "engine:analysis" {
                    this.receive(event.payload.clone(), cx);
                }
                // A search on a lost connection ended with it: the board's is started again.
                if event.name == crate::core::RECONNECTED {
                    this.analyse(cx);
                }
            }),
        ];
        let mut page = AnalysisPage {
            root,
            path,
            orientation,
            study,
            chapter,
            study_pgn,
            study_error,
            study_save: None,
            navigation: 0,
            study_bar,
            review: review::ReviewState::new(mistakes),
            practice: None,
            paused_for_review: false,
            notes: notes::Notes {
                field: notes_field,
                editing: false,
                filled_for: None,
            },
            moves_scroll: ScrollHandle::new(),
            reveal_move: std::rc::Rc::new(std::cell::Cell::new(true)),
            explorer,
            explorer_open: false,
            exporter: cx.new(|_| Exporter::default()),
            board,
            layout: PlayLayout::default(),
            move_input,
            move_message: None,
            lines_choice,
            import,
            import_error: None,
            attached: false,
            engine_ready: false,
            engine_identity: None,
            evaluations: HashMap::new(),
            order: VecDeque::new(),
            live: None,
            engine_error: None,
            engine_busy: false,
            wanted: String::new(),
            client_id: 0,
            latest_id: 0.,
            cloud: None,
            cloud_busy: false,
            cloud_error: None,
            cloud_request: 0,
            cloud_task: None,
            save: None,
            wheel: 0.,
            focus: cx.focus_handle(),
            voice: VoiceMoves::new("analysis", cx),
            tried_labels: Vec::new(),
            tried_done: Vec::new(),
            _subscriptions: subscriptions,
        };
        page.redraw(cx);
        let weak = cx.entity().downgrade();
        crate::core::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            Some(page.update(cx, |page, cx| {
                let study = page.persist_study(cx);
                let session = page.save_now(cx);
                Box::pin(async move {
                    let study = match study {
                        Some(study) => study.await.map(|_| ()),
                        None => Ok(()),
                    };
                    // The board's moves are in the session even when the study refused them.
                    session.await.and(study)
                }) as BoxedSave
            }))
        });
        page
    }

    /* ── The tree ─────────────────────────────────────────────────────── */

    fn node(&self) -> &TreeNode {
        self.root.node_at(&self.path)
    }

    fn position(&self) -> Option<VariantPosition> {
        position_of(&self.node().fen)
    }

    fn context(&self) -> String {
        analysis_context(&self.root.fen, &moves_of(&self.path))
    }

    /// Play a move from the position shown; false when it is not kept (not legal there, or a
    /// wrong answer while practising).
    fn play(&mut self, uci: &str, cx: &mut Context<Self>) -> bool {
        if let Some(kept) = self.practice_attempt(uci, cx) {
            return kept;
        }
        self.play_move(uci, cx)
    }

    fn play_move(&mut self, uci: &str, cx: &mut Context<Self>) -> bool {
        match self.root.add_move(&self.path, uci) {
            Some(next) => {
                self.path = next;
                self.changed(cx);
                true
            }
            None => {
                self.redraw(cx);
                false
            }
        }
    }

    fn go_to(&mut self, path: &str, cx: &mut Context<Self>) {
        self.path = self.root.existing_path(path);
        self.changed(cx);
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        let mut moves = moves_of(&self.path);
        moves.pop();
        self.go_to(&path_of(&moves), cx);
    }

    pub(crate) fn forward(&mut self, cx: &mut Context<Self>) {
        if let Some(child) = self.node().children.first() {
            let next = child_path(&self.path, &child.uci);
            self.go_to(&next, cx);
        }
    }

    pub(crate) fn go_to_start(&mut self, cx: &mut Context<Self>) {
        self.go_to("", cx);
    }

    fn go_to_end(&mut self, cx: &mut Context<Self>) {
        let end = self.root.line_end(&self.path);
        self.go_to(&end, cx);
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.orientation = !self.orientation;
        self.changed(cx);
    }

    /// Remove a move and what follows it; the board stays where it was unless that was removed.
    fn remove(&mut self, at: &str, cx: &mut Context<Self>) {
        if at.is_empty() {
            return;
        }
        let parent = self.root.delete_at(at);
        if self.path == at || self.path.starts_with(&format!("{at} ")) {
            self.path = parent;
        }
        self.changed(cx);
    }

    fn promote(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.root.promote(&path);
        self.changed(cx);
    }

    /// Start again from a position (`load`), or the usual start.
    pub fn load(&mut self, fen: Option<&str>, cx: &mut Context<Self>) {
        self.take_board(TreeNode::new(fen), cx);
        self.changed(cx);
    }

    /// Analyse a position set up in the board editor, facing as it did there (`analyseSetup`).
    pub fn analyse_setup(&mut self, fen: &str, orientation: Color, cx: &mut Context<Self>) {
        self.orientation = orientation;
        self.load(Some(fen), cx);
    }

    /// Load a game (`loadPgn`), opened at its start.
    fn load_pgn(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(tree) = TreeNode::from_pgn(text) else {
            return false;
        };
        self.take_board(tree, cx);
        self.changed(cx);
        true
    }

    /// Another document takes the board: the open study keeps its last edits and is let go.
    fn take_board(&mut self, tree: TreeNode, cx: &mut Context<Self>) {
        self.navigation += 1;
        self.persist_study(cx);
        self.root = tree;
        self.path.clear();
        self.clear_evaluations();
        self.study.clear();
        self.review.origin = None;
    }

    /* ── Studies ──────────────────────────────────────────────────────── */

    /// The whole tree as PGN (`analysis.pgn()`).
    pub fn pgn(&self) -> String {
        self.root.to_pgn()
    }

    pub fn study_id(&self) -> &str {
        &self.study
    }

    pub fn chapter_id(&self) -> &str {
        &self.chapter
    }

    pub fn study_error(&self) -> Option<&str> {
        self.study_error.as_deref()
    }

    /// The game's PGN headers (White, Black, Event, Date, Result).
    pub fn header(&self, key: &str) -> String {
        self.root
            .headers
            .as_ref()
            .and_then(|headers| headers.get(key))
            .and_then(Value::as_str)
            .filter(|value| *value != "?")
            .unwrap_or_default()
            .to_string()
    }

    pub fn set_header(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        self.root
            .headers
            .get_or_insert_with(Default::default)
            .insert(key.to_string(), json!(value));
        self.changed(cx);
    }

    /// The white and black players named in the headers, for a new study's name.
    pub fn players(&self) -> Option<String> {
        let (white, black) = (self.header("White"), self.header("Black"));
        (!white.is_empty() && !black.is_empty()).then(|| format!("{white} – {black}"))
    }

    /// Write the board to its study when it changed there (`persistStudy`); the answer, for a
    /// quit to wait on.
    fn persist_study(&mut self, cx: &mut Context<Self>) -> Option<StudySave> {
        self.study_save = None;
        if self.study.is_empty() {
            return None;
        }
        let library = cx.global::<StudyHandle>().0.clone();
        let name = library.read(cx).study(&self.study)?["name"].clone();
        let text = self.root.to_pgn();
        if text == self.study_pgn {
            return None;
        }
        // Counted as saved while the core writes it; a failure makes it unsaved again.
        let previous = std::mem::replace(&mut self.study_pgn, text.clone());
        let study = self.study.clone();
        let command = library.read(cx).command(json!({
            "op": "save",
            "name": name,
            "pgn": text,
            "id": self.study,
            "chapterId": self.chapter,
        }));
        let answer = (Box::pin(command) as std::pin::Pin<Box<dyn Future<Output = _>>>).shared();
        let outcome = answer.clone();
        cx.spawn(async move |this, cx| {
            let result: Result<Value, String> = outcome.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.study_error = None,
                    // Another window changed the chapter: the board keeps its moves as an unsaved
                    // copy rather than overwriting that window's, or blocking every later save.
                    Err(cause)
                        if crate::studies::is_stale(cause.as_str()) && this.study == study =>
                    {
                        this.study.clear();
                        this.chapter.clear();
                        this.study_pgn.clear();
                        this.study_error = Some(STALE_COPY.to_string());
                    }
                    Err(cause) => {
                        if this.study_pgn == text {
                            this.study_pgn = previous;
                        }
                        this.study_error = Some(cause);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        Some(answer)
    }

    /// Every edit so far written to the study, or why it could not be.
    pub fn flush_study(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl Future<Output = Result<(), String>> + 'static + use<> {
        let save = self.persist_study(cx);
        async move {
            match save {
                Some(save) => save.await.map(|_| ()),
                None => Ok(()),
            }
        }
    }

    /// Show a study's chapter (its first by default); its edits are saved to it from now on.
    pub fn open_study(
        &mut self,
        id: &str,
        chapter: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let library = cx.global::<StudyHandle>().0.clone();
        let found = library.read(cx).study(id).cloned();
        let chapters = found
            .as_ref()
            .and_then(|study| study["chapters"].as_array().cloned())
            .unwrap_or_default();
        let opened = chapters
            .iter()
            .find(|entry| Some(entry["id"].as_str().unwrap_or_default()) == chapter)
            .or_else(|| chapters.first())
            .cloned();
        let tree = opened
            .as_ref()
            .and_then(|chapter| chapter["pgn"].as_str())
            .and_then(TreeNode::from_pgn);
        let (Some(opened), Some(tree)) = (opened, tree) else {
            return Err("This study could not be opened.".into());
        };
        self.take_board(tree, cx);
        self.study = id.to_string();
        self.chapter = opened["id"].as_str().unwrap_or_default().to_string();
        library.read(cx).track(&self.study, &self.chapter);
        self.study_pgn = self.root.to_pgn();
        self.study_error = None;
        self.changed(cx);
        Ok(())
    }

    /// Keep the board as a new study (named), and save its edits to it from now on.
    pub fn save_as_study(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<String, String>> {
        self.persist_study(cx);
        self.navigation += 1;
        let navigation = self.navigation;
        let text = self.root.to_pgn();
        let library = cx.global::<StudyHandle>().0.clone();
        let answer = library
            .read(cx)
            .command(json!({ "op": "save", "name": name, "pgn": text }));
        cx.spawn(async move |this, cx| {
            let result = answer.await?;
            let id = result["id"]
                .as_str()
                .ok_or("The study could not be saved.")?
                .to_string();
            this.update(cx, |this, cx| {
                library.update(cx, |library, cx| {
                    library.apply(&result);
                    cx.notify();
                });
                // The new study stays in the library either way; only the board it was made
                // from links to it.
                if this.navigation != navigation {
                    return Ok(id.clone());
                }
                let chapter = library
                    .read(cx)
                    .study(&id)
                    .and_then(|study| study["chapters"][0]["id"].as_str().map(str::to_string));
                let Some(chapter) = chapter else {
                    return Err("The saved study has no chapter.".to_string());
                };
                this.study = id.clone();
                this.chapter = chapter;
                this.study_pgn = text;
                this.study_error = None;
                this.schedule_save(cx);
                cx.notify();
                Ok(id)
            })
            .map_err(|_| "The analysis board closed.".to_string())?
        })
    }

    /// Save the open study, then return to an empty board; the study stays open when it cannot
    /// be saved.
    pub fn close_study(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let flush = self.flush_study(cx);
        cx.spawn(async move |this, cx| {
            let saved = flush.await;
            this.update(cx, |this, cx| match saved {
                Ok(()) => {
                    this.study.clear();
                    this.chapter.clear();
                    this.study_pgn.clear();
                    this.load(None, cx);
                    true
                }
                Err(cause) => {
                    this.study_error = Some(cause);
                    cx.notify();
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    /// Link the board to a study again (a deleted one restored), while the board still holds it.
    pub fn relink(&mut self, id: String, chapter: String, navigation: u64, cx: &mut Context<Self>) {
        if self.navigation == navigation && self.study.is_empty() {
            cx.global::<StudyHandle>().0.read(cx).track(&id, &chapter);
            self.study = id;
            self.chapter = chapter;
            self.schedule_save(cx);
            cx.notify();
        }
    }

    pub fn navigation(&self) -> u64 {
        self.navigation
    }

    /// The board holds moves no study keeps (`useReplaceAnalysis`), so replacing it asks first.
    pub fn unsaved(&self) -> bool {
        self.study.is_empty() && !self.root.children.is_empty()
    }

    /// Load a line from the starting position, opened at its end (an opening's Analyse).
    pub fn load_line(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let Ok(pgn) = rules("pgnFromUci", &[json!(moves)]) else {
            return;
        };
        if self.load_pgn(pgn.as_str().unwrap_or_default(), cx) {
            self.go_to_end(cx);
        }
    }

    fn clear_evaluations(&mut self) {
        self.evaluations.clear();
        self.order.clear();
        self.live = None;
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let setup = json!({ "variant": "standard", "fen": self.node().fen });
        let typed = self
            .position()
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        match typed {
            Some(uci) => {
                self.move_message = None;
                self.move_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.play(&uci, cx);
            }
            None => {
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3.".into());
                cx.notify();
            }
        }
    }

    /// The board, the engine and the saved session follow a change.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.reveal_move.set(true);
        self.follow_review(cx);
        self.check_practice(cx);
        self.redraw(cx);
        self.analyse(cx);
        self.ask_cloud(cx);
        self.schedule_save(cx);
    }

    /// Save the session, and the open study's chapter, shortly after the last change.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.study.is_empty() {
            self.study_save = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVE_DELAY).await;
                let _ = this.update(cx, |this, cx| this.persist_study(cx));
            }));
        }
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let save = this.update(cx, |this, cx| this.save_now(cx)).ok();
            if let Some(save) = save {
                let _ = save.await;
            }
        }));
    }

    /// Save the session now (`persistSession('analysis', …)`).
    fn save_now(&mut self, cx: &mut Context<Self>) -> BoxedSave {
        self.save = None;
        let session = json!({
            "pgn": self.root.to_pgn(),
            "path": self.path,
            "orientation": if self.orientation == Color::White { "white" } else { "black" },
            "study": self.study,
            "chapter": self.chapter,
        });
        let save = cx
            .global::<AppCore>()
            .call("saveSession", vec![json!("analysis"), session]);
        Box::pin(async move {
            save.await.map(|_| ()).map_err(|cause| {
                crate::log::write(
                    Level::Warn,
                    "library",
                    &format!("saving session failed: analysis {}", cause.message),
                );
                format!("The analysis board could not be saved: {}", cause.message)
            })
        })
    }

    /* ── The engine ───────────────────────────────────────────────────── */

    pub(crate) fn engine_on(cx: &App) -> bool {
        crate::prefs::get(cx, ENGINE_ON, true)
    }

    /// Practising a position: the engine (and anything else that knows) would tell.
    fn hides_engine(&self) -> bool {
        self.practice
            .as_ref()
            .is_some_and(practice::Practice::hides_engine)
    }

    fn assistance_allowed(cx: &App) -> bool {
        cx.global::<OnlineHandle>().0.read(cx).assistance_allowed()
    }

    /// Start following the board with the engine (the page is shown).
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.attached {
            return;
        }
        self.attached = true;
        let status = cx.global::<AppCore>().call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let status = status.await.unwrap_or(Value::Null);
            let _ = this.update(cx, |this, cx| {
                this.engine_ready = status["ready"].as_bool() == Some(true);
                let identity = status["identity"].as_str().map(str::to_string);
                if identity != this.engine_identity {
                    // Another engine: what the old one said is no longer this one's.
                    this.engine_identity = identity;
                    this.clear_evaluations();
                }
                this.engine_changed(cx);
            });
        })
        .detach();
        self.ask_cloud(cx);
        self.analyse(cx);
    }

    /// Stop the engine when the page is left.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        self.attached = false;
        self.cloud_task = None;
        self.cloud_request += 1;
        self.wanted.clear();
        self.engine_busy = false;
        stop_analysis(true, cx);
    }

    fn engine_changed(&mut self, cx: &mut Context<Self>) {
        self.analyse(cx);
        self.ask_cloud(cx);
        self.redraw(cx);
    }

    /// `analyse`: search the position shown, unless the engine is off, nothing is left to search,
    /// or a finished search with enough lines is already known.
    fn analyse(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        let fen = self.node().fen.clone();
        self.client_id += 1;
        let request = self.client_id;
        self.engine_error = None;
        let engine_on = Self::engine_on(cx);
        let lines = prefs_lines(cx);
        let infinite = crate::prefs::get(cx, INFINITE, false);
        let position = self.position();
        if !engine_on
            || !self.engine_ready
            || position.is_none()
            || game_over(position.as_ref()).is_some()
            || !Self::assistance_allowed(cx)
            || self.reviewing(cx).is_some()
        {
            self.wanted.clear();
            self.engine_busy = false;
            // Turning analysis off releases the engine; other pauses keep it warm.
            stop_analysis(!engine_on, cx);
            cx.notify();
            return;
        }
        let context = self.context();
        let known = self.evaluations.get(&context);
        if !infinite
            && known.is_some_and(|known| {
                known["reason"] == "completed"
                    && known["engine"].as_str() == self.engine_identity.as_deref()
                    && known["lines"].as_array().map_or(0, Vec::len) >= lines
            })
        {
            self.wanted = fen;
            self.engine_busy = false;
            stop_analysis(false, cx);
            cx.notify();
            return;
        }
        self.wanted = fen.clone();
        self.live = None;
        self.engine_busy = true;
        let start = cx.global::<AppCore>().call(
            "startAnalysis",
            vec![json!({
                "fen": fen,
                "clientId": request,
                "rootFen": self.root.fen,
                "moves": moves_of(&self.path),
                "lines": lines.clamp(1, 5),
                "infinite": infinite,
            })],
        );
        cx.spawn(async move |this, cx| {
            let started = start.await;
            let _ = this.update(cx, |this, cx| {
                let current = this.wanted == fen && this.client_id == request;
                match started {
                    Ok(id) if current => {
                        this.latest_id = this.latest_id.max(id.as_f64().unwrap_or_default());
                    }
                    Err(cause) if current => {
                        crate::log::write(
                            Level::Warn,
                            "analysis",
                            &format!("starting analysis failed: {}", cause.message),
                        );
                        this.engine_busy = false;
                        this.engine_error = Some(cause.message);
                        cx.notify();
                    }
                    _ => {}
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// An engine update: kept when it is for the search running now.
    fn receive(&mut self, update: Value, cx: &mut Context<Self>) {
        let id = update["id"].as_f64().unwrap_or_default();
        if update["fen"].as_str() != Some(self.wanted.as_str())
            || update["clientId"].as_f64() != Some(self.client_id as f64)
            || update["context"].as_str() != Some(self.context().as_str())
            || id < self.latest_id
        {
            return;
        }
        self.latest_id = id;
        self.remember(&update);
        if let Some(error) = update["error"].as_str() {
            self.engine_error = Some(error.to_string());
            self.engine_busy = false;
        } else if update["done"] == json!(true) {
            self.engine_busy = false;
        }
        self.live = Some(update);
        self.redraw(cx);
    }

    /// `remember`: keep the deepest evaluation of each position, a completed one over an
    /// interrupted one, and never a failed search.
    fn remember(&mut self, update: &Value) {
        let key = update["context"]
            .as_str()
            .or(update["fen"].as_str())
            .unwrap_or_default()
            .to_string();
        let lines = update["lines"].as_array().map_or(0, Vec::len);
        if lines == 0 || !update["error"].is_null() || update["reason"] == "failed" {
            return;
        }
        if let Some(known) = self.evaluations.get(&key) {
            if known["reason"] == "completed" && update["reason"] != "completed" {
                let (old, new) = (known["engine"].as_str(), update["engine"].as_str());
                if old.is_none() || new.is_none() || old == new {
                    return;
                }
            }
            if known["depth"].as_f64() > update["depth"].as_f64()
                && known["lines"].as_array().map_or(0, Vec::len) >= lines
            {
                return;
            }
        }
        self.order.retain(|entry| *entry != key);
        self.order.push_back(key.clone());
        self.evaluations.insert(key, update.clone());
        while self.order.len() > CACHE_SIZE {
            if let Some(oldest) = self.order.pop_front() {
                self.evaluations.remove(&oldest);
            }
        }
    }

    /// The deepest evaluation known for the position shown.
    fn evaluation(&self, cx: &App) -> Option<&Value> {
        if !Self::assistance_allowed(cx) {
            return None;
        }
        let context = self.context();
        if let Some(live) = &self.live
            && live["fen"].as_str() == Some(self.node().fen.as_str())
            && live["context"].as_str() == Some(context.as_str())
        {
            return Some(live);
        }
        self.evaluations.get(&context)
    }

    /* ── Lichess cloud evaluation ─────────────────────────────────────── */

    fn ask_cloud(&mut self, cx: &mut Context<Self>) {
        self.cloud_task = None;
        self.cloud_request += 1;
        let request = self.cloud_request;
        let fen = self.node().fen.clone();
        if self.cloud.as_ref().and_then(|cloud| cloud["fen"].as_str()) != Some(fen.as_str()) {
            self.cloud = None;
        }
        self.cloud_error = None;
        let cloud_on = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .core
            .cloud_eval;
        if !self.attached
            || !cloud_on
            || !Self::assistance_allowed(cx)
            || game_over(self.position().as_ref()).is_some()
        {
            self.cloud = None;
            self.cloud_busy = false;
            return;
        }
        let lines = prefs_lines(cx).clamp(1, 5);
        let core = cx.global::<AppCore>().clone();
        self.cloud_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOUD_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_busy = true;
                cx.notify();
            });
            let answer = core.call("cloudEval", vec![json!(fen), json!(lines)]).await;
            let _ = this.update(cx, |this, cx| {
                if request != this.cloud_request {
                    return;
                }
                this.cloud_busy = false;
                match answer {
                    Ok(cloud) => this.cloud = (!cloud.is_null()).then_some(cloud),
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "analysis",
                            &format!("cloud evaluation failed: {}", cause.message),
                        );
                        this.cloud_error = Some(cause.message);
                    }
                }
                this.redraw(cx);
            });
        }));
    }

    /// Lichess's lines for the position shown, when it has some.
    fn cloud_lines(&self, cx: &App) -> Vec<Value> {
        match &self.cloud {
            Some(cloud)
                if Self::assistance_allowed(cx)
                    && cloud["fen"].as_str() == Some(self.node().fen.as_str()) =>
            {
                cloud["lines"].as_array().cloned().unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// Without the local engine, the cloud's lines drive the bar, arrows and line list.
    fn using_cloud(&self, cx: &App) -> bool {
        (!Self::engine_on(cx) || !self.engine_ready) && !self.cloud_lines(cx).is_empty()
    }

    fn lines(&self, cx: &App) -> Vec<Value> {
        if self.using_cloud(cx) {
            self.cloud_lines(cx)
        } else {
            self.evaluation(cx)
                .and_then(|update| update["lines"].as_array().cloned())
                .unwrap_or_default()
        }
    }

    /// Play an engine line up to (and including) the move clicked.
    fn play_line(&mut self, pv: &[String], up_to: usize, cx: &mut Context<Self>) {
        for uci in pv.iter().take(up_to + 1) {
            if !self.play(uci, cx) {
                break;
            }
        }
    }

    fn play_best(&mut self, cx: &mut Context<Self>) {
        if self.hides_engine() {
            return;
        }
        let best = self
            .lines(cx)
            .first()
            .and_then(|line| line["pv"][0].as_str().map(str::to_string));
        if let Some(uci) = best {
            self.play(&uci, cx);
        }
    }

    /// The board shows the position, its last move and the engine's arrows.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let node = self.node();
        let position = position_of(&node.fen).unwrap_or_default();
        let last_move = now_square(&node.uci, 0..2).zip(now_square(&node.uci, 2..4));
        let mut state = BoardState::of(
            &position,
            self.orientation,
            Some(position.turn()),
            last_move,
        );
        // The review's label on the move just played.
        let marks = self.review_marks(cx);
        state.badge = marks
            .get(&self.path)
            .and_then(|mark| mark.classification.as_deref())
            .zip(last_move)
            .map(|(classification, (_, to))| {
                let (_, glyph, color) =
                    review::classification_look(classification, cx.global::<Palette>().dark);
                Badge {
                    square: to,
                    glyph,
                    color,
                }
            });
        // Practising: the game's move in red, to find a better one, and the piece to move once
        // hinted.
        if let Some(practice::PracticeShapes { played, hint }) = self.practice_shapes() {
            state.arrows = played
                .map(|(from, to)| Arrow::brush("red", from, to))
                .into_iter()
                .collect();
            state.circles = hint.map(Circle::green).into_iter().collect();
            self.board
                .update(cx, |board, cx| board.set_state(state, cx));
            cx.notify();
            return;
        }
        let engine_shown = (Self::engine_on(cx) || self.using_cloud(cx))
            && crate::prefs::get(cx, ARROWS, true)
            && game_over(Some(&position)).is_none();
        if engine_shown {
            state.arrows = self
                .lines(cx)
                .iter()
                .enumerate()
                .filter_map(|(rank, line)| {
                    let uci = line["pv"][0].as_str()?;
                    Some(Arrow::engine(
                        now_square(uci, 0..2)?,
                        now_square(uci, 2..4)?,
                        rank,
                    ))
                })
                .collect();
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /* ── Keys, wheel, menus ───────────────────────────────────────────── */

    /// ← → ↑ ↓ step, F flips, L turns the engine on or off, Space plays the best move.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" => self.back(cx),
            "right" => self.forward(cx),
            "up" => self.go_to_start(cx),
            "down" => self.go_to_end(cx),
            "f" => self.flip(cx),
            "l" => self.toggle_engine(cx),
            "c" => self.focus_notes(window, cx),
            "space"
                if !cx
                    .global::<SettingsHandle>()
                    .0
                    .read(cx)
                    .settings
                    .frontend
                    .voice_push_to_talk =>
            {
                self.play_best(cx)
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The moves that led here, so that phrases said here learn which move was then played.
    fn voice_line(&self) -> Vec<String> {
        self.root
            .nodes_along(&self.path)
            .iter()
            .skip(1)
            .map(|node| node.san.clone())
            .collect()
    }

    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let (context, line) = (self.path.clone(), self.voice_line());
        self.voice.sync(
            &context,
            Some(line),
            crate::voice::grammar("ANALYSIS_GRAMMAR"),
            true,
            cx,
        );
    }

    fn voice_heard(&mut self, event: &crate::voice::VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) {
            return;
        }
        let fen = self.node().fen.clone();
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                self.voice.missed(heard, &fen, true, cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let setup = json!({ "variant": "standard", "fen": fen });
        let best = (Self::assistance_allowed(cx) && !self.hides_engine())
            .then(|| self.lines(cx).first().cloned())
            .flatten()
            .and_then(|line| line["pv"][0].as_str().map(str::to_string));
        let mut command: Option<String> = None;
        let turn = Turn {
            taking: true,
            idle: None,
            immediate: true,
        };
        let action = self.voice.hear(
            &heard,
            &setup,
            turn,
            |text| {
                let said = crate::game::rules("spokenAnalysisCommand", &[json!(text)]).ok()?;
                let said = said.as_str()?.to_string();
                let feedback = if said == "best" && best.is_none() {
                    "The engine has no move yet.".to_string()
                } else {
                    format!("“{text}”")
                };
                command = Some(said.clone());
                Some(Reply {
                    said,
                    feedback,
                    understood: true,
                })
            },
            cx,
        );
        match (action, command.as_deref()) {
            (Action::Play(uci), _) => {
                self.play(&uci, cx);
            }
            (_, Some("back")) => self.back(cx),
            (_, Some("forward")) => self.forward(cx),
            (_, Some("start")) => self.go_to_start(cx),
            (_, Some("end")) => self.go_to_end(cx),
            (_, Some("best")) => {
                if let Some(uci) = best {
                    self.play(&uci, cx);
                }
            }
            (_, Some("flip")) => self.flip(cx),
            (_, Some("engine")) => self.toggle_engine(cx),
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn toggle_engine(&mut self, cx: &mut Context<Self>) {
        let on = !Self::engine_on(cx);
        crate::prefs::set(cx, ENGINE_ON, on);
        self.engine_changed(cx);
    }

    /// Lichess steps through moves with the mouse wheel over the board.
    fn scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(window.line_height());
        if delta.x.abs() > delta.y.abs() {
            return;
        }
        // Down (towards the user) goes forward, as `deltaY > 0` does in the browser.
        self.wheel -= delta.y.as_f32();
        if self.wheel.abs() < 40. {
            return;
        }
        if self.wheel > 0. {
            self.forward(cx);
        } else {
            self.back(cx);
        }
        self.wheel = 0.;
        cx.stop_propagation();
    }

    /// The game an Export saves: the main line, and the position shown on it.
    fn game_export(&self) -> GameExport {
        let mainline = self.mainline();
        let depth = if self.path.is_empty() {
            0
        } else {
            self.path.split(' ').count()
        };
        GameExport {
            variant: "standard".into(),
            fen: self.root.fen.clone(),
            ply: Some(depth.min(mainline.len())),
            moves: mainline,
            orientation: self.orientation,
            white: self.player_name(Color::White),
            black: self.player_name(Color::Black),
            result: Some(self.header("Result")).filter(|result| !result.is_empty()),
            lichess_id: self
                .review
                .origin
                .as_ref()
                .and_then(|origin| origin["gameId"].as_str())
                .map(str::to_string),
        }
    }

    fn copy(&self, text: String, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(SharedString::from(format!("{what} copied")), cx);
    }

    /// Cutting a line has no undo, so it asks first.
    fn ask_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.path.clone();
        if at.is_empty() {
            return;
        }
        let page = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let (page, at) = (page.clone(), at.clone());
            dialog
                .title("Delete from here?")
                .description(
                    "This move and every move after it, with their variations and comments, are removed.",
                )
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.remove(&at, cx));
                    true
                })
        });
    }

    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.import_error = None;
        self.import
            .update(cx, |text, cx| text.set_value("", window, cx));
        let page = cx.entity().downgrade();
        let import = self.import.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let error = page
                .upgrade()
                .and_then(|page| page.read(cx).import_error.clone());
            let (run, text) = (page.clone(), import.clone());
            dialog.title("Import a position or game").w(px(512.)).child(
                v_flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(cx.global::<Palette>().text_muted)
                            .child("Paste a FEN, or a PGN with or without variations."),
                    )
                    .child(Textarea::new(&import).aria_label("FEN or PGN"))
                    .children(error.map(|error| {
                        div()
                            .text_size(px(14.))
                            .text_color(cx.global::<Palette>().error)
                            .child(error)
                    }))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                Button::new("import-cancel")
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("import-run")
                                    .primary()
                                    .label("Import")
                                    .disabled(text.read(cx).value().trim().is_empty())
                                    .on_click(move |_, window, cx| {
                                        let _ =
                                            run.update(cx, |page, cx| page.run_import(window, cx));
                                    }),
                            ),
                    ),
            )
        });
    }

    /// `runImport`: a FEN starts a new analysis there; otherwise the text is read as one PGN game.
    fn run_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.import.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(fen) = crate::game::editor_setup(&text)
            .and_then(|setup| crate::game::rules("setupFen", &[setup]).ok())
            .and_then(|fen| fen.as_str().map(str::to_string))
        {
            if let Some(problem) = crate::game::position_problem(&fen) {
                self.import_error = Some(problem.into());
                window.refresh();
                return;
            }
            self.load(Some(&fen), cx);
        } else if !self.load_pgn(&text, cx) {
            self.import_error =
                Some("Use a valid FEN or one PGN game with legal moves, under 2 MB.".into());
            window.refresh();
            return;
        } else if self.root.children.is_empty() {
            self.import_error = Some("No moves could be read from that PGN.".into());
            window.refresh();
            return;
        }
        self.import_error = None;
        window.close_dialog(cx);
    }
}

/// The engine lines chosen (1–5, three by default).
fn prefs_lines(cx: &App) -> usize {
    crate::prefs::get::<usize>(cx, ENGINE_LINES, 3).clamp(1, 5)
}

/// `stopAnalysisSafe`: stop the search; `release` also closes the engine.
fn stop_analysis(release: bool, cx: &mut App) {
    let stop = cx.global::<AppCore>().call(
        "stopAnalysis",
        if release {
            vec![json!(true)]
        } else {
            Vec::new()
        },
    );
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = stop.await {
                crate::log::write(
                    Level::Warn,
                    "analysis",
                    &format!("stopping analysis failed: {}", cause.message),
                );
            }
        })
        .detach();
}

/* ── Rendering ────────────────────────────────────────────────────────── */

/// One piece of the move list: moves and inline comments flow; main-line comments and the
/// variations of the main line stand on their own lines.
enum Piece {
    Inline(AnyElement),
    Block(AnyElement),
}

/// What a move's menu needs to know of it.
#[derive(Clone)]
struct MoveMenu {
    commented: bool,
    nag: Option<i64>,
    mainline: bool,
    fen: String,
}

/// What a menu item does to the page.
type PageAction = Box<dyn Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>)>;

/// Right-click a move to comment on it, annotate it, promote or delete it.
fn move_menu(
    page: &WeakEntity<AnalysisPage>,
    path: &str,
    state: &MoveMenu,
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let act = |run: PageAction| {
        let page = page.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| run(page, window, cx));
        }
    };
    let (at, current) = (path.to_string(), state.nag);
    let annotate = {
        let (page, at) = (page.clone(), at.clone());
        move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            notes::MOVE_GLYPHS
                .iter()
                .fold(menu, |menu, (nag, glyph, label)| {
                    let (page, at, nag) = (page.clone(), at.clone(), *nag);
                    menu.item(
                        PopupMenuItem::new(format!("{glyph}  {label}"))
                            .checked(current == Some(nag))
                            .on_click(move |_, _, cx| {
                                let _ = page.update(cx, |page, cx| page.toggle_glyph(&at, nag, cx));
                            }),
                    )
                })
        }
    };
    let (promote_at, delete_at, fen) = (at.clone(), at.clone(), state.fen.clone());
    menu.item(
        PopupMenuItem::new(if state.commented {
            "Edit comment"
        } else {
            "Add comment"
        })
        .icon(icon("message-square-text"))
        .on_click(act(Box::new(|page, window, cx| {
            page.focus_notes(window, cx)
        }))),
    )
    .submenu_with_icon(Some(icon("highlighter")), "Annotate", window, cx, annotate)
    .separator()
    .item(
        PopupMenuItem::new("Make main line")
            .icon(icon("arrow-up-to-line"))
            .disabled(state.mainline)
            .on_click(act(Box::new(move |page, _, cx| {
                page.path = promote_at.clone();
                page.promote(cx)
            }))),
    )
    .item(
        PopupMenuItem::new("Delete from here")
            .icon(icon("scissors"))
            .on_click(act(Box::new(move |page, window, cx| {
                page.path = delete_at.clone();
                page.ask_remove(window, cx)
            }))),
    )
    .separator()
    .item(
        PopupMenuItem::new("Copy FEN")
            .icon(icon("copy"))
            .on_click(act(Box::new(move |page, window, cx| {
                page.copy(fen.clone(), "FEN", window, cx)
            }))),
    )
}

struct TreeView<'a> {
    current: &'a str,
    page: WeakEntity<AnalysisPage>,
    palette: Palette,
    /// The review's label on each main-line move, by path.
    marks: HashMap<String, review::ReviewMark>,
    /// Paths on the main line.
    mainline: std::collections::HashSet<String>,
    /// Bring the move shown into view after it changes (`block: 'nearest'`).
    reveal: std::rc::Rc<std::cell::Cell<bool>>,
    scroll: ScrollHandle,
}

impl TreeView<'_> {
    fn move_chip(&self, node: &TreeNode, path: &str, number: String, main: bool) -> AnyElement {
        let active = path == self.current;
        let palette = &self.palette;
        let (page, target) = (self.page.clone(), path.to_string());
        let glyph = move_glyph(node);
        let annotated = glyph.is_some();
        let menu_state = MoveMenu {
            commented: node
                .comments
                .iter()
                .flatten()
                .any(|comment| !comment_text(comment).is_empty()),
            nag: node
                .nags
                .iter()
                .flatten()
                .copied()
                .find(|nag| (1..=6).contains(nag)),
            mainline: self.mainline.contains(path),
            fen: node.fen.clone(),
        };
        let (menu_page, menu_path) = (page.clone(), path.to_string());
        h_flex()
            .id(SharedString::from(format!("tree-{path}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(node.san.clone())
            .aria_selected(active)
            .px(px(4.))
            .py(px(2.))
            .mx(px(1.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(if main { palette.text } else { palette.toned })
            .when(main, |chip| chip.font_weight(FontWeight::SEMIBOLD))
            .when(active, |chip| {
                chip.bg(tint(palette.primary, 30.))
                    .text_color(palette.highlighted)
            })
            .when(!active, |chip| chip.hover(|chip| chip.bg(palette.accented)))
            .on_click({
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
                }
            })
            // A right-click shows the move it is about, then its menu.
            .on_mouse_down(MouseButton::Right, {
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
                }
            })
            .when(active, |chip| {
                let (reveal, scroll) = (self.reveal.clone(), self.scroll.clone());
                chip.relative().child(
                    canvas(
                        move |bounds, window, _| {
                            if !reveal.replace(false) {
                                return;
                            }
                            let view = scroll.bounds();
                            let mut offset = scroll.offset();
                            let margin = px(8.);
                            if bounds.bottom() + margin > view.bottom() {
                                offset.y -= bounds.bottom() + margin - view.bottom();
                            } else if bounds.top() - margin < view.top() {
                                offset.y += view.top() - (bounds.top() - margin);
                            } else {
                                return;
                            }
                            scroll.set_offset(offset);
                            window.refresh();
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .when(!number.is_empty(), |chip| {
                chip.child(
                    div()
                        .mr(px(2.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(palette.dimmed)
                        .child(number),
                )
            })
            .child(node.san.clone())
            .children(
                glyph.map(|(glyph, _)| div().ml(px(1.)).font_weight(FontWeight::BOLD).child(glyph)),
            )
            // Without an annotation, the review's label.
            .when(!annotated, |chip| {
                chip.children(
                    self.marks
                        .get(path)
                        .and_then(|mark| mark.classification.as_deref())
                        .map(|classification| {
                            let (label, glyph, color) =
                                review::classification_look(classification, palette.dark);
                            div()
                                .id(SharedString::from(format!("label-{path}")))
                                .ml(px(1.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(color)
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(label)
                                        .build(window, cx)
                                })
                                .child(glyph)
                        }),
                )
            })
            .context_menu(move |menu, window, cx| {
                move_menu(&menu_page, &menu_path, &menu_state, menu, window, cx)
            })
            .into_any_element()
    }

    fn comment(&self, text: String, path: &str, block: bool) -> AnyElement {
        let (page, target) = (self.page.clone(), path.to_string());
        let palette = &self.palette;
        div()
            .id(SharedString::from(format!("comment-{path}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(text.clone())
            .text_size(px(12.5))
            .italic()
            .line_height(relative(1.5))
            .text_color(palette.text_muted)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
            })
            .when(block, |comment| {
                comment
                    .w_full()
                    .my(px(2.))
                    .px(px(8.))
                    .py(px(4.))
                    .border_l_2()
                    .border_color(tint(palette.primary, 50.))
                    .rounded_r(px(6.))
                    .bg(palette.elevated)
            })
            .when(!block, |comment| comment.mx(px(4.)))
            .child(text)
            .into_any_element()
    }

    fn paren(&self, text: &'static str) -> AnyElement {
        div()
            .text_color(self.palette.dimmed)
            .child(text)
            .into_any_element()
    }

    /// A line's moves run on, the alternatives to a move follow it in
    /// brackets before the line resumes. Off the main line everything stays inline.
    fn line(
        &self,
        parent: &TreeNode,
        parent_path: &str,
        first: Option<&TreeNode>,
        depth: usize,
        out: &mut Vec<Piece>,
    ) {
        let mut parent = parent;
        let mut parent_path = parent_path.to_string();
        let mut node = first.or_else(|| parent.children.first());
        let mut numbered = true;
        while let Some(current) = node {
            let path = child_path(&parent_path, &current.uci);
            out.push(Piece::Inline(self.move_chip(
                current,
                &path,
                move_number(parent.ply, numbered),
                depth == 0,
            )));
            let comment = current
                .comments
                .iter()
                .flatten()
                .map(|text| comment_text(text))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !comment.is_empty() {
                let element = self.comment(comment, &path, depth == 0);
                out.push(if depth == 0 {
                    Piece::Block(element)
                } else {
                    Piece::Inline(element)
                });
            }
            numbered = false;
            let is_main = parent
                .children
                .first()
                .is_some_and(|main| main.uci == current.uci);
            if is_main && parent.children.len() > 1 {
                if depth == 0 {
                    let rows: Vec<AnyElement> = parent.children[1..]
                        .iter()
                        .map(|alternative| {
                            let mut inline = vec![Piece::Inline(self.paren("("))];
                            self.line(parent, &parent_path, Some(alternative), 1, &mut inline);
                            inline.push(Piece::Inline(self.paren(")")));
                            flow(inline).into_any_element()
                        })
                        .collect();
                    out.push(Piece::Block(
                        v_flex()
                            .w_full()
                            .my(px(2.))
                            .ml(px(10.))
                            .pl(px(8.))
                            .border_l_2()
                            .border_color(self.palette.border)
                            .text_size(px(12.5))
                            .children(rows)
                            .into_any_element(),
                    ));
                } else {
                    for alternative in &parent.children[1..] {
                        out.push(Piece::Inline(self.paren("(")));
                        self.line(parent, &parent_path, Some(alternative), depth + 1, out);
                        out.push(Piece::Inline(self.paren(")")));
                    }
                }
                numbered = true;
            }
            parent = current;
            parent_path = path;
            node = current.children.first();
        }
    }
}

/// Moves and inline comments in wrapping rows, broken by the pieces that stand alone.
fn flow(pieces: Vec<Piece>) -> Div {
    let mut column = v_flex().w_full();
    let mut row: Vec<AnyElement> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Inline(element) => row.push(element),
            Piece::Block(element) => {
                if !row.is_empty() {
                    column = column.child(h_flex().flex_wrap().children(std::mem::take(&mut row)));
                }
                column = column.child(element);
            }
        }
    }
    if !row.is_empty() {
        column = column.child(h_flex().flex_wrap().children(row));
    }
    column
}

impl AnalysisPage {
    fn engine_status(&self, cx: &App) -> String {
        if self.using_cloud(cx)
            && let Some(cloud) = &self.cloud
        {
            return format!(
                "Lichess cloud · depth {} · {}M nodes",
                cloud["depth"].as_f64().unwrap_or_default(),
                (cloud["knodes"].as_f64().unwrap_or_default() / 1000.).round()
            );
        }
        if !self.engine_ready {
            return "Stockfish not found".into();
        }
        if let Some(error) = &self.engine_error {
            return error.clone();
        }
        if !Self::engine_on(cx) {
            return "Engine off".into();
        }
        if let Some(over) = game_over(self.position().as_ref()) {
            return over.into();
        }
        if !Self::assistance_allowed(cx) {
            return if cx.global::<OnlineHandle>().0.read(cx).phase == crate::online::Phase::Checking
            {
                "Checking Lichess game status…".into()
            } else {
                "Analysis paused until Lichess game status is verified".into()
            };
        }
        let Some(update) = self.evaluation(cx) else {
            return "Starting…".into();
        };
        let infinite = crate::prefs::get(cx, INFINITE, false);
        let depth = format!(
            "Depth {}{}",
            update["depth"].as_f64().unwrap_or_default(),
            if infinite { "" } else { "/26" }
        );
        let nps = update["nps"].as_f64().unwrap_or_default();
        let speed = if nps >= 1e6 {
            format!(" · {:.1}M nodes/s", nps / 1e6)
        } else if nps > 0. {
            format!(" · {}k nodes/s", (nps / 1000.).round())
        } else {
            String::new()
        };
        let tail = if update["reason"] == "interrupted" {
            " · Interrupted — retry or change position"
        } else if update["done"] == json!(true) {
            ""
        } else {
            "…"
        };
        format!("{depth}{speed}{tail}")
    }

    /// The engine's head row, the cloud's line and the engine lines (`.ceval`).
    fn render_engine(&self, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let engine_on = Self::engine_on(cx);
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let lines = self.lines(cx);
        let position = self.position();
        let over = game_over(position.as_ref());
        let result = over.map(|_| {
            if position
                .as_ref()
                .is_some_and(|position| position.is_checkmate())
            {
                if position.as_ref().map(|position| position.turn()) == Some(Color::White) {
                    "0-1"
                } else {
                    "1-0"
                }
            } else {
                "½-½"
            }
        });
        let score = if !engine_on && !using_cloud {
            "—".to_string()
        } else {
            result
                .map(str::to_string)
                .unwrap_or_else(|| format_eval(lines.first()))
        };
        let settings = cx.global::<SettingsHandle>().0.read(cx).settings.clone();
        let cloud_on = settings.core.cloud_eval;
        let lines_choice = self.lines_choice.clone();
        let page = cx.entity().downgrade();
        let settings_button = Popover::new("engine-settings")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("engine-settings-button")
                    .ghost()
                    .xsmall()
                    .icon(icon("settings-2"))
                    .tooltip("Engine settings")
                    .accessibility_label("Engine settings"),
            )
            .content(move |_, _, cx| {
                let engine_on = AnalysisPage::engine_on(cx);
                let infinite = crate::prefs::get(cx, INFINITE, false);
                let arrows = crate::prefs::get(cx, ARROWS, true);
                let cloud_on = cx
                    .global::<SettingsHandle>()
                    .0
                    .read(cx)
                    .settings
                    .core
                    .cloud_eval;
                let allowed = AnalysisPage::assistance_allowed(cx);
                let palette = *cx.global::<Palette>();
                let (on_infinite, on_arrows) = (page.clone(), page.clone());
                let described = |title: &'static str, hint: Option<&'static str>| {
                    v_flex()
                        .child(div().text_size(px(14.)).child(title))
                        .children(hint.map(|hint| {
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(hint)
                        }))
                };
                v_flex()
                    .w(px(260.))
                    .gap(px(12.))
                    .p(px(4.))
                    .child(
                        h_flex()
                            .gap(px(10.))
                            .child(
                                Switch::new("infinite")
                                    .small()
                                    .checked(infinite)
                                    .disabled(!engine_on)
                                    .accessibility_label("Infinite analysis")
                                    .on_click(move |on, _, cx| {
                                        crate::prefs::set(cx, INFINITE, *on);
                                        let _ = on_infinite
                                            .update(cx, |page, cx| page.engine_changed(cx));
                                    }),
                            )
                            .child(described(
                                "Infinite analysis",
                                Some("Keep searching past depth 26"),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap(px(10.))
                            .child(
                                Switch::new("arrows")
                                    .small()
                                    .checked(arrows)
                                    .disabled(!engine_on)
                                    .accessibility_label("Show engine arrows")
                                    .on_click(move |on, _, cx| {
                                        crate::prefs::set(cx, ARROWS, *on);
                                        let _ = on_arrows.update(cx, |page, cx| page.redraw(cx));
                                    }),
                            )
                            .child(described("Show engine arrows", None)),
                    )
                    .child(
                        h_flex()
                            .gap(px(10.))
                            .child(
                                Switch::new("cloud")
                                    .small()
                                    .checked(cloud_on)
                                    .disabled(!allowed)
                                    .accessibility_label("Lichess cloud evaluation")
                                    .on_click(|on, _, cx| {
                                        let on = *on;
                                        crate::settings::change(cx, move |s| {
                                            s.core.cloud_eval = on
                                        });
                                    }),
                            )
                            .child(described(
                                "Lichess cloud evaluation",
                                Some("Sends positions to Lichess"),
                            )),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_size(px(14.)).child("Engine lines"))
                            .child(
                                div().w(px(112.)).child(
                                    Select::new(&lines_choice)
                                        .small()
                                        .disabled(!engine_on && !cloud_on)
                                        .accessibility_label("Engine lines"),
                                ),
                            ),
                    )
            });
        let status = self.engine_status(cx);
        let head = h_flex()
            .gap(px(8.))
            .child(
                Switch::new("engine-on")
                    .checked(engine_on)
                    .disabled(!self.engine_ready || !allowed)
                    .accessibility_label("Engine analysis")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_engine(cx))),
            )
            .child(
                div()
                    .min_w(px(75.))
                    .text_size(px(22.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if !engine_on && !using_cloud {
                        palette.dimmed
                    } else {
                        palette.text
                    })
                    .child(score),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child("Stockfish"),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .min_w_0()
                            .text_size(px(11.))
                            .text_color(if self.engine_error.is_some() {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            .when(engine_on && self.engine_busy, |row| {
                                row.child(icon("loader-circle").size(px(12.)))
                            })
                            .child(div().truncate().child(status)),
                    ),
            )
            .child(settings_button);
        let cloud_line = (cloud_on && allowed && !using_cloud).then(|| {
            let cloud_lines = self.cloud_lines(cx);
            let text: AnyElement = match cloud_lines.first() {
                Some(best) => {
                    let pv: Vec<String> =
                        serde_json::from_value(best["pv"].clone()).unwrap_or_default();
                    let first = pv_san(&self.node().fen, &pv, 1)
                        .first()
                        .map(|played| played.san.clone())
                        .unwrap_or_default();
                    h_flex()
                        .gap(px(4.))
                        .child(format!(
                            "Lichess cloud: {} at depth {} ·",
                            format_eval(Some(best)),
                            self.cloud
                                .as_ref()
                                .and_then(|cloud| cloud["depth"].as_f64())
                                .unwrap_or_default()
                        ))
                        .child(
                            div()
                                .id("cloud-play")
                                .role(Role::Button)
                                .keyboard_focus(palette.primary)
                                .aria_label(format!("Play {first}"))
                                .underline()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.play_line(&pv, pv.len().saturating_sub(1), cx)
                                }))
                                .child(format!("play {first}")),
                        )
                        .into_any_element()
                }
                None => div()
                    .child(if self.cloud_busy {
                        "Asking Lichess cloud…".to_string()
                    } else {
                        self.cloud_error.clone().unwrap_or_else(|| {
                            "Lichess cloud has no evaluation of this position.".into()
                        })
                    })
                    .into_any_element(),
            };
            h_flex()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(icon("cloud").size(px(12.)))
                .child(text)
        });
        let body: Option<AnyElement> = if !self.engine_ready && !using_cloud {
            Some(
                h_flex()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(
                        div()
                            .id("setup-stockfish")
                            .role(Role::Link)
                            .keyboard_focus(palette.primary)
                            .aria_label("Set up Stockfish")
                            .underline()
                            .cursor_pointer()
                            .on_click(|_, window, cx| {
                                crate::shell::open_settings(
                                    crate::pages::settings::Section::Engine,
                                    window,
                                    cx,
                                )
                            })
                            .child("Set up Stockfish"),
                    )
                    .child("to see evaluations.")
                    .into_any_element(),
            )
        } else if allowed && (engine_on || using_cloud) {
            Some(
                {
                    let page = cx.entity().downgrade();
                    crate::engine_view::engine_lines(
                        &self.node().fen,
                        &lines,
                        if using_cloud { 0 } else { prefs_lines(cx) },
                        over.map(|_| "Nothing to analyse."),
                        std::rc::Rc::new(move |pv: &[String], at, _: &mut Window, cx: &mut App| {
                            let _ = page.update(cx, |page, cx| page.play_line(pv, at, cx));
                        }),
                        palette,
                    )
                }
                .into_any_element(),
            )
        } else {
            None
        };
        v_flex()
            .flex_none()
            .gap(px(8.))
            .child(head)
            .children(cloud_line)
            .children(body)
    }

    /// `.panel-tabs`: the tool under the engine, one at a time.
    fn render_tabs(&self, tool: Tool, palette: &Palette) -> impl IntoElement {
        let tabs = [
            (Tool::Moves, "Moves", icon("list-tree")),
            (Tool::Review, "Review", icon("sparkles")),
            (Tool::Explorer, "Explorer", icon("book-open-text")),
        ];
        h_flex()
            .flex_none()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(9.))
            .bg(palette.accented)
            .children(tabs.into_iter().map(|(choice, label, glyph)| {
                let chosen = choice == tool;
                h_flex()
                    .id(label)
                    .test_support()
                    .role(Role::Tab)
                    .keyboard_focus(palette.primary)
                    .aria_label(label)
                    .aria_selected(chosen)
                    .flex_1()
                    .justify_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(7.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .when(chosen, |tab| {
                        tab.bg(palette.bg).text_color(palette.highlighted)
                    })
                    .when(!chosen, |tab| {
                        tab.text_color(palette.toned)
                            .hover(|tab| tab.text_color(palette.text))
                    })
                    .on_click(move |_, window, cx| {
                        crate::prefs::set(cx, TOOL, choice.key());
                        window.refresh();
                    })
                    .child(glyph.size(px(14.)))
                    .child(label)
            }))
    }

    fn render_moves(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        if self.root.children.is_empty() {
            return div()
                .p(px(16.))
                .text_size(px(13.))
                .text_center()
                .text_color(palette.text_muted)
                .child("Make a move on the board, or import a game. Variations are kept as you explore.")
                .into_any_element();
        }
        let view = TreeView {
            current: &self.path,
            page: cx.entity().downgrade(),
            palette: *palette,
            marks: self.review_marks(cx),
            mainline: {
                let moves = self.mainline();
                (1..=moves.len())
                    .map(|count| path_of(&moves[..count]))
                    .collect()
            },
            reveal: self.reveal_move.clone(),
            scroll: self.moves_scroll.clone(),
        };
        let mut pieces = Vec::new();
        view.line(&self.root, "", None, 0, &mut pieces);
        flow(pieces)
            .text_size(px(13.))
            .line_height(relative(1.9))
            .into_any_element()
    }

    /// First, previous, next and last move, flip, import, and the rest in a menu.
    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let at_start = self.path.is_empty();
        let at_end = self.node().children.is_empty();
        let on_variation = !self.path.is_empty() && !self.root.on_mainline(&self.path);
        // A finished game cannot be played on from its last position.
        let over = game_over(self.position().as_ref()).is_some();
        let page = cx.entity().downgrade();
        let step = |id: &'static str, glyph: &'static str, label: &'static str, disabled: bool| {
            Button::new(id)
                .ghost()
                .small()
                .icon(match glyph {
                    "skip-back" => icon("skip-back"),
                    "chevron-left" => icon("chevron-left"),
                    "chevron-right" => icon("chevron-right"),
                    _ => icon("skip-forward"),
                })
                .tooltip(label)
                .accessibility_label(label)
                .disabled(disabled)
        };
        h_flex()
            .flex_none()
            .justify_center()
            .gap(px(4.))
            .child(
                step("analysis-start", "skip-back", "Go to start", at_start)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_start(cx))),
            )
            .child(
                step("analysis-back", "chevron-left", "Previous move", at_start)
                    .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .child(
                step("analysis-forward", "chevron-right", "Next move", at_end)
                    .on_click(cx.listener(|this, _, _, cx| this.forward(cx))),
            )
            .child(
                step("analysis-end", "skip-forward", "Go to end of line", at_end)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_end(cx))),
            )
            .child(
                Button::new("analysis-flip")
                    .ghost()
                    .small()
                    .icon(icon("arrow-up-down"))
                    .tooltip("Flip board (F)")
                    .accessibility_label("Flip board")
                    .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
            )
            .child(crate::export::export_menu(
                "analysis-export",
                &self.exporter,
                self.game_export(),
                Anchor::BottomRight,
                cx,
            ))
            .child(
                Button::new("analysis-import")
                    .ghost()
                    .small()
                    .icon(icon("import"))
                    .tooltip("Import a PGN or FEN")
                    .accessibility_label("Import")
                    .on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx))),
            )
            .child(
                Button::new("analysis-more")
                    .ghost()
                    .small()
                    .icon(icon("ellipsis"))
                    .accessibility_label("More analysis actions")
                    .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                        let action = |run: fn(
                            &mut AnalysisPage,
                            &mut Window,
                            &mut Context<AnalysisPage>,
                        )| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let _ = page.update(cx, |page, cx| run(page, window, cx));
                            }
                        };
                        // Leaving for another page takes the position and orientation first: the
                        // switch stops this page's engine, which must not happen inside an update
                        // of this page.
                        let leave = |run: fn(String, Color, &mut Window, &mut App)| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(page) = page.upgrade() else { return };
                                let (fen, orientation) = {
                                    let page = page.read(cx);
                                    (page.node().fen.clone(), page.orientation)
                                };
                                run(fen, orientation, window, cx);
                            }
                        };
                        menu.item(PopupMenuItem::new("Copy FEN").icon(icon("copy")).on_click(
                            action(|page, window, cx| {
                                let fen = page.node().fen.clone();
                                page.copy(fen, "FEN", window, cx)
                            }),
                        ))
                        .item(
                            PopupMenuItem::new("Copy PGN")
                                .icon(icon("file-text"))
                                .on_click(action(|page, window, cx| {
                                    let pgn = page.root.to_pgn();
                                    page.copy(pgn, "PGN", window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Import FEN or PGN…")
                                .icon(icon("import"))
                                .on_click(action(|page, window, cx| page.open_import(window, cx))),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Make main line")
                                .icon(icon("arrow-up-to-line"))
                                .disabled(!on_variation)
                                .on_click(action(|page, _, cx| page.promote(cx))),
                        )
                        .item(
                            PopupMenuItem::new("Delete from here")
                                .icon(icon("scissors"))
                                .disabled(at_start)
                                .on_click(action(|page, window, cx| page.ask_remove(window, cx))),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Edit this position")
                                .icon(icon("pencil-ruler"))
                                .on_click(leave(|fen, orientation, window, cx| {
                                    crate::shell::edit_position(fen, orientation, window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Play Stockfish from here")
                                .icon(icon("cpu"))
                                .disabled(over)
                                .on_click(leave(|fen, _, window, cx| {
                                    crate::shell::play_computer_from(fen, window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Play a friend from here")
                                .icon(icon("users-round"))
                                .disabled(over)
                                .on_click(leave(|fen, _, window, cx| {
                                    crate::shell::play_local_from(fen, window, cx)
                                })),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("New analysis")
                                .icon(icon("rotate-ccw"))
                                .on_click(action(|page, _, cx| page.load(None, cx))),
                        )
                    }),
            )
    }
}

impl Render for AnalysisPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_notes(window, cx);
        let palette = *cx.global::<Palette>();
        let tool = Tool::of(&crate::prefs::get::<String>(cx, TOOL, "moves".into()));
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let show_bar = allowed && (Self::engine_on(cx) || using_cloud) && !self.hides_engine();
        let lines = self.lines(cx);
        let position = self.position();
        let result = game_over(position.as_ref()).map(|_| {
            if position
                .as_ref()
                .is_some_and(|position| position.is_checkmate())
            {
                if position.as_ref().map(|position| position.turn()) == Some(Color::White) {
                    "0-1"
                } else {
                    "1-0"
                }
            } else {
                "½-½"
            }
        });
        // The column always keeps the eval bar's width beside the board.
        // The column holds the board and the move box under it (`.analysis-surface`).
        let board_size = self.layout.board_size_around(
            EVAL_BAR + EVAL_GAP,
            STUDY_BAR,
            widgets::STACK_GAP + px(36.),
            window,
        );
        let entry = widgets::move_entry(&self.move_input, true, &palette).h(px(36.));
        let voice = board_bar(
            &self.voice,
            cx.entity().downgrade(),
            Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| {
                    this.play(uci, cx);
                },
            },
            BoardBar {
                id: "analysis-voice",
                hint: "Say a move like “Knight F three”, or “back”, “next”, “best move”.",
                active: true,
                listening: true,
                grammar: crate::voice::grammar("ANALYSIS_GRAMMAR"),
            },
            cx,
        );
        let board_column = v_flex()
            .w(board_size + EVAL_BAR + EVAL_GAP)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(
                h_flex()
                    .items_stretch()
                    .gap(EVAL_GAP)
                    .when(show_bar, |row| {
                        row.child(crate::engine_view::eval_bar(
                            lines.first(),
                            self.orientation,
                            result,
                        ))
                    })
                    .when(!show_bar, |row| row.pl(EVAL_BAR))
                    .child(
                        div()
                            .id("analysis-board")
                            .w(board_size)
                            .on_scroll_wheel(cx.listener(Self::scroll))
                            // The board takes the keys back from the move box, as a click
                            // outside a text field does in the browser.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                            )
                            .child(crate::board::cached(&self.board)),
                    ),
            )
            .child(
                v_flex()
                    .pl(EVAL_BAR + EVAL_GAP)
                    .gap(px(4.))
                    .child(widgets::input_row(entry, voice))
                    .children(self.move_message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    })),
            );
        let tool_body: AnyElement = match tool {
            // Never shorter than its list's minimum (`.moves-tool { min-height: auto }`).
            Tool::Moves => v_flex()
                .flex_1()
                .gap(px(8.))
                .child(
                    div()
                        .id("analysis-moves")
                        .flex_1()
                        // Under the board the list has a height of its own (`.moves-wrap`).
                        .min_h(if self.layout.is_stacked() {
                            px(240.)
                        } else {
                            px(96.)
                        })
                        .overflow_y_scroll()
                        .track_scroll(&self.moves_scroll)
                        .p(px(8.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(px(10.))
                        .bg(palette.bg)
                        // Away from any move, the list's menu copies the game.
                        .context_menu({
                            let page = cx.entity().downgrade();
                            move |menu, _, _| {
                                let page = page.clone();
                                menu.item(
                                    PopupMenuItem::new("Copy PGN")
                                        .icon(icon("file-text"))
                                        .on_click(move |_, window, cx| {
                                            let _ = page.update(cx, |page, cx| {
                                                let pgn = page.root.to_pgn();
                                                page.copy(pgn, "PGN", window, cx)
                                            });
                                        }),
                                )
                            }
                        })
                        .child(self.render_moves(cx, &palette)),
                )
                .child(self.render_notes(cx, &palette))
                .into_any_element(),
            Tool::Review => {
                let note = |text: &'static str| {
                    div()
                        .px(px(8.))
                        .py(px(24.))
                        .text_size(px(13.))
                        .text_center()
                        .text_color(palette.text_muted)
                        .child(text)
                        .into_any_element()
                };
                let body = if !allowed {
                    note("Engine assistance is paused until Lichess game status is verified.")
                } else if self.root.children.is_empty() {
                    note("Make some moves or import a game to review it.")
                } else {
                    self.render_review(cx, &palette)
                };
                div()
                    .id("analysis-review")
                    .flex_1()
                    .min_h(px(96.))
                    .overflow_y_scroll()
                    .child(body)
                    .into_any_element()
            }
            Tool::Explorer => {
                let note = if self.hides_engine() {
                    Some("Shown once you find the move.")
                } else if !allowed {
                    Some("Engine assistance is paused until Lichess game status is verified.")
                } else {
                    None
                };
                match note {
                    Some(note) => div()
                        .flex_1()
                        .px(px(8.))
                        .py(px(24.))
                        .text_size(px(13.))
                        .text_center()
                        .text_color(palette.text_muted)
                        .child(note)
                        .into_any_element(),
                    None => {
                        let (fen, opened) = (self.node().fen.clone(), !self.explorer_open);
                        self.explorer
                            .update(cx, |explorer, cx| explorer.show(&fen, None, opened, cx));
                        div()
                            .id("analysis-explorer")
                            .flex_1()
                            .min_h(px(96.))
                            .overflow_y_scroll()
                            .child(self.explorer.clone())
                            .into_any_element()
                    }
                }
            }
        };
        self.explorer_open = tool == Tool::Explorer;
        // A click in the panel gives the keys back to the board, before a text field in it
        // (a comment, the explorer's player) takes them for itself.
        let panel = widgets::side_panel(&palette)
            // A short window scrolls the panel rather than let the move list cover the controls.
            .id("analysis-panel")
            .overflow_y_scroll()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left {
                    window.focus(&this.focus, cx);
                }
            }))
            .h(self.layout.fixed(board_size + px(36.) + widgets::STACK_GAP))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(12.))
            .when(self.practice.is_some(), |panel| {
                panel.child(self.render_practice(cx, &palette))
            })
            .when(self.practice.is_none(), |panel| {
                panel.child(self.render_engine(cx, &palette))
            })
            .child(self.render_tabs(tool, &palette))
            .child(tool_body)
            .child(self.render_controls(cx));
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .child(self.layout.render_under(
                "analysis",
                self.study_bar.clone(),
                board_column,
                panel,
                window,
                cx,
            ))
    }
}
