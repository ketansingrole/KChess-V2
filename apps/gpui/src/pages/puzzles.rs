//! Puzzles (`pages/puzzles.vue`): the Train tab (`PuzzleTrain.vue`, `PuzzleBoard.vue`,
//! `PuzzleStatusPanel.vue`) with its three modes: Lichess practice, the downloaded database, and
//! rated puzzles. The session's transitions are Rust (`game/puzzle.rs`); this page fetches what
//! they ask for, plays the puzzle on the board and keeps the attempt log up to date.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, Icon, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::core::{AppCore, CoreEvent};
use crate::game::puzzle::{OnBoard, PuzzleSession, Step, local_day};
use crate::game::{STANDARD_FEN, rules, view};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, side_padding};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, tint};
use crate::widgets::{self, Pill, Tone};

const TAB: &str = "kchess:puzzle-tab";
const MODE: &str = "kchess:puzzle-mode";
const ANGLE: &str = "kchess:puzzle-angle";
const DIFFICULTY: &str = "kchess:puzzle-difficulty";
const COLOR: &str = "kchess:puzzle-color";
const ACCOUNT: &str = "kchess:puzzle-account";
/// The opening an opening page chose puzzles of: its key is the angle while it is chosen.
const OPENING: &str = "kchess:puzzle-opening";

/// Milliseconds before the opponent replies, and the solution's pace when it plays itself.
const REPLY_DELAY: Duration = Duration::from_millis(350);
const SOLUTION_STEP: Duration = Duration::from_millis(650);
const SOLUTION_REPLY: Duration = Duration::from_millis(450);
/// A pause on the final position before the next puzzle replaces it.
const AUTO_NEXT_DELAY: Duration = Duration::from_millis(800);
const FLASH: Duration = Duration::from_millis(900);

const MODES: [(&str, &str); 3] = [
    ("practice", "Practice"),
    ("offline", "Downloaded"),
    ("rated", "Rated"),
];
const DIFFICULTIES: [(&str, &str); 5] = [
    ("easiest", "Easiest"),
    ("easier", "Easier"),
    ("normal", "Normal"),
    ("harder", "Harder"),
    ("hardest", "Hardest"),
];
const COLORS: [(&str, &str); 3] = [("random", "Random"), ("white", "White"), ("black", "Black")];

/// `MODE_HINTS`: what each mode means for the result, where the choice is made.
fn mode_hint(mode: &str) -> &'static str {
    match mode {
        "offline" => "Puzzles downloaded to this computer.",
        "rated" => "Results affect your Lichess rating.",
        _ => "Lichess puzzles, unrated.",
    }
}

/// The puzzle themes (`data/puzzle-themes.json`), as Lichess names them.
#[derive(serde::Deserialize)]
struct ThemeGroup {
    themes: Vec<Theme>,
}

#[derive(serde::Deserialize, Clone)]
struct Theme {
    key: String,
    name: String,
}

fn themes() -> Vec<Theme> {
    let groups: Vec<ThemeGroup> = serde_json::from_str(include_str!(
        "../../../../crates/kchess-domain/data/puzzle-themes.json"
    ))
    .unwrap_or_default();
    std::iter::once(Theme {
        key: "mix".into(),
        name: "Healthy mix".into(),
    })
    .chain(groups.into_iter().flat_map(|group| group.themes))
    .collect()
}

/// An opening chosen for puzzles (`openingAngle`).
#[derive(serde::Serialize, serde::Deserialize)]
struct OpeningAngle {
    key: String,
    name: String,
}

/// The themes, and after the mix the opening an opening page chose puzzles of.
fn theme_items(cx: &App) -> Vec<Theme> {
    let mut list = themes();
    if let Some(opening) = crate::prefs::get::<Option<OpeningAngle>>(cx, OPENING, None) {
        list.insert(
            1,
            Theme {
                key: opening.key,
                name: opening.name,
            },
        );
    }
    list
}

/// `themeName`: the name Lichess gives a theme key; unknown keys split into words.
fn theme_name(key: &str, themes: &[Theme]) -> String {
    if let Some(theme) = themes.iter().find(|theme| theme.key == key) {
        return theme.name.clone();
    }
    let mut name = String::new();
    for (index, letter) in key.chars().enumerate() {
        if index == 0 {
            name.extend(letter.to_uppercase());
        } else {
            if letter.is_uppercase() {
                name.push(' ');
            }
            name.push(letter);
        }
    }
    name
}

/// `formatBytes`: "312 MB", "1.5 GB".
fn format_bytes(bytes: f64) -> String {
    if !bytes.is_finite() || bytes <= 0. {
        return "0 B".into();
    }
    let units = ["B", "KB", "MB", "GB"];
    let exponent = ((bytes.ln() / 1024f64.ln()).floor() as usize).min(units.len() - 1);
    let value = bytes / 1024f64.powi(exponent as i32);
    if exponent == 0 || value >= 100. {
        format!("{} {}", value.round(), units[exponent])
    } else {
        format!("{value:.1} {}", units[exponent])
    }
}

