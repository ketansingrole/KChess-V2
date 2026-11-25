//! The analysis board: a move tree explored on the
//! board, Stockfish's lines and arrows for the position shown, Lichess's cloud evaluation, and the
//! session the profile keeps (`sessions.analysis`), so it opens as it was left.

mod edits;
mod engine;
mod import;
mod load;
mod notes;
mod novelty;
mod opening_report;
mod page;
mod practice;
mod review;
mod study;
mod tree_view;
mod view;

use crate::game::surface::TakesKeys as _;
use crate::game::tree::TreePgn as _;
use crate::shortcuts::{PageKey, page_key};
use engine::prefs_lines;
use kchess_contracts::calls;
use kchess_contracts::types::AnalysisUpdate;
use tree_view::{TreeView, flow};

/// What the analysis board's own keys do.
#[derive(Clone, Copy)]
pub(crate) enum AnalysisKey {
    Engine,
    Threat,
    Best,
    Comment,
    Explorer,
    Delete,
}

/// The analysis board's own keys, which the shortcuts list shows.
pub(crate) const KEYS: &[PageKey<AnalysisKey>] = &[
    PageKey {
        action: AnalysisKey::Engine,
        keys: &["l"],
        shown: "L",
        does: "Engine on or off",
    },
    PageKey {
        action: AnalysisKey::Threat,
        keys: &["x"],
        shown: "X",
        does: "Show threat",
    },
    PageKey {
        action: AnalysisKey::Best,
        keys: &["space"],
        shown: "Space",
        does: "Play the best move",
    },
    PageKey {
        action: AnalysisKey::Comment,
        keys: &["c"],
        shown: "C",
        does: "Comment",
    },
    PageKey {
        action: AnalysisKey::Explorer,
        keys: &["e"],
        shown: "E",
        does: "Explorer",
    },
    PageKey {
        action: AnalysisKey::Delete,
        keys: &["delete", "backspace"],
        shown: "Delete",
        does: "Delete from the move shown",
    },
];

#[cfg(test)]
pub use load::board;
pub use load::{init, new_board, open_linked, open_study, replace};
pub use review::Opening;

use crate::voice::moves::{Access, Action, BoardBar, Reply, Turn, VoiceMoves, board_bar};
use kchess_domain::voice::AnalysisCommand;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::Duration;

use futures::FutureExt as _;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Redo, Textarea, TextareaState, Undo};
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
use kchess_contracts::{CoreError, Level};
use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position as _, Square};

use crate::assets::icon;
use crate::board::{Arrow, Badge, BoardEvent, BoardState, ChessBoard};
use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::engine_view::EVAL_BAR;
use crate::explorer::{ExplorerEvent, PositionExplorer};
use crate::export::Exporter;
use crate::game::STANDARD_FEN;
use crate::game::played::{DatabaseGame, PlayedGame};
use crate::game::session::{Resumes, SAVE_DELAY, SessionSaver};
use crate::game::surface::Steps;
use crate::game::tree::{
    TreeNode, analysis_context, child_path, comment_text, format_eval, move_glyph, move_number,
    moves_of, path_of, pv_san,
};
use crate::game::view::Way;
use crate::game_db::GameDbHandle;
use crate::online::OnlineHandle;
use crate::pages::study_bar::{STUDY_BAR, StudyBar};
use crate::part::Part;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, panel_style};
use crate::quit::BoxedSave;
use crate::reviews::ReviewHandle;
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::studies::StudyHandle;
use crate::theme::{Palette, radii, tint};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;
use crate::widgets::Pressable as _;
use kchess_contracts::types::{AnalysisSession, SessionDocument, SessionKind, Sessions};
use kchess_domain::variant::{Setup, Variant};

/// Why the board left its study: another window changed the chapter it was saving to.
const STALE_COPY: &str = "This study was changed in another KChess window. Your board is kept as an unsaved copy; save it under a new name or reopen the study.";
const CONFLICT: &str = "Recovered session differs from the saved study. It is kept as an unsaved copy; save it under a new name or reopen the saved study.";

/// A study write's answer, which the quit and the page's own handling both wait on.
type StudySave = futures::future::Shared<
    std::pin::Pin<
        Box<dyn Future<Output = Result<kchess_contracts::types::StudyCommandResult, CoreError>>>,
    >,
