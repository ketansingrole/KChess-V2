//! The analysis board (`pages/analysis.vue`, `stores/analysis.ts`): a move tree explored on the
//! board, Stockfish's lines and arrows for the position shown, Lichess's cloud evaluation, and the
//! session the Electron app keeps (`sessions.analysis`), so either app opens it as it was left.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
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
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::core::{AppCore, BoxedSave, CoreEvent};
use crate::game::tree::{
    TreeNode, analysis_context, child_path, comment_text, format_eval, move_glyph, move_number,
    moves_of, path_of, pv_san, winning_chances,
};
use crate::game::{STANDARD_FEN, rules, view};
use crate::online::OnlineHandle;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, tint};
use crate::widgets;

/// Evaluations kept per position, so stepping back and forth shows them at once.
const CACHE_SIZE: usize = 600;
const SAVE_DELAY: Duration = Duration::from_millis(150);
/// Stepping quickly through a game should not send every position on the way to Lichess.
const CLOUD_DELAY: Duration = Duration::from_millis(300);
/// The evaluation bar's width and its gap to the board; without it the board keeps its place.
const EVAL_BAR: Pixels = px(14.);
const EVAL_GAP: Pixels = px(8.);
/// One engine line's row.
const PV_ROW: Pixels = px(34.);

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
    /// The study the Electron app links the board to; kept as it was.
    study: Value,
    chapter: Value,
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
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AnalysisPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AnalysisPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> AnalysisPage {
        let session = cx
            .global::<AppCore>()
            .bridge
            .call_now("library", Vec::new())
            .map(|library| library["sessions"]["analysis"].clone())
            .unwrap_or(Value::Null);
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
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, cx);
                }
            }),
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
            cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
                if event.name == "engine:analysis" {
                    this.receive(event.payload.clone(), cx);
                }
            }),
        ];
        let mut page = AnalysisPage {
            root,
            path,
            orientation,
            study: session["study"].clone(),
            chapter: session["chapter"].clone(),
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
            _subscriptions: subscriptions,
        };
        page.redraw(cx);
        let weak = cx.entity().downgrade();
        crate::core::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            Some(page.update(cx, |page, cx| page.save_now(cx)))
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

    /// Play a move from the position shown; false when it is not legal there.
    fn play(&mut self, uci: &str, cx: &mut Context<Self>) -> bool {
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

    fn forward(&mut self, cx: &mut Context<Self>) {
        if let Some(child) = self.node().children.first() {
            let next = child_path(&self.path, &child.uci);
            self.go_to(&next, cx);
        }
    }

    fn go_to_start(&mut self, cx: &mut Context<Self>) {
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
    fn load(&mut self, fen: Option<&str>, cx: &mut Context<Self>) {
        self.root = TreeNode::new(fen);
        self.path.clear();
        self.clear_evaluations();
        self.study = Value::String(String::new());
        self.changed(cx);
    }

    /// Load a game (`loadPgn`), opened at its start.
    fn load_pgn(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(tree) = TreeNode::from_pgn(text) else {
            return false;
        };
        self.root = tree;
        self.path.clear();
        self.clear_evaluations();
        self.study = Value::String(String::new());
        self.changed(cx);
        true
    }

    /// The board holds moves no study keeps (`useReplaceAnalysis`), so replacing it asks first.
    pub fn unsaved(&self) -> bool {
        self.study.as_str().unwrap_or_default().is_empty() && !self.root.children.is_empty()
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
        self.redraw(cx);
        self.analyse(cx);
        self.ask_cloud(cx);
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let save = this.update(cx, |this, cx| this.save_now(cx)).ok();
            if let Some(save) = save {
                save.await;
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
            "study": self.study.as_str().unwrap_or_default(),
            "chapter": self.chapter.as_str().unwrap_or_default(),
        });
        let save = cx
            .global::<AppCore>()
            .call("saveSession", vec![json!("analysis"), session]);
        Box::pin(async move {
            if let Err(cause) = save.await {
                crate::log::write(
                    Level::Warn,
                    "library",
                    &format!("saving session failed: analysis {}", cause.message),
                );
            }
        })
    }

    /* ── The engine ───────────────────────────────────────────────────── */

    fn engine_on(cx: &App) -> bool {
        crate::prefs::get(cx, ENGINE_ON, true)
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
        let node = self.node();
        let position = position_of(&node.fen).unwrap_or_default();
        let last_move = now_square(&node.uci, 0..2).zip(now_square(&node.uci, 2..4));
        let mut state = BoardState::of(
            &position,
            self.orientation,
            Some(position.turn()),
            last_move,
        );
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

    fn toggle_engine(&mut self, cx: &mut Context<Self>) {
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

struct TreeView<'a> {
    current: &'a str,
    page: WeakEntity<AnalysisPage>,
    palette: Palette,
}

impl TreeView<'_> {
    fn move_chip(&self, node: &TreeNode, path: &str, number: String, main: bool) -> AnyElement {
        let active = path == self.current;
        let palette = &self.palette;
        let (page, target) = (self.page.clone(), path.to_string());
        let glyph = move_glyph(node);
        h_flex()
            .id(SharedString::from(format!("tree-{path}")))
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
            .on_click(move |_, _, cx| {
                let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
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
            .into_any_element()
    }

    fn comment(&self, text: String, path: &str, block: bool) -> AnyElement {
        let (page, target) = (self.page.clone(), path.to_string());
        let palette = &self.palette;
        div()
            .id(SharedString::from(format!("comment-{path}")))
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

    /// `AnalysisLine.vue`: a line's moves run on, the alternatives to a move follow it in
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

/// `EvalBar.vue`: White's share fills from White's side of the board; stripes until there is
/// something to show.
fn eval_bar(line: Option<&Value>, orientation: Color, result: Option<&str>) -> impl IntoElement {
    let known = line.is_some() || result.is_some();
    let white = match result {
        Some("1-0") => 1.,
        Some("0-1") => 0.,
        Some(_) => 0.5,
        None => 0.5 + 0.5 * winning_chances(line),
    };
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let dark = rgb(0x403d39);
            window.paint_quad(fill(bounds, dark).corner_radii(px(4.)));
            if !known {
                // `repeating-linear-gradient(135deg, …)`: light stripes over the dark bar.
                let step = 8.;
                let height = bounds.size.height.as_f32();
                let width = bounds.size.width.as_f32();
                let mut offset = -width;
                while offset < height {
                    let mut stripe = PathBuilder::fill();
                    let at = |x: f32, y: f32| {
                        point(
                            bounds.origin.x + px(x),
                            bounds.origin.y + px(y.clamp(0., height)),
                        )
                    };
                    stripe.move_to(at(0., offset + 4.));
                    stripe.line_to(at(0., offset + step));
                    stripe.line_to(at(width, offset + step - width));
                    stripe.line_to(at(width, offset + 4. - width));
                    stripe.close();
                    if let Ok(path) = stripe.build() {
                        window.paint_path(path, rgb(0x68645e));
                    }
                    offset += step;
                }
                return;
            }
            let fill_height = bounds.size.height * white;
            let white_part = if orientation == Color::White {
                Bounds::new(
                    point(bounds.origin.x, bounds.bottom() - fill_height),
                    size(bounds.size.width, fill_height),
                )
            } else {
                Bounds::new(bounds.origin, size(bounds.size.width, fill_height))
            };
            window.paint_quad(fill(white_part, rgb(0xf2f0ea)).corner_radii(px(4.)));
            let middle = Bounds::new(
                point(bounds.origin.x, bounds.center().y - px(1.)),
                size(bounds.size.width, px(2.)),
            );
            window.paint_quad(fill(middle, rgba(0xd64f008c)));
        },
    )
    .w(EVAL_BAR)
    .h_full()
    .flex_none()
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
                self.render_lines(
                    &lines,
                    if using_cloud { 0 } else { prefs_lines(cx) },
                    over.map(|_| "Nothing to analyse."),
                    cx,
                    palette,
                )
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

    /// `EngineLines.vue`: each line's score and its moves, a click playing up to the move.
    fn render_lines(
        &self,
        lines: &[Value],
        pending: usize,
        ended: Option<&'static str>,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement {
        let fen = self.node().fen.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(ended) = ended {
            rows.push(
                h_flex()
                    .h(PV_ROW * pending.max(1) as f32)
                    .justify_center()
                    .text_color(palette.text_muted)
                    .child(ended)
                    .into_any_element(),
            );
        } else {
            for (index, line) in lines.iter().enumerate() {
                let pv: Vec<String> =
                    serde_json::from_value(line["pv"].clone()).unwrap_or_default();
                let moves = pv_san(&fen, &pv, 12);
                let eval_pv = pv.clone();
                rows.push(
                    h_flex()
                        .h(PV_ROW)
                        .gap(px(8.))
                        .px(px(8.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .child(
                            div()
                                .id(("pv-eval", index))
                                .w(px(55.))
                                .flex_none()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(6.))
                                .bg(palette.accented)
                                .font_weight(FontWeight::BOLD)
                                .text_center()
                                .cursor_pointer()
                                .hover(|eval| eval.bg(tint(palette.primary, 24.)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.play_line(&eval_pv, 0, cx)
                                }))
                                .child(format_eval(Some(line))),
                        )
                        .child(h_flex().min_w_0().overflow_hidden().children(
                            moves.into_iter().enumerate().map(|(at, played)| {
                                let pv = pv.clone();
                                div()
                                    .id(SharedString::from(format!("pv-{index}-{at}")))
                                    .flex_none()
                                    .px(px(2.))
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .text_color(palette.toned)
                                    .cursor_pointer()
                                    .hover(|chip| chip.bg(tint(palette.primary, 24.)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.play_line(&pv, at, cx)
                                    }))
                                    .child(played.label)
                            }),
                        ))
                        .into_any_element(),
                );
            }
            for index in lines.len()..pending {
                rows.push(
                    h_flex()
                        .h(PV_ROW)
                        .px(px(8.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .child(
                            div()
                                .w(px(55.))
                                .text_center()
                                .text_color(palette.dimmed)
                                .child("…"),
                        )
                        .into_any_element(),
                );
            }
        }
        v_flex()
            .flex_none()
            .text_size(px(12.5))
            .border_1()
            .border_color(palette.border)
            .rounded(px(10.))
            .bg(palette.bg)
            .overflow_hidden()
            .children(rows)
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
        let palette = *cx.global::<Palette>();
        let tool = Tool::of(&crate::prefs::get::<String>(cx, TOOL, "moves".into()));
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let show_bar = allowed && (Self::engine_on(cx) || using_cloud);
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
        let board_size = self.layout.board_size(window);
        let entry = h_flex()
            .h(px(36.))
            .flex_1()
            .border_1()
            .border_color(palette.border)
            .rounded(px(8.))
            .bg(palette.bg)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.move_input).appearance(false).small()),
            )
            .child(
                div()
                    .w(px(32.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_color(palette.border)
                    .text_color(palette.text_muted)
                    .child("↵"),
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
                        row.child(eval_bar(lines.first(), self.orientation, result))
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
                            .child(self.board.clone()),
                    ),
            )
            .child(
                v_flex()
                    .pl(EVAL_BAR + EVAL_GAP)
                    .gap(px(4.))
                    .child(entry)
                    .children(self.move_message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    })),
            );
        let tool_body: AnyElement = match tool {
            Tool::Moves => div()
                .id("analysis-moves")
                .flex_1()
                .min_h(px(96.))
                .overflow_y_scroll()
                .p(px(8.))
                .border_1()
                .border_color(palette.border)
                .rounded(px(10.))
                .bg(palette.bg)
                .child(self.render_moves(cx, &palette))
                .into_any_element(),
            Tool::Review | Tool::Explorer => div()
                .flex_1()
                .px(px(8.))
                .py(px(24.))
                .text_size(px(13.))
                .text_center()
                .text_color(palette.text_muted)
                .child(if tool == Tool::Review {
                    "Game review is not in this app yet."
                } else {
                    "The opening explorer is not in this app yet."
                })
                .into_any_element(),
        };
        let panel = widgets::side_panel(&palette)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .h(board_size + px(36.) + widgets::STACK_GAP)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(12.))
            .child(self.render_engine(cx, &palette))
            .child(self.render_tabs(tool, &palette))
            .child(tool_body)
            .child(self.render_controls(cx));
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .child(
                self.layout
                    .render("analysis", board_column, panel, window, cx),
            )
    }
}
