//! The analysis board: a move tree explored on the
//! board, Stockfish's lines and arrows for the position shown, Lichess's cloud evaluation, and the
//! session the profile keeps (`sessions.analysis`), so it opens as it was left.

mod edits;
mod engine;
mod import;
mod notes;
mod practice;
mod review;
mod study;
mod tree_view;

use engine::{prefs_lines, stop_analysis};
use kchess_contracts::calls;
use tree_view::{TreeView, flow};

pub use review::classification_look;

use crate::voice::moves::{Access, Action, BoardBar, Reply, Turn, VoiceMoves, board_bar};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::Duration;

use futures::FutureExt as _;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState, Redo, Textarea, TextareaState, Undo};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
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
use crate::part::Part;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, panel_style};
use crate::reviews::ReviewHandle;
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::studies::StudyHandle;
use crate::theme::{Palette, radii, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;
use crate::widgets::Pressable as _;

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

/// A position of the tree, read by its variant's rules (a Lichess key).
fn position_of(variant: &str, fen: &str) -> Option<VariantPosition> {
    kchess_domain::rules::setup_start(variant, fen)
}

/// The position with the other side to move (a null move), whose best line is the threat; `None`
/// in check, where the side to move cannot pass.
fn threat_fen(variant: &str, fen: &str) -> Option<String> {
    let position = position_of(variant, fen)?;
    if position.is_check() {
        return None;
    }
    let passed = position.swap_turn().ok()?;
    Some(kchess_domain::rules::make_fen(&passed))
}

/// Checkmate, stalemate, bare kings or a variant's own ending (an exploded king, a king on the
/// hill…) end the analysis: there is nothing to search.
fn game_over(position: Option<&VariantPosition>) -> Option<&'static str> {
    let position = position?;
    let info = kchess_domain::position::info(position);
    if info.variant_end {
        Some(match position {
            VariantPosition::Atomic(_) => "King exploded",
            VariantPosition::KingOfTheHill(_) => "King reached the centre",
            VariantPosition::ThreeCheck(_) => "Third check",
            VariantPosition::RacingKings(_) => "King reached the last rank",
            VariantPosition::Antichess(_) => "No pieces left",
            VariantPosition::Horde(_) => "The horde is captured",
            _ => "Game over",
        })
    } else if info.checkmate {
        Some("Checkmate")
    } else if info.stalemate {
        Some("Stalemate")
    } else if info.insufficient {
        Some("Insufficient material")
    } else {
        None
    }
}

/// The result of a position that ends the game: "1-0", "0-1" or "½-½".
fn result_of(position: &VariantPosition) -> &'static str {
    let info = kchess_domain::position::info(position);
    match info
        .outcome
        .as_ref()
        .and_then(|outcome| outcome["winner"].as_str())
    {
        Some("white") => "1-0",
        Some("black") => "0-1",
        _ => "½-½",
    }
}