/// `formatCount`: "100,000".
fn format_count(count: f64) -> String {
    let digits = (count.max(0.).round() as u64).to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Feedback {
    None,
    Good,
    Wrong,
}

pub struct PuzzlesPage {
    session: PuzzleSession,
    on_board: Option<OnBoard>,
    /// The puzzle key the board shows (`:key="puzzleKey"`): a new key starts the board afresh.
    shown_key: u64,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    theme_list: Vec<Theme>,
    theme_select: Entity<SelectState<Vec<SharedString>>>,
    difficulty_select: Entity<SelectState<Vec<SharedString>>>,
    color_select: Entity<SelectState<Vec<SharedString>>>,
    waiting: bool,
    mistakes: u32,
    outcome: Option<bool>,
    revealed: bool,
    hinted: bool,
    hint_used: bool,
    feedback: Feedback,
    flash: bool,
    flipped: bool,
    /// Bumped when the puzzle changes, so a late timer from the old one does nothing.
    generation: u64,
    timers: Vec<Task<()>>,
    progress: Value,
    accounts: Vec<String>,
    db: Value,
    db_progress: Option<Value>,
    db_error: Option<String>,
    started: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for PuzzlesPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn select_of(
    items: Vec<SharedString>,
    chosen: Option<usize>,
    window: &mut Window,
    cx: &mut Context<PuzzlesPage>,
) -> Entity<SelectState<Vec<SharedString>>> {
    cx.new(|cx| SelectState::new(items, chosen.map(IndexPath::new), window, cx))
}

impl PuzzlesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> PuzzlesPage {
        let theme_list = theme_items(cx);
        let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
        let difficulty = crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into());
        let color = crate::prefs::get::<String>(cx, COLOR, "random".into());
        let theme_select = select_of(
            theme_list
                .iter()
                .map(|theme| SharedString::from(theme.name.clone()))
                .collect(),
            theme_list
                .iter()
                .position(|theme| theme.key == angle)
                .or(Some(0)),
            window,
            cx,
        );
        let difficulty_select = select_of(
            DIFFICULTIES
                .iter()
                .map(|(_, label)| SharedString::from(*label))
                .collect(),
            DIFFICULTIES
                .iter()
                .position(|(key, _)| *key == difficulty)
                .or(Some(2)),
            window,
            cx,
        );
        let color_select = select_of(
            COLORS
                .iter()
                .map(|(_, label)| SharedString::from(*label))
                .collect(),
            COLORS.iter().position(|(key, _)| *key == color).or(Some(0)),
            window,
            cx,
        );
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
        let events = cx.global::<AppCore>().events.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.on_move(uci, cx)
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
                &theme_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row);
                    if let Some(theme) = this.theme_list.get(row) {
                        crate::prefs::set(cx, ANGLE, theme.key.clone());
                        this.load_next(cx);
                    }
                },
            ),
            cx.subscribe(
                &difficulty_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(2, |index| index.row);
                    crate::prefs::set(cx, DIFFICULTY, DIFFICULTIES[row.min(4)].0);
                    this.load_next(cx);
                },
            ),
            cx.subscribe(
                &color_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row);
                    crate::prefs::set(cx, COLOR, COLORS[row.min(2)].0);
                    this.load_next(cx);
                },
            ),
            cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
                if event.name == "puzzledb:progress" {
                    this.db_progress = Some(event.payload.clone());
                    cx.notify();
                }
            }),
        ];
        PuzzlesPage {
            session: PuzzleSession::new(),
            on_board: None,
            shown_key: 0,
            board,
            layout: PlayLayout::default(),
            move_input,
            move_message: None,
            theme_list,
            theme_select,
            difficulty_select,
            color_select,
            waiting: false,
            mistakes: 0,
            outcome: None,
            revealed: false,
            hinted: false,
            hint_used: false,
            feedback: Feedback::None,
            flash: false,
            flipped: false,
            generation: 0,
            timers: Vec::new(),
            progress: Value::Null,
            accounts: Vec::new(),
            db: Value::Null,
            db_progress: None,
            db_error: None,
            started: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The page was shown (`onMounted`): read the accounts, the database and the log, and find a
    /// puzzle if there is none.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        let data = core.call("loadData", Vec::new());
        let db = core.call("puzzleDbStatus", Vec::new());
        let progress = core.call(
            "puzzleProgress",
            vec![json!({ "today": local_day(), "limit": 10 })],
        );
        let first = !self.started;
        self.started = true;
        cx.spawn(async move |this, cx| {
            let (data, db, progress) = (data.await, db.await, progress.await);
            let _ = this.update(cx, |this, cx| {
                if let Ok(data) = data {
                    this.accounts = data["accounts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|account| account["connected"] == json!(true))
                        .filter_map(|account| account["username"].as_str().map(str::to_string))
                        .collect();
                }
                match db {
                    Ok(db) => this.db = db,
                    Err(cause) => this.db_error = Some(cause.message),
                }
                if let Ok(progress) = progress {
                    this.progress = progress;
                }
                if first || this.session.phase() == "idle" {
                    this.load_next(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /* ── The session ──────────────────────────────────────────────────── */

    fn mode(cx: &App) -> String {
        crate::prefs::get::<String>(cx, MODE, "practice".into())
    }

    /// The connected account rated puzzles go to (`pickConnectedAccount`).
    fn account(&self, cx: &App) -> String {
        let preferred = crate::prefs::get::<String>(cx, ACCOUNT, String::new());
        let accounts: Vec<Value> = self
            .accounts
            .iter()
            .map(|username| json!({ "username": username, "connected": true }))
            .collect();
        rules(
            "connectedAccountIndex",
            &[json!(accounts), json!(preferred)],
        )
        .ok()
        .filter(|index| !index.is_null())
        .map(|index| crate::game::whole(&index))
        .and_then(|index| self.accounts.get(index as usize).cloned())
        .unwrap_or_default()
    }

    fn selection(&self, cx: &App) -> Value {
        json!({
            "account": self.account(cx),
            "angle": crate::prefs::get::<String>(cx, ANGLE, "mix".into()),
            "difficulty": crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into()),
            "color": crate::prefs::get::<String>(cx, COLOR, "random".into()),
            "mode": Self::mode(cx),
        })
    }

    fn load_next(&mut self, cx: &mut Context<Self>) {
        let step = self.session.apply(
            "puzzleSessionLoadBegin",
            json!({ "selection": self.selection(cx), "online": true }),
        );
        self.follow(step, "puzzleSessionLoadSettled", cx);
        self.sync_board(cx);
    }

    /// Run the transitions' host calls in order until one asks for nothing more.
    fn follow(&mut self, step: Step, settle: &'static str, cx: &mut Context<Self>) {
        if let Some(record) = step.record {
            self.record(record, cx);
        }
        let Some(call) = step.call else {
            return;
        };
        let request = cx
            .global::<AppCore>()
            .call(&call.method, vec![call.args.clone()]);
        cx.spawn(async move |this, cx| {
            let reply = match request.await {
                Ok(ok) => json!({ "ok": ok }),
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "puzzles",
                        &format!("{} failed: {}", call.method, cause.message),
                    );
                    json!({ "error": cause.message })
                }
            };
            let _ = this.update(cx, |this, cx| {
                let next = this.session.apply(
                    settle,
                    json!({
                        "resume": call.resume,
                        "outcome": reply,
                        "selection": this.selection(cx),
                    }),
                );
                this.follow(next, settle, cx);
                this.sync_board(cx);
            });
        })
        .detach();
    }

    /// Keep an attempt in the local log; a failed write never interrupts training.
    fn record(&mut self, attempt: Value, cx: &mut Context<Self>) {
        let save = cx
            .global::<AppCore>()
            .call("recordPuzzleAttempt", vec![attempt]);
        cx.spawn(async move |this, cx| match save.await {
            Ok(progress) => {
                let _ = this.update(cx, |this, cx| {
                    this.progress = progress;
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "puzzles",
                &format!("recording the attempt failed: {}", cause.message),
            ),
        })
        .detach();
    }

    fn report(&mut self, win: bool, cx: &mut Context<Self>) {
        let detail = json!({
            "revealed": self.revealed,
            "hinted": self.hint_used,
            "total": self.on_board.as_ref().map_or(0, OnBoard::player_moves),
        });
        let step = self.session.apply(
            "puzzleSessionReportBegin",
            json!({ "win": win, "detail": detail, "selection": self.selection(cx) }),
        );
        self.follow(step, "puzzleSessionReportSettled", cx);
    }

    /* ── The board ────────────────────────────────────────────────────── */

    /// A new puzzle (a new key) starts the board afresh (`begin`).
    fn sync_board(&mut self, cx: &mut Context<Self>) {
        let ready = self.session.phase() == "ready";
        let key = self.session.puzzle_key();
        match self.session.current().cloned() {
            Some(puzzle) if ready && (self.on_board.is_none() || key != self.shown_key) => {
                self.generation += 1;
                self.timers.clear();
                self.on_board = Some(OnBoard::start(&puzzle));
                self.shown_key = key;
                self.waiting = false;
                self.outcome = self.session.state["attemptOutcome"].as_bool();
                self.mistakes = u32::from(self.outcome == Some(false));
                self.revealed = false;
                self.hinted = false;
                self.hint_used = false;
                self.feedback = Feedback::None;
                self.flash = false;
                self.move_message = None;
            }
            _ if !ready => self.on_board = None,
            _ => {}
        }
        self.redraw(cx);
    }

    fn status(&self) -> &str {
        match &self.on_board {
            Some(_) if self.revealed => "revealed",
            Some(board) => board.status(),
            None => "playing",
        }
    }

    /// The player may move: not while the opponent replies or the solution plays itself out.
    fn playing(&self) -> bool {
        self.on_board
            .as_ref()
            .is_some_and(|board| board.status() == "playing")
            && !self.waiting
            && !self.revealed
    }

    fn orientation(&self) -> Color {
        let color = self
            .on_board
            .as_ref()
            .map_or(Color::White, OnBoard::player_color);
        if self.flipped { !color } else { color }
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let Some(on_board) = &self.on_board else {
            cx.notify();
            return;
        };
        let position =
            kchess_domain::rules::setup_start("standard", on_board.fen()).unwrap_or_default();
        let movable = self.playing().then(|| on_board.player_color());
        let mut state =
            BoardState::of(&position, self.orientation(), movable, on_board.last_move());
        if on_board.status() == "playing"
            && let Some((from, to)) = on_board.next_squares()
        {
            if self.revealed {
                state.arrows = vec![Arrow::solution(from, to)];
            } else if self.hinted {
                state.hint = Some(from);
            }
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// The first verdict on the puzzle, sent once.
    fn decide(&mut self, win: bool, cx: &mut Context<Self>) {
        if self.outcome.is_some() {
            return;
        }
        self.outcome = Some(win);
        self.report(win, cx);
    }

    fn on_move(&mut self, uci: &str, cx: &mut Context<Self>) {
        if !self.playing() {
            self.redraw(cx);
            return;
        }
        let Some(board) = &mut self.on_board else {
            return;
        };
        let index = board.index() / 2;
        let (correct, reply) = board.player_move(uci);
        let session_move =
            |correct: bool| json!({ "index": index, "correct": correct, "uci": uci });
        if !correct {
            self.mistakes += 1;
            self.feedback = Feedback::Wrong;
            self.flash = true;
            self.hinted = false;
            let generation = self.generation;
            self.timers.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FLASH).await;
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.flash = false;
                        cx.notify();
                    }
                });
            }));
            let step = self.session.apply("puzzleSessionMove", session_move(false));
            self.follow(step, "puzzleSessionLoadSettled", cx);
            self.decide(false, cx);
            self.redraw(cx);
            return;
        }
        self.feedback = Feedback::Good;
        self.hinted = false;
        crate::sound::move_sound(&board.last_san(), cx);
        let solved = board.status() == "solved";
        let step = self.session.apply("puzzleSessionMove", session_move(true));
        self.follow(step, "puzzleSessionLoadSettled", cx);
        if solved {
            let clean = self.mistakes == 0 && !self.revealed;
            self.decide(clean, cx);
            crate::sound::play(crate::sound::Sound::GenericNotify, cx);
            self.done(cx);
        } else if let Some(reply) = reply {
            self.reply(reply, cx);
        }
        self.redraw(cx);
    }

    fn reply(&mut self, uci: String, cx: &mut Context<Self>) {
        self.waiting = true;
        let generation = self.generation;
        self.timers.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPLY_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if let Some(board) = &mut this.on_board {
                    board.opponent_reply(&uci);
                    crate::sound::move_sound(&board.last_san(), cx);
                }
                this.waiting = false;
                this.redraw(cx);
            });
        }));
    }

    /// The puzzle is over: count and log it, and go on to the next one after a solve when asked.
    fn done(&mut self, cx: &mut Context<Self>) {
        let step = self.session.apply("puzzleSessionEnd", json!({}));
        self.follow(step, "puzzleSessionLoadSettled", cx);
        let solved = self
            .on_board
            .as_ref()
            .is_some_and(|board| board.status() == "solved")
            && !self.revealed;
        let auto_next = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .puzzle_auto_next;
        if solved && auto_next {
            let key = self.session.puzzle_key();
            self.timers.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTO_NEXT_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    // Moving on by hand, or changing what to train, already replaced the puzzle.
                    if this.session.puzzle_key() == key && this.session.phase() == "ready" {
                        this.load_next(cx);
                    }
                });
            }));
        }
    }

    /// Light up the piece to move, or hide it again.
    fn hint(&mut self, cx: &mut Context<Self>) {
        if self.status() != "playing" {
            return;
        }
        self.hinted = !self.hinted;
        if self.hinted {
            self.hint_used = true;
        }
        self.redraw(cx);
    }

    /// Play out the rest of the solution; the puzzle then counts as failed.
    fn show_solution(&mut self, cx: &mut Context<Self>) {
        if self.status() != "playing" {
            return;
        }
        self.revealed = true;
        self.decide(false, cx);
        self.redraw(cx);
        let generation = self.generation;
        self.timers.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SOLUTION_STEP).await;
                let reply = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return Err(());
                        }
                        let Some(board) = &mut this.on_board else {
                            return Err(());
                        };
                        if board.status() != "playing" {
                            return Err(());
                        }
                        let Some(uci) = board.solution(board.index()) else {
                            return Err(());
                        };
                        let (correct, reply) = board.player_move(&uci);
                        if !correct {
                            return Err(());
                        }
                        crate::sound::move_sound(&board.last_san(), cx);
                        this.redraw(cx);
                        Ok(reply)
                    })
                    .unwrap_or(Err(()));
                let Ok(reply) = reply else { break };
                if let Some(reply) = reply {
                    cx.background_executor().timer(SOLUTION_REPLY).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Some(board) = &mut this.on_board {
                            board.opponent_reply(&reply);
                            crate::sound::move_sound(&board.last_san(), cx);
                        }
                        this.redraw(cx);
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.done(cx);
                    this.redraw(cx);
                }
            });
        }));
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        let Some(board) = &self.on_board else { return };
        if text.is_empty() || !self.playing() {
            return;
        }
        let setup = json!({ "variant": "standard", "fen": board.fen() });
        let typed = kchess_domain::rules::setup_start("standard", board.fen())
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        match typed {
            Some(uci) => {
                self.move_message = None;
                self.move_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.on_move(&uci, cx);
            }
            None => {
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3.".into());
                cx.notify();
            }
        }
    }

    fn choose_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        crate::prefs::set(cx, MODE, mode);
        self.load_next(cx);
    }

    /// Train another theme, as choosing it in the select does.
    fn choose_angle(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, ANGLE, key);
        let index = self
            .theme_list
            .iter()
            .position(|theme| theme.key == key)
            .unwrap_or(0);
        self.theme_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(index)), window, cx)
        });
        self.load_next(cx);
    }

    fn choose_difficulty(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, DIFFICULTY, key);
        let index = DIFFICULTIES
            .iter()
            .position(|(value, _)| *value == key)
            .unwrap_or(2);
        self.difficulty_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(index)), window, cx)
        });
        self.load_next(cx);
    }

    /// `startOpening`: remember the opening an opening page asked for puzzles of, as the theme to
    /// train; a page made after this starts with it.
    pub fn choose_opening(key: &str, name: &str, cx: &mut App) {
        crate::prefs::set(
            cx,
            OPENING,
            OpeningAngle {
                key: key.to_string(),
                name: name.to_string(),
            },
        );
        crate::prefs::set(cx, ANGLE, key);
        crate::prefs::set(cx, TAB, "train");
    }

    /// The opening chosen with [`Self::choose_opening`], on a page made before it.
    pub fn opening_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_list = theme_items(cx);
        let names: Vec<SharedString> = self
            .theme_list
            .iter()
            .map(|theme| SharedString::from(theme.name.clone()))
            .collect();
        self.theme_select
            .update(cx, |state, cx| state.set_items(names, window, cx));
        let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
        self.choose_angle(&angle, window, cx);
    }

    fn reset_session(&mut self, cx: &mut Context<Self>) {
        let step = self.session.apply("puzzleSessionResetSession", json!({}));
        self.follow(step, "puzzleSessionLoadSettled", cx);
        cx.notify();
    }

    /* ── The downloaded database ──────────────────────────────────────── */

    fn install_db(&mut self, cx: &mut Context<Self>) {
        self.db_error = None;
        self.db_progress = Some(json!({ "phase": "downloading", "received": 0, "kept": 0 }));
        cx.notify();
        let install = cx.global::<AppCore>().call("puzzleDbInstall", Vec::new());
        cx.spawn(async move |this, cx| {
            let result = install.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(db) => this.db = db,
                    Err(cause) => this.db_error = Some(cause.message),
                }
                let phase = this
                    .db_progress
                    .as_ref()
                    .and_then(|progress| progress["phase"].as_str().map(str::to_string));
                if matches!(phase.as_deref(), Some("done" | "cancelled") | None)
                    || this.db_error.is_none()
                {
                    this.db_progress = None;
                }
                if this.session.phase() == "nodb" {
                    this.load_next(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel_db(&mut self, cx: &mut Context<Self>) {
        let cancel = cx.global::<AppCore>().call("puzzleDbCancel", Vec::new());
        cx.background_executor()
            .spawn(async move {
                let _ = cancel.await;
            })
            .detach();
    }

    fn ask_delete_db(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let page = page.clone();
            dialog
                .title("Delete the puzzle database?")
                .description(
                    "This removes the puzzles stored on this computer. Storm, Streak, Rush and offline puzzles stop working until you download it again. Your scores are kept.",
                )
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.delete_db(cx));
                    true
                })
        });
    }

    fn delete_db(&mut self, cx: &mut Context<Self>) {
        self.db_error = None;
        let delete = cx.global::<AppCore>().call("puzzleDbDelete", Vec::new());
        cx.spawn(async move |this, cx| {
            let result = delete.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(db) => {
                        this.db = db;
                        this.db_progress = None;
                    }
                    Err(cause) => this.db_error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn db_busy(&self) -> bool {
        self.db["busy"] == json!(true)
            || self.db_progress.as_ref().is_some_and(|progress| {
                matches!(
                    progress["phase"].as_str(),
                    Some("downloading" | "importing")
                )
            })
    }
}

/* ── Rendering ────────────────────────────────────────────────────────── */

impl PuzzlesPage {
    /// `PuzzleDbCard.vue`: download, see and delete the puzzles Downloaded mode plays from.
    fn render_db_card(&self, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let hint = |text: String| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(text)
        };
        let body: AnyElement = if self.db_busy() {
            let progress = self.db_progress.clone().unwrap_or(Value::Null);
            let received = progress["received"].as_f64().unwrap_or_default();
            let total = progress["total"].as_f64();
            let text = match progress["phase"].as_str() {
                Some("importing") => "Saving puzzles…".to_string(),
                Some("failed") => progress["message"]
                    .as_str()
                    .unwrap_or("The download failed.")
                    .to_string(),
                _ => format!(
                    "Downloading… {}{}",
                    format_bytes(received),
                    total
                        .map(|total| format!(" of {}", format_bytes(total)))
                        .unwrap_or_default()
                ),
            };
            let kept = progress["kept"].as_f64().unwrap_or_default();
            let share = total
                .filter(|total| *total > 0.)
                .map(|total| (received / total).min(1.));
            v_flex()
                .gap(px(8.))
                .child(
                    div()
                        .h(px(8.))
                        .w_full()
                        .rounded_full()
                        .bg(palette.accented)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(palette.primary)
                                .w(relative(share.unwrap_or(0.1) as f32)),
                        ),
                )
                .child(hint(if kept > 0. {
                    format!("{text} · {} puzzles picked so far", format_count(kept))
                } else {
                    text
                }))
                .child(
                    h_flex().child(
                        Button::new("db-cancel")
                            .outline()
                            .small()
                            .icon(icon("x"))
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_db(cx))),
                    ),
                )
                .into_any_element()
        } else if self.db["installed"] == json!(true) {
            let mut line = format!(
                "{} puzzles · {} on disk",
                format_count(self.db["count"].as_f64().unwrap_or_default()),
                format_bytes(self.db["bytes"].as_f64().unwrap_or_default())
            );
            if let Some(at) = self.db["importedAt"].as_f64() {
                use chrono::TimeZone as _;
                if let Some(day) = chrono::Local.timestamp_millis_opt(at as i64).single() {
                    line.push_str(&format!(" · downloaded {}", day.format("%-m/%-d/%Y")));
                }
            }
            v_flex()
                .gap(px(8.))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .text_size(px(14.))
                        .child(
                            icon("circle-check")
                                .size(px(16.))
                                .text_color(palette.success),
                        )
                        .child(line),
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .flex_wrap()
                        .child(
                            Button::new("db-refresh")
                                .outline()
                                .small()
                                .icon(icon("refresh-cw"))
                                .label("Download a fresh sample (~300 MB)")
                                .on_click(cx.listener(|this, _, _, cx| this.install_db(cx))),
                        )
                        .child(
                            Button::new("db-delete")
                                .danger()
                                .small()
                                .icon(icon("trash-2"))
                                .label("Delete")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.ask_delete_db(window, cx)
                                })),
                        ),
                )
                .into_any_element()
        } else {
            v_flex()
                .gap(px(8.))
                .child(hint(
                    "About 300 MB download · keeps 100,000 puzzles.".into(),
                ))
                .child(
                    h_flex().child(
                        Button::new("db-install")
                            .primary()
                            .icon(icon("download"))
                            .label("Download puzzle database")
                            .on_click(cx.listener(|this, _, _, cx| this.install_db(cx))),
                    ),
                )
                .into_any_element()
        };
        v_flex()
            .w_full()
            .max_w(px(440.))
            .gap(px(4.))
            .p(px(16.))
            .border_1()
            .border_color(palette.border)
            .rounded(px(12.))
            .bg(palette.bg)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight(650.))
                    .text_color(palette.highlighted)
                    .child("Puzzle database"),
            )
            .child(hint("For offline puzzles, Storm, Streak and Rush.".into()))
            .child(div().mt(px(8.)).child(body))
            .children(self.db_error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
    }

    /// What stands in for the board while there is no puzzle to play.
    fn render_placeholder(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let empty = |glyph: Icon, title: String, description: Option<String>| {
            v_flex()
                .items_center()
                .gap(px(10.))
                .child(glyph.size(px(24.)).text_color(palette.text_muted))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(title),
                )
                .children(description.map(|description| {
                    div()
                        .text_size(px(14.))
                        .text_center()
                        .text_color(palette.text_muted)
                        .child(description)
                }))
        };
        let other = |cx: &mut Context<Self>| {
            Button::new("other-mode")
                .outline()
                .label("Practice instead")
                .on_click(cx.listener(|this, _, _, cx| this.choose_mode("practice", cx)))
        };
        let content: AnyElement = match self.session.phase() {
            "noaccount" => empty(
                icon("user-round-plus"),
                "Connect a Lichess account for rated puzzles".into(),
                Some("Connect Lichess or switch to Practice.".into()),
            )
            .child(other(cx))
            .into_any_element(),
            "reconnect" => empty(
                icon("key-round"),
                "Lichess needs your permission for puzzles".into(),
                Some(format!(
                    "Sign in to @{} on lichess.org, then reconnect to enable rated puzzles.",
                    self.account(cx)
                )),
            )
            .child(other(cx))
            .into_any_element(),
            "nodb" => self.render_db_card(cx, palette).into_any_element(),
            "exhausted" => {
                let difficulty = crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into());
                let label = DIFFICULTIES
                    .iter()
                    .find(|(key, _)| *key == difficulty)
                    .map_or("Normal", |(_, label)| label);
                let index = DIFFICULTIES
                    .iter()
                    .position(|(key, _)| *key == difficulty)
                    .unwrap_or(2);
                let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
                let mode = Self::mode(cx);
                let mut actions: Vec<Button> = Vec::new();
                if index > 0 {
                    let (key, easier) = DIFFICULTIES[index - 1];
                    actions.push(
                        Button::new("try-easier")
                            .icon(icon("chevrons-down"))
                            .label(format!("Try {easier}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_difficulty(key, window, cx)
                            })),
                    );
                }
                if angle != "mix" {
                    actions.push(
                        Button::new("try-mix")
                            .icon(icon("shuffle"))
                            .label("Try Healthy mix")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_angle("mix", window, cx)
                            })),
                    );
                }
                if mode != "offline" {
                    actions.push(
                        Button::new("try-offline")
                            .icon(icon("hard-drive"))
                            .label("Use downloaded puzzles")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_mode("offline", cx)),
                            ),
                    );
                }
                let actions: Vec<Button> = actions
                    .into_iter()
                    .enumerate()
                    .map(|(order, action)| {
                        if order == 0 {
                            action.primary()
                        } else {
                            action.outline()
                        }
                    })
                    .collect();
                empty(
                    icon("puzzle"),
                    format!("No more {label} puzzles for this theme"),
                    None,
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .flex_wrap()
                        .justify_center()
                        .children(actions),
                )
                .into_any_element()
            }
            "error" => empty(
                icon("triangle-alert"),
                "Couldn’t get a puzzle".into(),
                self.session.state["trainError"]
                    .as_str()
                    .map(str::to_string),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("try-again")
                            .primary()
                            .icon(icon("rotate-cw"))
                            .label("Try again")
                            .on_click(cx.listener(|this, _, _, cx| this.load_next(cx))),
                    )
                    .when(Self::mode(cx) != "offline", |row| {
                        row.child(
                            Button::new("error-offline")
                                .outline()
                                .label("Use downloaded puzzles")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.choose_mode("offline", cx)),
                                ),
                        )
                    }),
            )
            .into_any_element(),
            _ => h_flex()
                .gap(px(8.))
                .text_color(palette.text_muted)
                .child(icon("loader-circle").size(px(16.)))
                .child("Finding a puzzle…")
                .into_any_element(),
        };
        content
    }

    /// `PuzzleStatusPanel.vue`: what to do, how it went, and the buttons that help.
    fn render_status(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let Some(board) = &self.on_board else {
            return div().into_any_element();
        };
        let status = self.status();
        let color = if board.player_color() == Color::White {
            "white"
        } else {
            "black"
        };
        let (tone, glyph, title) = match status {
            "revealed" => (
                Tone::Loss,
                icon("eye"),
                "Solution shown — not counted as solved".to_string(),
            ),
            "solved" if self.outcome == Some(true) => {
                (Tone::Win, icon("trophy"), "Puzzle solved!".to_string())
            }
            "solved" => (
                Tone::Plain,
                icon("check"),
                "Solved after a mistake — not counted".to_string(),
            ),
            "failed" => (
                Tone::Loss,
                icon("x"),
                "Not the move — puzzle over".to_string(),
            ),
            _ if self.feedback == Feedback::Wrong => (
                Tone::Loss,
                icon("x"),
                "That’s not the move — try again".to_string(),
            ),
            _ if self.waiting => (Tone::Plain, icon("hourglass"), "Good move…".to_string()),
            _ => (
                Tone::Plain,
                icon("mouse-pointer-click"),
                format!("Find the best move for {color}"),
            ),
        };
        let playing = status == "playing";
        let finished = matches!(status, "solved" | "revealed" | "failed");
        let themes = &self.theme_list;
        let facts = finished.then(|| {
            let puzzle = &board.puzzle;
            v_flex()
                .gap(px(6.))
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(64.))
                                .text_color(palette.text_muted)
                                .child("Rating"),
                        )
                        .child(format!("{}", puzzle["rating"].as_f64().unwrap_or_default())),
                )
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(64.))
                                .text_color(palette.text_muted)
                                .child("Themes"),
                        )
                        .child(
                            h_flex().flex_1().flex_wrap().gap(px(4.)).children(
                                puzzle["themes"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .map(|theme| {
                                        div()
                                            .px(px(6.))
                                            .py(px(1.))
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(palette.border_accented)
                                            .bg(palette.elevated)
                                            .text_size(px(12.))
                                            .child(theme_name(theme, themes))
                                    }),
                            ),
                        ),
                )
        });
        v_flex()
            .gap(px(12.))
            .child(widgets::status_banner(
                tone,
                glyph,
                title.into(),
                None,
                None,
                palette,
            ))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("puzzle-hint")
                            .when(self.hinted, |button| button.primary())
                            .when(!self.hinted, |button| button.outline())
                            .icon(icon("lightbulb"))
                            .label("Hint")
                            .disabled(!playing)
                            .on_click(cx.listener(|this, _, _, cx| this.hint(cx))),
                    )
                    .child(
                        Button::new("puzzle-solution")
                            .outline()
                            .icon(icon("eye"))
                            .label("Solution")
                            .disabled(!playing)
                            .on_click(cx.listener(|this, _, _, cx| this.show_solution(cx))),
                    )
                    .child(
                        Button::new("puzzle-flip")
                            .outline()
                            .icon(icon("arrow-up-down"))
                            .tooltip("Flip board")
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.flipped = !this.flipped;
                                this.redraw(cx);
                            })),
                    )
                    .child(
                        Button::new("puzzle-next")
                            .when(finished, |button| button.primary())
                            .when(!finished, |button| button.outline())
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child("Next")
                                    .child(icon("arrow-right").size(px(14.))),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.load_next(cx))),
                    ),
            )
            .children(facts)
            .into_any_element()
    }

    /// The session's tally, the day streak and the latest attempts.
    fn render_session(&self, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let session = &self.session.state["session"];
        let count = |key: &str| session[key].as_f64().unwrap_or_default();
        let parts = [
            ("solved", "solved", palette.success, true),
            ("hinted", "with hint", palette.success, false),
            ("partial", "partial", palette.info, false),
            ("missed", "missed", palette.error, true),
            ("revealed", "revealed", palette.text, false),
        ];
        let tried = count("movesTried");
        let rating = self.session.state["rating"].as_f64();
        let day_streak = self.progress["dayStreak"].as_f64().unwrap_or_default();
        let mut line = h_flex()
            .flex_wrap()
            .gap_x(px(12.))
            .gap_y(px(4.))
            .text_size(px(13.))
            .text_color(palette.text_muted);
        for (key, label, tone, always) in parts {
            let value = count(key);
            if always || value > 0. {
                line = line.child(
                    h_flex()
                        .gap(px(4.))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(tone)
                                .child(format!("{value}")),
                        )
                        .child(label),
                );
            }
        }
        if tried > 0. {
            line = line.child(
                h_flex()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(palette.text)
                            .child(format!("{}", count("movesFound"))),
                    )
                    .child(format!("/{tried} moves right")),
            );
        }
        if let Some(rating) = rating.filter(|_| Self::mode(cx) == "rated") {
            let change = count("ratingChange");
            line = line.child(format!(
                "rating {rating}{}",
                if change != 0. {
                    format!(" ({}{change})", if change > 0. { "+" } else { "" })
                } else {
                    String::new()
                }
            ));
        }
        line = line.child(
            h_flex()
                .ml_auto()
                .gap(px(8.))
                .when(day_streak >= 2., |row| {
                    row.child(
                        h_flex()
                            .gap(px(2.))
                            .text_color(palette.warning)
                            .child(icon("flame").size(px(14.)))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(palette.text)
                                    .child(format!("{day_streak}")),
                            ),
                    )
                })
                .child(
                    Button::new("puzzle-reset")
                        .text()
                        .xsmall()
                        .label("Reset")
                        .on_click(cx.listener(|this, _, _, cx| this.reset_session(cx))),
                ),
        );
        // The latest attempts, oldest first, so the newest sits at the end of the strip.
        let recent: Vec<Value> = self.progress["recent"]
            .as_array()
            .map(|recent| recent.iter().rev().cloned().collect())
            .unwrap_or_default();
        let strip = (!recent.is_empty()).then(|| {
            h_flex().flex_wrap().gap(px(4.)).mt(px(8.)).children(
                recent.into_iter().enumerate().map(|(index, attempt)| {
                    let outcome = attempt["outcome"].as_str().unwrap_or("missed").to_string();
                    let (glyph, fg, bg) = match outcome.as_str() {
                        "solved" => (icon("check"), palette.success, tint(palette.success, 10.)),
                        "hinted" => (icon("lightbulb"), palette.success, palette.bg),
                        "partial" => (
                            icon("chart-no-axes-column"),
                            palette.info,
                            tint(palette.info, 10.),
                        ),
                        "revealed" => (icon("eye"), palette.toned, palette.elevated),
                        _ => (icon("x"), palette.error, tint(palette.error, 10.)),
                    };
                    let label = format!(
                        "{}, {}/{} moves{}, rated {}",
                        match outcome.as_str() {
                            "solved" => "Solved",
                            "hinted" => "With hint",
                            "partial" => "Partial",
                            "revealed" => "Revealed",
                            _ => "Missed",
                        },
                        attempt["found"].as_f64().unwrap_or_default(),
                        attempt["total"].as_f64().unwrap_or_default(),
                        match attempt["wrong"].as_f64().unwrap_or_default() {
                            0. => String::new(),
                            wrong => format!(" · {wrong} wrong"),
                        },
                        attempt["puzzle"]["rating"].as_f64().unwrap_or_default()
                    );
                    div()
                        .id(("attempt", index))
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .bg(bg)
                        .border_1()
                        .border_color(tint(fg, 40.))
                        .text_color(fg)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        })
                        .child(glyph.size(px(14.)))
                }),
            )
        });
        v_flex().child(line).children(strip)
    }

    fn render_train(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let board_size = self.layout.board_size(window);
        let mode = Self::mode(cx);
        let ready = self.on_board.is_some() && self.session.phase() == "ready";
        let fen = self
            .on_board
            .as_ref()
            .map(|board| board.fen().to_string())
            .unwrap_or_default();
        let set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        let orientation = self.orientation();
        let entry = widgets::move_entry(&self.move_input, true, palette)
            .h(px(36.))
            .flex_1();
        let board_column = if ready {
            v_flex()
                .w(board_size)
                .flex_none()
                .gap(px(4.))
                .child(h_flex().h(px(18.)).children(widgets::material(
                    &fen,
                    orientation == Color::Black,
                    &set,
                    palette,
                )))
                .child(
                    div()
                        .rounded(px(6.))
                        .border_3()
                        .border_color(if self.flash {
                            tint(palette.loss, 55.)
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .m(px(-3.))
                        .child(self.board.clone()),
                )
                .child(
                    h_flex()
                        .h(px(36.))
                        .gap(px(12.))
                        .children(widgets::material(
                            &fen,
                            orientation == Color::White,
                            &set,
                            palette,
                        ))
                        .child(div().flex_1().pl(px(60.)).child(entry)),
                )
                .children(self.move_message.clone().map(|message| {
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(message)
                }))
                .into_any_element()
        } else {
            v_flex()
                .w(board_size)
                .h(board_size)
                .flex_none()
                .items_center()
                .justify_center()
                .p(px(24.))
                .border_1()
                .border_color(palette.border)
                .rounded(px(14.))
                .bg(palette.elevated)
                .child(self.render_placeholder(cx, palette))
                .into_any_element()
        };
        let field = |label: &'static str, control: AnyElement| {
            v_flex()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(label),
                )
                .child(control)
        };
        let page = cx.entity().downgrade();
        let mode_tabs = widgets::pills(
            "puzzle-mode",
            MODES
                .iter()
                .map(|(key, label)| Pill {
                    icon: Some(match *key {
                        "practice" => icon("dumbbell"),
                        "offline" => icon("hard-drive"),
                        _ => icon("cloud-upload"),
                    }),
                    label: (*label).into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            MODES.iter().position(|(key, _)| *key == mode).unwrap_or(0),
            move |index, _, cx| {
                let _ = page.update(cx, |page, cx| page.choose_mode(MODES[index].0, cx));
            },
            false,
            palette,
        );
        let is_retry = self.session.state["isRetry"] == json!(true);
        let account = self.account(cx);
        let source = if is_retry {
            Some("Retry · unrated".to_string())
        } else if mode == "rated" {
            Some(if account.is_empty() {
                "Rated".to_string()
            } else {
                format!("Rated · @{account}")
            })
        } else {
            None
        };
        let rating_text = self.session.state["lastReport"].as_object().map(|report| {
            if let Some(failed) = report.get("failed").and_then(Value::as_str) {
                (failed.to_string(), true)
            } else if let Some(diff) = report.get("ratingDiff").and_then(Value::as_f64) {
                let rating = self.session.state["rating"]
                    .as_f64()
                    .map(|rating| format!(" → {rating}"))
                    .unwrap_or_default();
                (
                    format!(
                        "Lichess puzzle rating {}{diff}{rating}",
                        if diff >= 0. { "+" } else { "" }
                    ),
                    false,
                )
            } else {
                ("Sent to Lichess.".to_string(), false)
            }
        });
        let panel = widgets::side_panel(palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(field("Mode", mode_tabs.into_any_element()))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(match &source {
                        Some(source) => source.clone(),
                        None => mode_hint(&mode).to_string(),
                    }),
            )
            .child(field(
                "Theme",
                Select::new(&self.theme_select)
                    .small()
                    .accessibility_label("Theme")
                    .into_any_element(),
            ))
            .child(
                h_flex()
                    .gap(px(10.))
                    .child(
                        div().flex_1().child(field(
                            "Difficulty",
                            Select::new(&self.difficulty_select)
                                .small()
                                .accessibility_label("Difficulty")
                                .into_any_element(),
                        )),
                    )
                    .child(
                        div().flex_1().child(field(
                            "Play as",
                            Select::new(&self.color_select)
                                .small()
                                .disabled(mode == "offline")
                                .accessibility_label("Play as")
                                .into_any_element(),
                        )),
                    ),
            )
            .when(ready, |panel| {
                panel
                    .child(div().h(px(1.)).bg(palette.border))
                    .child(self.render_status(cx, palette))
                    .children(rating_text.map(|(text, failed)| {
                        div()
                            .text_size(px(12.))
                            .text_color(if failed {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            .child(text)
                    }))
            })
            .child(div().h(px(1.)).bg(palette.border))
            .child(self.render_session(cx, palette));
        self.layout
            .render("puzzle-train", board_column, panel, window, cx)
            .into_any_element()
    }
}

impl Render for PuzzlesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let tab = crate::prefs::get::<String>(cx, TAB, "train".into());
        let tabs = [
            ("train", "Train"),
            ("daily", "Daily"),
            ("rush", "Storm · Streak · Rush"),
            ("stats", "Lichess stats"),
            ("history", "History"),
        ];
        let tab_row = widgets::pills(
            "puzzle-tabs",
            tabs.iter()
                .map(|(key, label)| Pill {
                    icon: Some(match *key {
                        "train" => icon("puzzle"),
                        "daily" => icon("calendar-days"),
                        "rush" => icon("zap"),
                        "stats" => icon("chart-column"),
                        _ => icon("history"),
                    }),
                    label: (*label).into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            tabs.iter().position(|(key, _)| *key == tab).unwrap_or(0),
            move |index, window, cx| {
                crate::prefs::set(cx, TAB, tabs[index].0);
                window.refresh();
            },
            false,
            &palette,
        );
        let body = if tab == "train" {
            self.render_train(window, cx, &palette)
        } else {
            div()
                .px(side_padding(window))
                .pt(px(20.))
                .child(
                    widgets::card(&palette)
                        .items_center()
                        .py(px(40.))
                        .text_color(palette.text_muted)
                        .child("Use the Electron app for this tab for now."),
                )
                .into_any_element()
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .child(
                div()
                    .flex_none()
                    .px(side_padding(window))
                    .pt(crate::play_layout::PAGE_TOP)
                    .child(tab_row),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, format_count, theme_name, themes};

    #[test]
    fn sizes_and_counts_read_as_in_electron() {
        assert_eq!(format_bytes(0.), "0 B");
        assert_eq!(format_bytes(1536.), "1.5 KB");
        assert_eq!(format_bytes(300. * 1024. * 1024.), "300 MB");
        assert_eq!(format_count(100_000.), "100,000");
        assert_eq!(format_count(999.), "999");
    }

    #[test]
    fn theme_names_come_from_the_shared_list() {
        let list = themes();
        assert_eq!(list[0].name, "Healthy mix");
        assert_eq!(theme_name("mateIn2", &list), "Mate in 2");
        assert_eq!(theme_name("someNewTheme", &list), "Some New Theme");
    }
}