>;

/// Evaluations kept per position, so stepping back and forth shows them at once.
const CACHE_SIZE: usize = 600;
/// Stepping quickly through a game should not send every position on the way to Lichess.
const CLOUD_DELAY: Duration = Duration::from_millis(300);
/// The evaluation bar's gap to the board; without the bar the board keeps its place.
const EVAL_GAP: Pixels = px(8.);
/// The row above the board that holds what the far side has won.
const MATERIAL_ROW: Pixels = px(18.);
const ENGINE_ON: &str = "kchess:analysis-engine";
const ENGINE_LINES: &str = "kchess:analysis-lines";
pub(crate) const INFINITE: &str = "kchess:analysis-infinite";
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

/// A position of the tree, read by its variant's rules.
fn position_of(variant: Variant, fen: &str) -> Option<VariantPosition> {
    kchess_domain::rules::setup_start(variant, fen)
}

/// The position with the other side to move (a null move), whose best line is the threat; `None`
/// in check, where the side to move cannot pass.
fn threat_fen(variant: Variant, fen: &str) -> Option<String> {
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

/// Why the review or the explorer is held back, and Check again when a check for a Lichess game
/// failed.
fn paused_note(pause: crate::online::Pause, palette: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .gap(px(4.))
        .px(px(8.))
        .py(px(24.))
        .text_size(px(13.))
        .text_center()
        .text_color(palette.text_muted)
        .child(pause.text())
        .when(pause == crate::online::Pause::Unverified, |note| {
            note.child(crate::online::check_again_button("assistance-check-again"))
        })
        .into_any_element()
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
    /// Engine help was lent when the online state last changed: its review is asked for when
    /// that comes back.
    assisted: bool,
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
    move_box: crate::game::surface::MoveBox,
    lines_choice: Entity<SelectState<Vec<SharedString>>>,
    import: Entity<TextareaState>,
    import_error: Option<SharedString>,
    /// A Lichess game being fetched for the import, by its turn: a later import replaces it.
    importing: Option<u64>,
    import_turn: u64,
    /// Shown while the page is open: the engine follows the board.
    attached: bool,
    /// Whether the engine can analyse: `None` until the core says, so the page neither offers
    /// to set Stockfish up nor makes room for nothing while it asks.
    engine_ready: Option<bool>,
    engine_identity: Option<String>,
    evaluations: HashMap<String, AnalysisUpdate>,
    /// The cache's keys, oldest first.
    order: VecDeque<String>,
    /// The search of the position shown, as every board searches one: its latest update, error
    /// and whether it runs.
    search: crate::position_engine::PositionEngine,
    cloud: Option<kchess_contracts::types::CloudEval>,
    cloud_busy: bool,
    cloud_error: Option<String>,
    cloud_request: u64,
    cloud_task: Option<Task<()>>,
    /// The main line's novelty in the game databases.
    novelty: novelty::Novelty,
    saver: SessionSaver<AnalysisPage>,
    wheel: crate::game::view::Wheel,
    focus: FocusHandle,
    /// Spoken moves, and “back”, “next”, “best move”, “flip board”.
    voice: VoiceMoves,
    /// Moves tried instead of a studied puzzle's solution, waiting for their reviews to be
    /// labelled.
    tried_labels: Vec<Subscription>,
    /// The tried moves already labelled, by path.
    tried_done: Vec<String>,
    /// Moves played without their sound: all but the last of an engine line played at once.
    quiet_moves: bool,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AnalysisPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The mouse wheel over the board steps along the line shown, as the arrows do; the moves are a
/// tree, so the page steps by `step_by` alone.
impl Steps for AnalysisPage {
    fn steps(&self, _: &App) -> Option<(usize, usize)> {
        None
    }

    fn step_to(&mut self, _: usize, _: &mut Window, _: &mut Context<Self>) {}

    fn wheel(&mut self) -> &mut crate::game::view::Wheel {
        &mut self.wheel
    }

    fn step_by(&mut self, way: Way, _: &mut Window, cx: &mut Context<Self>) {
        match way {
            Way::On => self.forward(cx),
            Way::Back => self.back(cx),
        }
    }
}

impl Resumes for AnalysisPage {
    fn saver(&mut self) -> &mut SessionSaver<Self> {
        &mut self.saver
    }

    fn snapshot(&mut self, _: &mut Context<Self>) -> SessionDocument {
        SessionDocument::Analysis(AnalysisSession {
            pgn: self.root.pgn_text(),
            path: self.path.clone(),
            orientation: self.orientation,
            study: self.study.clone(),
            chapter: self.chapter.clone(),
        })
    }

    /// The board opens as it was left, unless another document took it while the session was
    /// read: that one is kept, and saved in its place.
    fn resume(&mut self, saved: Option<Sessions>, _: &mut Window, cx: &mut Context<Self>) {
        let Some(mut saved) = saved else {
            return;
        };
        let session = saved.analysis.take();
        if let Some(session) = session.filter(|_| self.navigation == 0) {
            self.reopen(&session, cx);
            self.follow_game(cx);
        } else if self.navigation > 0 || !self.root.children.is_empty() {
            self.schedule_save(cx);
        }
    }
}

impl AnalysisPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> AnalysisPage {
        let library = cx.global::<StudyHandle>().0.clone();
        let (root, path, orientation) = (TreeNode::new(None), String::new(), Color::White);
        let page = cx.weak_entity();
        let study_bar = cx.new(|cx| StudyBar::new(page, window, cx));
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &position_of(Variant::Standard, STANDARD_FEN).unwrap_or_default(),
                orientation,
                None,
                None,
            ))
        });
        let (move_box, typed) = crate::game::surface::MoveBox::new(Self::typed_move, window, cx);
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
        let explorer =
            cx.new(|cx| PositionExplorer::new(crate::explorer::Layout::List, window, cx));
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
            typed,
            cx.subscribe_in(
                &lines_choice,
                window,
                |this, state, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(2, |index| index.row);
                    let before = prefs_lines(cx);
                    if before == row + 1 {
                        return;
                    }
                    crate::prefs::set(cx, ENGINE_LINES, row + 1);
                    this.engine_changed(cx);
                    let page = cx.entity().downgrade();
                    crate::setting_notice::offer(
                        "Engine lines changed",
                        move |window, cx| {
                            let _ = page.update(cx, |page, cx| page.set_lines(before, window, cx));
                        },
                        window,
                        cx,
                    );
                },
            ),
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.ask_cloud(cx);
                this.redraw(cx);
            }),
            cx.observe(&online, |this, _, cx| {
                this.engine_changed(cx);
                let assisted = Self::assistance_allowed(cx);
                if assisted != this.assisted {
                    this.assisted = assisted;
                    this.follow_review(cx);
                    this.find_novelty(cx);
                }
            }),
            // A database imported, changed or deleted: the novelty is looked up again.
            cx.observe(&cx.global::<GameDbHandle>().0.clone(), |this, _, cx| {
                this.find_novelty(cx)
            }),
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
            cx.subscribe(&events, |this, _, event: &ClientEvent, cx| match event {
                ClientEvent::Event(CoreEvent::Analysis(update)) => this.receive(update.clone(), cx),
                // A search on a lost connection ended with it: the board's is started again.
                ClientEvent::Reconnected => this.analyse(cx),
                _ => {}
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
            study: String::new(),
            chapter: String::new(),
            study_pgn: String::new(),
            study_error: None,
            study_save: None,
            study_writing: None,
            study_writes: 0,
            navigation: 0,
            study_bar,
            review: review::ReviewState::new(Vec::new()),
            explainer: cx.new(|_| crate::ai::Explainer::default()),
            practice: None,
            paused_for_review: false,
            assisted: Self::assistance_allowed(cx),
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
            move_box,
            lines_choice,
            import,
            import_error: None,
            importing: None,
            import_turn: 0,
            attached: false,
            engine_ready: None,
            engine_identity: None,
            evaluations: HashMap::new(),
            order: VecDeque::new(),
            search: Default::default(),
            cloud: None,
            cloud_busy: false,
            cloud_error: None,
            cloud_request: 0,
            cloud_task: None,
            novelty: Default::default(),
            saver: SessionSaver::start(SessionKind::Analysis, &[], window, cx),
            wheel: Default::default(),
            focus: cx.focus_handle(),
            voice: VoiceMoves::new(crate::voice::VoiceSource::Analysis, cx),
            tried_labels: Vec::new(),
            tried_done: Vec::new(),
            quiet_moves: false,
            _subscriptions: subscriptions,
        };
        page.redraw(cx);
        page.read_mistakes(cx);
        let weak = cx.entity().downgrade();
        // The session is the saver's to save; the open study's chapter is the page's.
        crate::quit::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            let study = page.update(cx, |page, cx| page.persist_study(cx))?;
            Some(
                Box::pin(async move { study.await.map(|_| ()).map_err(|cause| cause.message) })
                    as BoxedSave,
            )
        });
        page
    }

    /* ── The tree ─────────────────────────────────────────────────────── */

    fn node(&self) -> &TreeNode {
        self.root.node_at(&self.path)
    }

    #[cfg(test)]
    pub(crate) fn board_badge(&self, cx: &App) -> Option<Badge> {
        self.board.read(cx).state().badge.clone()
    }

    #[cfg(test)]
    pub(crate) fn shown_fen(&self) -> String {
        self.node().fen.clone()
    }

    #[cfg(test)]
    pub(crate) fn engine_text(&self, cx: &App) -> String {
        self.engine_status(cx)
    }

    /// The review shown, once it covers the whole main line and is finished, with its summary.
    #[cfg(test)]
    pub(crate) fn finished_review(
        &self,
        cx: &App,
    ) -> Option<(
        kchess_contracts::types::StoredReview,
        kchess_domain::review::GameAnalysis,
    )> {
        let review = self
            .review(cx)
            .filter(|review| review.complete && review.key == self.mainline_key())?;
        let (analysis, _) = self.game_analysis(cx)?;
        Some((review, analysis))
    }

    fn position(&self) -> Option<VariantPosition> {
        position_of(self.root.variant(), &self.node().fen)
    }

    /// Whether the game on the board is played by standard rules (standard chess or Chess960):
    /// cloud evaluation and the editor are theirs.
    fn standard_rules(&self) -> bool {
        self.root.variant().standard_rules()
    }

    /// The position shown, as a game to play from: in the game's variant, or (standard chess)
    /// castling as Chess960 when its rooks need it.
    fn play_setup(&self) -> Setup {
        let fen = self.node().fen.clone();
        match self.fairy_variant() {
            Some(variant) => Setup { variant, fen },
            None => crate::game::setup_from_fen(&fen),
        }
    }

    /// The game's variant when Fairy-Stockfish analyses and reviews it.
    pub(super) fn fairy_variant(&self) -> Option<Variant> {
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
            Some(fen) => analysis_context::<&str>(&fen, &[]),
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

    /// `text` said clearly, with voice on: as the recognizer's phrase reaches the board.
    #[cfg(test)]
    pub(crate) fn hear_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        self.voice.control.enabled = true;
        let heard = crate::voice::Heard {
            text: text.into(),
            raw: text.into(),
            confidence: 1.,
            words: Vec::new(),
            unclear: false,
        };
        let owner = self.voice.control.owner;
        self.voice_heard(&crate::voice::VoiceEvent::Heard { owner, heard }, cx);
    }

    #[cfg(test)]
    pub(crate) fn orientation_for_test(&self) -> Color {
        self.orientation
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
                if !self.quiet_moves {
                    self.move_sound(cx);
                }
                self.changed(cx);
                true
            }
            None => {
                self.redraw(cx);
                false
            }
        }
    }

    /// Show the end of `moves` (UCI) from the start, adding those the tree does not have, as if
    /// played on the board; the moves stop at one that is not legal.
    pub(crate) fn show_line(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let mut path = String::new();
        for uci in moves {
            match self.root.add_move(&path, uci) {
                Some(next) => path = next,
                None => break,
            }
        }
        self.path = path;
        self.changed(cx);
    }

    /// Show the position at `path`.
    ///
    /// Which moves sound, as on Lichess's analysis board: a move played here (board, typed,
    /// spoken, explorer, engine, in `play_move`) and a step one move forward, by → or the wheel,
    /// "next", Shift → or a click on the next move in the list. Stepping back, jumping further
    /// (start, end, a move elsewhere in the list, the review's graph), loading a game, undo and
    /// deleting are silent, so no step plays a clip for a move it did not just show being made.
    fn go_to(&mut self, path: &str, cx: &mut Context<Self>) {
        let next = self.root.existing_path(path);
        let forward = moves_of(&next)
            .split_last()
            .is_some_and(|(_, before)| *before == *moves_of(&self.path));
        self.path = next;
        if forward {
            self.move_sound(cx);
        }
        self.changed(cx);
    }

    /// The clip for the move that led to the position shown: a capture's or a quiet move's.
    fn move_sound(&self, cx: &mut Context<Self>) {
        let san = &self.node().san;
        if !san.is_empty() {
            crate::sound::move_sound(san, cx);
        }
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

    /// Analyse a position set up in the board editor, facing as it did there.
    /// The board editor's position, in its variant. False when the board cannot take it.
    pub fn analyse_setup(
        &mut self,
        setup: &Setup,
        orientation: Color,
        cx: &mut Context<Self>,
    ) -> bool {
        let fen = setup.fen.as_str();
        if setup.variant.standard_rules() {
            self.orientation = orientation;
            self.load(Some(fen), cx);
            return true;
        }
        // Another variant opens as a game of it from that position.
        let pgn = format!(
            "[Variant \"{}\"]\n[FEN \"{fen}\"]\n[SetUp \"1\"]\n\n*",
            setup.variant.label()
        );
        let loaded = self.load_pgn(&pgn, cx);
        if loaded {
            self.orientation = orientation;
        }
        loaded
    }

    /// Load a game, opened at its start; refused for a variant this board cannot play.
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
        // After a session that could not be read, the board's new document is saved.
        self.saver.open();
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

    /// The whole tree as PGN.
    pub fn pgn(&self) -> String {
        self.root.pgn_text()
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
        !self.study.is_empty() && self.root.pgn_text() != self.study_pgn
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

    /// The game database game the board's game was opened from, or last saved as.
    pub fn database_game(&self) -> Option<&DatabaseGame> {
        self.review.origin.as_ref()?.database.as_ref()
    }

    /// The board's game is now `game` of a game database: Save changes replaces that one.
    pub fn saved_to_database(&mut self, game: DatabaseGame, cx: &mut Context<Self>) {
        self.review
            .origin
            .get_or_insert_with(Default::default)
            .database = Some(game);
        self.find_novelty(cx);
        cx.notify();
    }

    pub fn navigation(&self) -> u64 {
        self.navigation
    }

    /// The board holds moves no study keeps, so replacing it asks first.
    pub fn unsaved(&self) -> bool {
        self.study.is_empty() && !self.root.children.is_empty()
    }

    /// Load a line from the starting position, opened at its end (an opening's Analyse).
    pub fn load_line(&mut self, moves: &[String], cx: &mut Context<Self>) {
        let pgn = kchess_domain::pgn::pgn_from_uci(moves);
        // Straight to its end, not by `go_to`: a loaded line of one move is not a move played.
        if self.load_pgn(&pgn, cx) {
            self.path = self.root.line_end("");
            self.changed(cx);
        }
    }

    fn clear_evaluations(&mut self) {
        self.evaluations.clear();
        self.order.clear();
        self.search.update = None;
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = self.position();
        if let Some(uci) = self.move_box.take(position.as_ref(), window, cx) {
            self.play(&uci, cx);
        }
    }

    /// The board, the engine and the saved session follow a change.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let typing = self.typing_comment.take();
        self.edits.track(&self.root, &self.path, typing);
        self.reveal_move.set(true);
        self.follow_game(cx);
        self.schedule_save(cx);
    }

    /// Everything shown of the game follows the one on the board: its review, the practice of
    /// it, the board, the engine and the cloud's evaluation. A change and a reopened session both
    /// come here; the reopened one once listed its own steps and left out the review, which then
    /// showed only once the review queue's first report, 10 s after launch, looked it up.
    fn follow_game(&mut self, cx: &mut Context<Self>) {
        self.follow_review(cx);
        self.check_practice(cx);
        self.redraw(cx);
        self.analyse(cx);
        self.ask_cloud(cx);
        self.find_novelty(cx);
    }

    /// Save the session, and the open study's chapter, shortly after the last change.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.study.is_empty() {
            self.study_save = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVE_DELAY).await;
                let _ = this.update(cx, |this, cx| this.persist_study(cx));
            }));
        }
        SessionSaver::changed(self, cx);
    }

    /// The board as the profile keeps it, to open as it was left.
    fn reopen(&mut self, session: &AnalysisSession, cx: &mut Context<Self>) {
        let root = TreeNode::from_pgn(&session.pgn).unwrap_or_else(|| TreeNode::new(None));
        self.path = root.existing_path(&session.path);
        self.root = root;
        self.orientation = session.orientation;
        // A recovered session that differs from its study stays an unsaved copy; it never
        // overwrites the library.
        let library = cx.global::<StudyHandle>().0.clone();
        let saved_study = session.study.as_str();
        let linked = library.read(cx).study(saved_study).cloned();
        let linked_pgn = linked.as_ref().map(|study| {
            study
                .chapter(&session.chapter)
                .map_or_else(|| study.pgn.clone(), |chapter| chapter.pgn.clone())
        });
        let conflict = linked_pgn
            .as_deref()
            .and_then(TreeNode::from_pgn)
            .is_some_and(|tree| tree.pgn_text() != self.root.pgn_text());
        self.study = if conflict || linked.is_none() {
            String::new()
        } else {
            saved_study.to_string()
        };
        self.chapter = Some(session.chapter.clone())
            .filter(|chapter| !chapter.is_empty())
            .or_else(|| {
                linked
                    .as_ref()
                    .and_then(|study| Some(study.chapters.first()?.id.clone()))
            })
            .unwrap_or_default();
        self.study_pgn = if self.study.is_empty() {
            String::new()
        } else {
            linked_pgn.unwrap_or_default()
        };
        if !self.study.is_empty() {
            library.read(cx).track(&self.study, &self.chapter);
        }
        self.study_error = conflict.then(|| CONFLICT.to_string());
        self.edits = edits::Edits::new(&self.root, &self.path);
        self.clear_evaluations();
    }

    /// The positions kept for practice, which the review counts its own among.
    fn read_mistakes(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().request(calls::Mistakes {});
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |this, cx| match read {
                // A review that saved mistakes meanwhile holds the newer list.
                Ok(mistakes) if this.review.mistakes.is_empty() => {
                    this.review.mistakes = mistakes;
                    cx.notify();
                }
                Ok(_) => {}
                Err(cause) => crate::log::write(
                    Level::Warn,
                    "library",
                    &format!("Reading the mistakes to practise failed: {}", cause.message),
                ),
            });
        })
        .detach();
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
        // The review's label on the move just played; without one, checkmate's "#", then the
        // game's novelty.
        let marks = self.review_marks(cx);
        let mate = state.badge.take();
        let novelty = self.novelty_badge(last_move);
        state.badge = marks
            .get(&self.path)
            .and_then(|mark| mark.classification)
            .zip(last_move)
            .map(|(classification, (_, to))| {
                let (_, glyph, color) = review::classification_look(
                    classification.as_str(),
                    cx.global::<Palette>().dark,
                );
                Badge {
                    square: to,
                    glyph,
                    color,
                }
            })
            .or(mate)
            .or(novelty);
        // Practising: the game's move in red, to find a better one, and the piece to move once
        // hinted.
        if let Some(practice::PracticeShapes { played, hint }) = self.practice_shapes() {
            state.arrows = played
                .map(|(from, to)| Arrow::brush("red", from, to))
                .into_iter()
                .collect();
            // The piece to move ringed as a puzzle's hint is, which the board also says.
            state.hint = hint;
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
                    let uci = line.pv.first()?;
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
        let key = event.keystroke.key.as_str();
        match key {
            "left" => self.back(cx),
            "right" => self.forward(cx),
            "up" | "home" => self.go_to_start(cx),
            "down" | "end" => self.go_to_end(cx),
            _ => match page_key(KEYS, key) {
                Some(AnalysisKey::Engine) => self.toggle_engine(cx),
                Some(AnalysisKey::Threat) => self.toggle_threat(cx),
                Some(AnalysisKey::Comment) => self.focus_notes(window, cx),
                Some(AnalysisKey::Explorer) => self.toggle_explorer(window, cx),
                Some(AnalysisKey::Delete) => self.delete_from_here(window, cx),
                // Push to talk holds Space for the microphone.
                Some(AnalysisKey::Best) if !crate::settings::frontend(cx).voice_push_to_talk => {
                    self.play_best(cx)
                }
                _ => return,
            },
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
            kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Analysis).to_vec(),
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
        let setup = Setup {
            variant: self.root.variant(),
            fen,
        };
        // The threat's best move is the opponent's: "best move" waits for the position's own.
        let best = (Self::assistance_allowed(cx)
            && !self.hides_engine()
            && self.threat_position().is_none())
        .then(|| self.lines(cx).first().cloned())
        .flatten()
        .and_then(|line| line.pv.first().cloned());
        let mut command: Option<AnalysisCommand> = None;
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
                let said = kchess_domain::voice::analysis_command(text)?;
                let feedback = if said == AnalysisCommand::Best && best.is_none() {
                    "The engine has no move yet".to_string()
                } else {
                    format!("“{text}”")
                };
                command = Some(said);
                Some(Reply {
                    said: said.as_str().to_string(),
                    feedback,
                    understood: true,
                })
            },
            cx,
        );
        match (action, command) {
            (Action::Play(uci), _) => {
                self.play(&uci, cx);
            }
            (_, Some(AnalysisCommand::Back)) => self.back(cx),
            (_, Some(AnalysisCommand::Forward)) => self.forward(cx),
            (_, Some(AnalysisCommand::Start)) => self.go_to_start(cx),
            (_, Some(AnalysisCommand::End)) => self.go_to_end(cx),
            (_, Some(AnalysisCommand::Best)) => {
                if let Some(uci) = best {
                    self.play(&uci, cx);
                }
            }
            (_, Some(AnalysisCommand::Flip)) => self.flip(cx),
            (_, Some(AnalysisCommand::Engine)) => self.toggle_engine(cx),
            (_, None) => {}
        }
        cx.notify();
    }

    pub(crate) fn toggle_engine(&mut self, cx: &mut Context<Self>) {
        let on = !Self::engine_on(cx);
        crate::prefs::set(cx, ENGINE_ON, on);
        self.engine_changed(cx);
    }

    /// The game an Export saves: the document with its variations and comments, its main line
    /// for the GIF, and the position shown (in a variation too) for the PNG.
    fn game_export(&self) -> PlayedGame {
        let mainline = self.mainline();
        let depth = if self.path.is_empty() {
            0
        } else {
            self.path.split(' ').count()
        };
        PlayedGame {
            setup: if self.standard_rules() {
                crate::game::setup_from_fen(&self.root.fen)
            } else {
                Setup {
                    variant: self.root.variant(),
                    fen: self.root.fen.clone(),
                }
            },
            ply: Some(depth.min(mainline.len())),
            moves: mainline,
            orientation: self.orientation,
            white: self.player_name(Color::White),
            black: self.player_name(Color::Black),
            result: kchess_domain::games::Outcome::from_tag(&self.header("Result")),
            player: self.review.origin.as_ref().and_then(|origin| origin.player),
            account: self
                .review
                .origin
                .as_ref()
                .and_then(|origin| origin.account.clone()),
            // A Chess.com game has no page on Lichess: it is uploaded like any other game.
            lichess_id: self
                .review
                .origin
                .as_ref()
                .and_then(|origin| origin.game_id.clone())
                .filter(|id| !kchess_domain::chesscom::is_game_id(id)),
            pgn: Some(self.root.pgn_text()),
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
        let pgn = self.root.pgn_text();
        self.copy(pgn, "PGN", window, cx);
    }

    /// Copy the moves to the move shown (Quick Pick).
    pub(crate) fn copy_shown_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.copy_line(&path, window, cx);
    }

    /// Copy the moves from the start to `path` as PGN, without the side lines.
    fn copy_line(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let pgn = self.root.line_to(path).pgn_text();
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