pub struct AnalysisPage {
    root: TreeNode,
    path: String,
    /// Undo and Redo over the tree's edits.
    edits: edits::Edits,
    /// The move whose comment the last change typed, so a comment's typing is one step.
    typing_comment: Option<String>,
    /// The threat is shown (X): the engine searches the position with the other side to move.
    threat: bool,
    orientation: Color,
    /// The saved study the board is showing, and its chapter; edits are saved back to it.
    study: String,
    chapter: String,
    /// The PGN last written to the open study, so opening one does not count as an edit.
    study_pgn: String,
    /// Why the study could not be saved (or a recovered session that differs from it).
    study_error: Option<String>,
    study_save: Option<Task<()>>,
    /// The chapter write not yet answered, and its number: the text counts as saved meanwhile,
    /// so a flush waits for it rather than for nothing.
    study_writing: Option<(u64, StudySave)>,
    study_writes: u64,
    /// Bumped whenever another document takes the board, so a late save-as does not link it.
    navigation: u64,
    study_bar: Entity<StudyBar>,
    /// The main line's game review, and practising its mistakes.
    review: review::ReviewState,
    /// The explanation of the review's move on the board.
    explainer: Entity<crate::ai::Explainer>,
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
    /// The panel beside the board, drawn as a cached view of its own (`render_panel`).
    panel: Entity<Part<AnalysisPage>>,
    exporter: Entity<Exporter>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    lines_choice: Entity<SelectState<Vec<SharedString>>>,
    import: Entity<TextareaState>,
    import_error: Option<SharedString>,
    /// A Lichess game being fetched for the import, by its turn: a later import replaces it.
    importing: Option<u64>,
    import_turn: u64,
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
    wheel: crate::game::view::Wheel,
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
        // Its session and the drills alone: the whole library replays every played game.
        let bridge = &cx.global::<AppCore>().bridge;
        let session = bridge
            .request_now(calls::Session {
                kind: json!("analysis"),
            })
            .unwrap_or(Value::Null);
        let mistakes = bridge
            .request_now(calls::Mistakes {})
            .ok()
            .and_then(|mistakes| mistakes.as_array().cloned())
            .unwrap_or_default();
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
                &position_of("standard", STANDARD_FEN).unwrap_or_default(),
                orientation,
                None,
                None,
            ))
        });
        let move_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
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
        let edits = edits::Edits::new(&root, &path);
        let mut page = AnalysisPage {
            root,
            path,
            edits,
            typing_comment: None,
            threat: false,
            orientation,
            study,
            chapter,
            study_pgn,
            study_error,
            study_save: None,
            study_writing: None,
            study_writes: 0,
            navigation: 0,
            study_bar,
            review: review::ReviewState::new(mistakes),
            explainer: cx.new(|_| crate::ai::Explainer::new()),
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
            panel: Part::panel(|this, window, cx| this.render_panel(window, cx), cx),
            exporter: cx.new(|_| Exporter::default()),
            board,
            layout: PlayLayout::default(),
            move_input,
            move_message: None,
            lines_choice,
            import,
            import_error: None,
            importing: None,
            import_turn: 0,
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
            wheel: Default::default(),
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

    /// The FEN of the position shown.
    #[cfg(test)]
    pub(crate) fn shown_fen(&self) -> String {
        self.node().fen.clone()
    }

    /// The engine's status line.
    #[cfg(test)]
    pub(crate) fn engine_text(&self, cx: &App) -> String {
        self.engine_status(cx)
    }

    /// The review shown, once it covers the whole main line and is finished, with its summary.
    #[cfg(test)]
    pub(crate) fn finished_review(&self, cx: &App) -> Option<(Value, Value)> {
        let review = self.review(cx).filter(|review| {
            review["complete"] == json!(true) && review["key"] == json!(self.mainline_key())
        })?;
        let (analysis, _) = self.game_analysis(cx)?;
        Some((review, analysis))
    }

    fn position(&self) -> Option<VariantPosition> {
        position_of(self.root.variant(), &self.node().fen)
    }

    /// Whether the game on the board is played by standard rules (standard chess or Chess960):
    /// cloud evaluation and the editor are theirs.
    fn standard_rules(&self) -> bool {
        crate::game::standard_rules(self.root.variant())
    }

    /// The position shown, as a game to play from: in the game's variant, or (standard chess)
    /// castling as Chess960 when its rooks need it.
    fn play_setup(&self) -> Value {
        let fen = self.node().fen.clone();
        match self.fairy_variant() {
            Some(variant) => json!({ "variant": variant, "fen": fen }),
            None => crate::game::setup_from_fen(&fen),
        }
    }

    /// The Lichess key of the game's variant when Fairy-Stockfish analyses and reviews it.
    pub(super) fn fairy_variant(&self) -> Option<&'static str> {
        (!self.standard_rules()).then(|| self.root.variant())
    }

    fn context(&self) -> String {
        analysis_context(&self.root.fen, &moves_of(&self.path))
    }

    /// The position the threat is searched from, while the threat is shown and the side to move
    /// could pass.
    fn threat_position(&self) -> Option<String> {
        self.threat
            .then(|| threat_fen(self.root.variant(), &self.node().fen))
            .flatten()
    }

    /// The position the engine searches: the one shown, or its threat's.
    fn engine_fen(&self) -> String {
        self.threat_position()
            .unwrap_or_else(|| self.node().fen.clone())
    }

    /// The engine's search and its cache entry: the threat's position stands on its own.
    fn engine_context(&self) -> String {
        match self.threat_position() {
            Some(fen) => analysis_context(&fen, &[]),
            None => self.context(),
        }
    }

    /// Show or hide the opponent's threat (X, as on Lichess).
    pub(crate) fn toggle_threat(&mut self, cx: &mut Context<Self>) {
        if self.hides_engine() {
            return;
        }
        self.threat = !self.threat;
        self.engine_changed(cx);
    }

    #[cfg(test)]
    pub(crate) fn threat_shown(&self) -> Option<String> {
        self.threat_position()
    }

    /// The delayed save of the open study's chapter, run now.
    #[cfg(test)]
    pub(crate) fn autosave_for_test(&mut self, cx: &mut Context<Self>) {
        self.persist_study(cx);
    }

    /// A move on the board, as a click would make it.
    #[cfg(test)]
    pub(crate) fn play_for_test(&mut self, uci: &str, cx: &mut Context<Self>) -> bool {
        self.play(uci, cx)
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

    /// Into the first variation from here (Shift →), as Lichess's analysis board does.
    fn enter_variation(&mut self, cx: &mut Context<Self>) {
        if let Some(child) = self.node().children.get(1) {
            let next = child_path(&self.path, &child.uci);
            self.go_to(&next, cx);
        }
    }

    /// Out of the variation shown, back to where it branched off (Shift ←).
    fn exit_variation(&mut self, cx: &mut Context<Self>) {
        if let Some(fork) = self.root.fork_before(&self.path) {
            self.go_to(&fork, cx);
        }
    }

    /// E shows the explorer, or the moves again.
    fn toggle_explorer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = Tool::of(&crate::prefs::get::<String>(cx, TOOL, "moves".into()));
        let next = if shown == Tool::Explorer {
            Tool::Moves
        } else {
            Tool::Explorer
        };
        crate::prefs::set(cx, TOOL, next.key());
        window.refresh();
        cx.notify();
    }

    pub(crate) fn go_to_start(&mut self, cx: &mut Context<Self>) {
        self.go_to("", cx);
    }

    fn go_to_end(&mut self, cx: &mut Context<Self>) {
        let end = self.root.line_end(&self.path);
        self.go_to(&end, cx);
    }

    pub(crate) fn flip(&mut self, cx: &mut Context<Self>) {
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
    /// The board editor's position (`{ variant, fen }`), in its variant. False when the board
    /// cannot take it.
    pub fn analyse_setup(
        &mut self,
        setup: &Value,
        orientation: Color,
        cx: &mut Context<Self>,
    ) -> bool {
        let variant = setup["variant"].as_str().unwrap_or("standard");
        let fen = setup["fen"].as_str().unwrap_or(STANDARD_FEN);
        if crate::game::standard_rules(variant) {
            self.orientation = orientation;
            self.load(Some(fen), cx);
            return true;
        }
        // Another variant opens as a game of it from that position.
        let pgn = format!(
            "[Variant \"{}\"]\n[FEN \"{fen}\"]\n[SetUp \"1\"]\n\n*",
            crate::game::variant_label(variant)
        );
        let loaded = self.load_pgn(&pgn, cx);
        if loaded {
            self.orientation = orientation;
        }
        loaded
    }

    /// Load a game (`loadPgn`), opened at its start; refused for a variant this board cannot play.
    fn load_pgn(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !crate::game::pgn_analysable(text) {
            return false;
        }
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
        self.edits.reset(&self.root, "");
        self.clear_evaluations();
        // The study is let go whole, its chapter and saved text with it.
        self.study.clear();
        self.chapter.clear();
        self.study_pgn.clear();
        self.review.origin = None;
    }

    /* ── Studies ──────────────────────────────────────────────────────── */

    /// The whole tree as PGN (`analysis.pgn()`).
    pub fn pgn(&self) -> String {
        self.root.to_pgn()
    }

    /// The bar over the board: the study open, or Save as study.
    pub(crate) fn study_bar(&self) -> Entity<StudyBar> {
        self.study_bar.clone()
    }

    pub fn study_id(&self) -> &str {
        &self.study
    }

    pub fn chapter_id(&self) -> &str {
        &self.chapter
    }

    /// Whether the board holds edits its study has not been sent yet.
    pub fn unsaved_study_edits(&self) -> bool {
        !self.study.is_empty() && self.root.to_pgn() != self.study_pgn
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

    pub fn navigation(&self) -> u64 {
        self.navigation
    }

    /// The board holds moves no study keeps (`useReplaceAnalysis`), so replacing it asks first.
    pub fn unsaved(&self) -> bool {
        self.study.is_empty() && !self.root.children.is_empty()
    }

    /// Load a line from the starting position, opened at its end (an opening's Analyse).
    pub fn load_line(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let pgn = kchess_domain::pgn::pgn_from_uci(moves);
        if self.load_pgn(&pgn, cx) {
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
        let setup = json!({ "variant": self.root.variant(), "fen": self.node().fen });
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
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3".into());
                cx.notify();
            }
        }
    }

    /// The board, the engine and the saved session follow a change.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let typing = self.typing_comment.take();
        self.edits.track(&self.root, &self.path, typing);
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
        let save = cx.global::<AppCore>().request(calls::SaveSession {
            kind: json!("analysis"),
            session,
        });
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

    /// The board shows the position, its last move and the engine's arrows.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let node = self.node();
        let position = position_of(self.root.variant(), &node.fen).unwrap_or_default();
        let last_move = now_square(&node.uci, 0..2).zip(now_square(&node.uci, 2..4));
        let mut state = BoardState::of(
            &position,
            self.orientation,
            Some(position.turn()),
            last_move,
        );
        // The review's label on the move just played; without one, checkmate's "#".
        let marks = self.review_marks(cx);
        let mate = state.badge.take();
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
            })
            .or(mate);
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
        let threat = self.threat_position().is_some();
        if engine_shown {
            state.arrows = self
                .lines(cx)
                .iter()
                .enumerate()
                // The threat is the opponent's best move, in red.
                .take(if threat { 1 } else { usize::MAX })
                .filter_map(|(rank, line)| {
                    let uci = line["pv"][0].as_str()?;
                    let (from, to) = (now_square(uci, 0..2)?, now_square(uci, 2..4)?);
                    Some(if threat {
                        Arrow::brush("red", from, to)
                    } else {
                        Arrow::engine(from, to, rank)
                    })
                })
                .collect();
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /* ── Keys, wheel, menus ───────────────────────────────────────────── */

    /// ← → ↑ ↓ (and Home, End) step, Shift → and Shift ← go into and out of a variation, F flips,
    /// L turns the engine on or off, E shows the explorer, Space plays the best move, Delete or
    /// Backspace deletes from the move shown (its notice's Undo, or ⌘Z, brings it back).
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::board::page_has_keys(&self.focus, window, cx) {
            return;
        }
        let modifiers = &event.keystroke.modifiers;
        if modifiers.shift && !modifiers.control && !modifiers.platform && !modifiers.alt {
            match event.keystroke.key.as_str() {
                "right" => self.enter_variation(cx),
                "left" => self.exit_variation(cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" => self.back(cx),
            "right" => self.forward(cx),
            "up" | "home" => self.go_to_start(cx),
            "down" | "end" => self.go_to_end(cx),
            "f" => self.flip(cx),
            "l" => self.toggle_engine(cx),
            "x" => self.toggle_threat(cx),
            "c" => self.focus_notes(window, cx),
            "e" => self.toggle_explorer(window, cx),
            "delete" | "backspace" => self.delete_from_here(window, cx),
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
            crate::voice::grammar(crate::voice::Grammar::Analysis),
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
        let setup = json!({ "variant": self.root.variant(), "fen": fen });
        // The threat's best move is the opponent's: "best move" waits for the position's own.
        let best = (Self::assistance_allowed(cx)
            && !self.hides_engine()
            && self.threat_position().is_none())
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
                let said = kchess_domain::voice::analysis_command(text)?.to_string();
                let feedback = if said == "best" && best.is_none() {
                    "The engine has no move yet".to_string()
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
    fn scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        match self
            .wheel
            .turn(event.delta.pixel_delta(window.line_height()))
        {
            Some("right") => self.forward(cx),
            Some(_) => self.back(cx),
            None => return,
        }
        cx.stop_propagation();
    }

    /// The game an Export saves: the document with its variations and comments, its main line
    /// for the GIF, and the position shown (in a variation too) for the PNG.
    fn game_export(&self) -> GameExport {
        let mainline = self.mainline();
        let depth = if self.path.is_empty() {
            0
        } else {
            self.path.split(' ').count()
        };
        GameExport {
            variant: if self.standard_rules() {
                crate::game::setup_from_fen(&self.root.fen)["variant"]
                    .as_str()
                    .unwrap_or("standard")
                    .into()
            } else {
                self.root.variant().into()
            },
            fen: self.root.fen.clone(),
            ply: Some(depth.min(mainline.len())),
            moves: mainline,
            orientation: self.orientation,
            white: self.player_name(Color::White),
            black: self.player_name(Color::Black),
            result: Some(self.header("Result")).filter(|result| !result.is_empty()),
            // A Chess.com game has no page on Lichess: it is uploaded like any other game.
            lichess_id: self
                .review
                .origin
                .as_ref()
                .and_then(|origin| origin["gameId"].as_str())
                .filter(|id| !kchess_domain::chesscom::is_game_id(id))
                .map(str::to_string),
            pgn: Some(self.root.to_pgn()),
            shown: Some(moves_of(&self.path)),
        }
    }

    /// Copy the position shown.
    pub(crate) fn copy_fen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fen = self.node().fen.clone();
        self.copy(fen, "FEN", window, cx);
    }

    /// Copy the whole game, its variations and comments with it.
    pub(crate) fn copy_pgn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pgn = self.root.to_pgn();
        self.copy(pgn, "PGN", window, cx);
    }

    /// Copy the moves to the move shown (Quick Pick).
    pub(crate) fn copy_shown_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.copy_line(&path, window, cx);
    }

    /// Copy the moves from the start to `path` as PGN, without the side lines.
    fn copy_line(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let pgn = self.root.line_to(path).to_pgn();
        self.copy(pgn, "Line", window, cx);
    }

    fn copy(&self, text: String, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(SharedString::from(format!("{what} copied")), cx);
    }

    /// Cut the move shown and what follows it at once; the notice's Undo (or ⌘Z) brings it back.
    fn delete_from_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.path.clone();
        if at.is_empty() {
            return;
        }
        self.remove(&at, cx);
        let page = cx.entity().downgrade();
        window.push_notification(
            widgets::action_notification(
                Notification::new().title("Moves deleted"),
                "undo-delete-moves",
                "Undo",
                move |window, cx| {
                    let _ = page.update(cx, |page, cx| page.undo(window, cx));
                },
            ),
            cx,
        );
    }

    /// Take back the last edit to the tree (⌘Z, Edit → Undo).
    pub(crate) fn undo(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.edits.undo() {
            self.restore(snapshot, cx);
        }
    }

    /// Make the last edit undone again (⇧⌘Z, Edit → Redo).
    pub(crate) fn redo(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.edits.redo() {
            self.restore(snapshot, cx);
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.edits.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.edits.can_redo()
    }

    /// Show a tree from the edit history, at the move it was left on.
    fn restore(&mut self, snapshot: edits::Snapshot, cx: &mut Context<Self>) {
        self.root = snapshot.root;
        self.path = self.root.existing_path(&snapshot.path);
        // The comment field shows the restored tree's comment.
        self.notes.filled_for = None;
        self.changed(cx);
        cx.notify();
    }
}

/* ── Rendering ────────────────────────────────────────────────────────── */

impl AnalysisPage {
    /// `.panel-tabs`: the tool under the engine, one at a time.
    fn render_tabs(&self, tool: Tool, palette: &Palette) -> impl IntoElement {
        let tabs = [
            (Tool::Moves, "Moves", icon("list-tree")),
            (Tool::Review, "Review", icon("chart-line")),
            (Tool::Explorer, "Explorer", icon("book-open-text")),
        ];
        h_flex()
            .flex_none()
            .gap(px(4.))
            .p(px(2.))
            .rounded(radii::MD)
            .border_1()
            .border_color(palette.border)
            .bg(palette.bg)
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
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(radii::SM)
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(chosen, |tab| {
                        tab.bg(palette.accented).text_color(palette.highlighted)
                    })
                    .when(!chosen, |tab| {
                        tab.text_color(palette.text_muted)
                            .pressable_text(palette.highlighted)
                    })
                    .on_click(move |_, window, cx| {
                        crate::prefs::set(cx, TOOL, choice.key());
                        window.refresh();
                    })
                    .child(glyph.size(px(16.)))
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
                .child("Make a move on the board, or import a game")
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
            figurine: crate::notation::figurine(cx),
        };
        let mut pieces = Vec::new();
        view.line(&self.root, "", None, 0, &mut pieces);
        v_flex()
            .children(self.opening(cx).map(|opening| {
                div()
                    .id("analysis-opening")
                    .test_support()
                    .px(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.text_muted)
                    .aria_label(opening.clone())
                    .child(opening)
            }))
            .child(flow(pieces).text_size(px(13.)).line_height(relative(1.9)))
            .into_any_element()
    }

    /// The opening the moves up to the one shown reached, as the game pages name it
    /// (`openingAt`): only in standard chess, and while Settings show opening names.
    fn opening(&self, cx: &App) -> Option<String> {
        let shown = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .show_opening_name;
        if !shown || self.root.variant() != "standard" {
            return None;
        }
        let moves = moves_of(&self.path);
        let opening = kchess_domain::training::openings::opening_at(
            "standard",
            &self.root.fen,
            &moves,
            moves.len() as f64,
        )?;
        Some(format!("{} {}", opening.eco, opening.name))
    }

    /// First, previous, next and last move, flip, import, and the rest in a menu.
    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let at_start = self.path.is_empty();
        let (can_undo, can_redo) = (self.can_undo(), self.can_redo());
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
                false,
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
                        let leave = |run: fn(Value, Color, &mut Window, &mut App)| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(page) = page.upgrade() else { return };
                                let (setup, orientation) = {
                                    let page = page.read(cx);
                                    (page.play_setup(), page.orientation)
                                };
                                run(setup, orientation, window, cx);
                            }
                        };
                        // The same, with the position's setup: its variant and FEN.
                        let play = |run: fn(Value, &mut Window, &mut App)| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(page) = page.upgrade() else { return };
                                let setup = page.read(cx).play_setup();
                                run(setup, window, cx);
                            }
                        };
                        menu.item(
                            PopupMenuItem::new("Undo")
                                .icon(icon("undo-2"))
                                .disabled(!can_undo)
                                .on_click(action(|page, window, cx| page.undo(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Redo")
                                .icon(icon("redo-2"))
                                .disabled(!can_redo)
                                .on_click(action(|page, window, cx| page.redo(window, cx))),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Copy FEN")
                                .icon(icon("copy"))
                                .on_click(action(|page, window, cx| page.copy_fen(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Copy PGN")
                                .icon(icon("file-text"))
                                .on_click(action(|page, window, cx| page.copy_pgn(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Copy line")
                                .icon(icon("list-tree"))
                                .disabled(at_start)
                                .on_click(action(|page, window, cx| {
                                    let path = page.path.clone();
                                    page.copy_line(&path, window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Import a position or game…")
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
                                .on_click(action(|page, window, cx| {
                                    page.delete_from_here(window, cx)
                                })),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Edit this position")
                                .icon(icon("pencil-ruler"))
                                .on_click(leave(|setup, orientation, window, cx| {
                                    crate::shell::edit_position(setup, orientation, window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Play Stockfish from here")
                                .icon(icon("cpu"))
                                .disabled(over)
                                .on_click(play(crate::shell::play_computer_from)),
                        )
                        .item(
                            PopupMenuItem::new("Play a friend from here")
                                .icon(icon("users-round"))
                                .disabled(over)
                                .on_click(play(crate::shell::play_local_from)),
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

impl AnalysisPage {
    /// The tools beside (or under) the board: the engine or practice, the tabs and the tool
    /// shown, and the controls. Beside the board it is the page's `Part`, so a board animation
    /// or the move box redrawing reuses it.
    fn render_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let tool = Tool::of(&crate::prefs::get::<String>(cx, TOOL, "moves".into()));
        let allowed = Self::assistance_allowed(cx);
        let board_size = self.board_size(window);
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
                        .rounded(radii::LG)
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
                    note("Engine assistance is paused until Lichess game status is verified")
                } else if self.root.children.is_empty() {
                    note("Make some moves or import a game to review it")
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
                    Some("Shown once you find the move")
                } else if !allowed {
                    Some("Engine assistance is paused until Lichess game status is verified")
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
                        let variant = self.root.variant().to_string();
                        self.explorer.update(cx, |explorer, cx| {
                            explorer.show(Some(&variant), &fen, None, opened, cx)
                        });
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
        widgets::side_panel(&palette)
            // A short window scrolls the panel rather than let the move list cover the controls.
            .id("analysis-panel")
            .overflow_y_scroll()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left {
                    window.focus(&this.focus, cx);
                }
            }))
            .h(self.layout.fixed(self.panel_height(board_size)))
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
            .child(self.render_controls(cx))
            .into_any_element()
    }

    /// The panel's height beside a board of `board_size`: the board and the move box under it.
    fn panel_height(&self, board_size: Pixels) -> Pixels {
        board_size + px(36.) + widgets::STACK_GAP
    }

    /// The board's size in the page, the eval bar beside it and the study bar above.
    fn board_size(&self, window: &Window) -> Pixels {
        self.layout.board_size_around(
            EVAL_BAR + EVAL_GAP,
            STUDY_BAR,
            widgets::STACK_GAP + px(36.),
            window,
        )
    }
}

impl Render for AnalysisPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_notes(window, cx);
        let palette = *cx.global::<Palette>();
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let show_bar = allowed && (Self::engine_on(cx) || using_cloud) && !self.hides_engine();
        let bar_line = self.bar_line(cx);
        let position = self.position();
        let result = position
            .as_ref()
            .filter(|position| game_over(Some(position)).is_some())
            .map(result_of);
        // The column always keeps the eval bar's width beside the board.
        // The column holds the board and the move box under it (`.analysis-surface`).
        let board_size = self.board_size(window);
        let entry = widgets::move_entry(&self.move_input, true, &palette);
        let (voice, voice_line) = board_bar(
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
                hint: "Say a move like “Knight F three”, or “back”, “next”, “best move”",
                active: true,
                listening: true,
                grammar: crate::voice::grammar(crate::voice::Grammar::Analysis),
            },
            cx,
        );
        // A Crazyhouse game's pockets: the far side's above the board, the near side's below.
        let pocket = |color: Color| {
            crate::board::pocket(&self.board, color, cx)
                .map(|pocket| div().pl(EVAL_BAR + EVAL_GAP).child(pocket))
        };
        let (top_pocket, bottom_pocket) = (pocket(!self.orientation), pocket(self.orientation));
        let board_column = v_flex()
            .w(board_size + EVAL_BAR + EVAL_GAP)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .children(top_pocket)
            .child(
                h_flex()
                    .items_stretch()
                    .gap(EVAL_GAP)
                    .when(show_bar, |row| {
                        // The bar ends where the squares end, above any outside coordinates.
                        row.child(div().pb(crate::board::outside_gutter(cx)).child(
                            crate::engine_view::eval_bar(
                                bar_line.as_ref(),
                                self.orientation,
                                result,
                            ),
                        ))
                    })
                    .when(!show_bar, |row| row.pl(EVAL_BAR))
                    .child(
                        div()
                            .id("analysis-board")
                            .w(board_size)
                            .on_scroll(cx.listener(Self::scroll))
                            // The board takes the keys back from the move box, as a click
                            // outside a text field does in the browser.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                            )
                            .child(crate::board::cached(&self.board)),
                    ),
            )
            .children(bottom_pocket)
            .child(
                v_flex()
                    .pl(EVAL_BAR + EVAL_GAP)
                    .gap(px(4.))
                    .child(widgets::input_row(entry, voice))
                    .children(voice_line)
                    .children(self.move_message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    })),
            );
        let panel = if self.layout.is_stacked() {
            self.render_panel(window, cx)
        } else {
            self.panel
                .clone()
                .cached(panel_style(self.panel_height(board_size), PANEL_MAX))
                .into_any_element()
        };
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            // A text field takes these for its own text first.
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
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
