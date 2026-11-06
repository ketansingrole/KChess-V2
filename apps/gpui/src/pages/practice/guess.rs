//! Guess the move: a game (a master game from the explorer, or a PGN) is played through and one
//! side's moves are guessed. Each guess scores against the move played, and otherwise against the
//! local engine (`kchess_domain::training::guess`); the other side's moves are played for you.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_domain::training::guess::{self, Eval, Grade, Judged};
use serde_json::{Value, json};
use shakmaty::{Color, Position as _};

use super::run;
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, ChessBoard};
use crate::core::{AppCore, CoreEvent};
use crate::game::tree::TreeNode;
use crate::game::view;
use crate::online::OnlineHandle;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, side_padding};
use crate::position_engine::PositionEngine;
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::widgets::{self, Pill, Tone};

const SIDE: &str = "kchess:guess-side";
/// The other side's move follows a guess after this long.
const REPLY: Duration = Duration::from_millis(450);
/// An engine look at a position stops at this depth, or after this long.
const PROBE_DEPTH: f64 = 14.;
const PROBE_TIME: Duration = Duration::from_millis(2000);
/// The engine's best lines looked at for a guess before searching the guess itself.
const LINES: usize = 3;

/// A game to guess through: its start, its moves and who played it.
#[derive(Clone)]
struct Game {
    setup: Value,
    moves: Vec<String>,
    sans: Vec<String>,
    white: String,
    black: String,
}

impl Game {
    /// The mainline of a standard game's PGN, or why it cannot be guessed.
    fn from_pgn(pgn: &str) -> Result<Game, &'static str> {
        if crate::game::pgn_variant_key(pgn) != Some("standard") {
            return Err("Only standard chess games can be guessed");
        }
        let root = TreeNode::from_pgn(pgn).ok_or("That PGN could not be read")?;
        let mut moves = Vec::new();
        let mut node = &root;
        while let Some(next) = node.children.first() {
            moves.push(next.uci.clone());
            node = next;
        }
        if moves.is_empty() {
            return Err("That game has no moves");
        }
        let setup = json!({ "variant": "standard", "fen": root.fen });
        let sans = view::sans(&setup, &moves);
        if sans.len() != moves.len() {
            return Err("That PGN could not be read");
        }
        let header = |key: &str| {
            root.headers
                .as_ref()
                .and_then(|headers| headers.get(key))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty() && *name != "?")
                .unwrap_or("?")
                .to_string()
        };
        Ok(Game {
            setup,
            moves,
            sans,
            white: header("White"),
            black: header("Black"),
        })
    }

    fn turn_at(&self, ply: usize) -> Color {
        view::turn(view::position_at(&self.setup, &self.moves, ply).as_ref())
    }
}

/// A scored guess at a move of the game.
struct Guessed {
    san: String,
    played: String,
    judged: Judged,
}

/// What the engine is looking at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    /// The position before the guess, at this ply: its best lines.
    Best(usize),
    /// The position after the guess being judged.
    Guess,
}

/// A line's score from the guessing side's point of view; lines are from White's.
fn eval_of(line: &Value, side: Color) -> Option<Eval> {
    let eval = if let Some(mate) = line["mate"].as_f64() {
        Eval::Mate(mate as i32)
    } else {
        Eval::Cp(line["cp"].as_f64()? as i32)
    };
    Some(eval.for_side(side == Color::White))
}

fn grade_text(grade: Grade) -> &'static str {
    match grade {
        Grade::Master => "The move played",
        Grade::Best => "As good as the best move",
        Grade::Good => "A good move",
        Grade::Fair => "Playable",
        Grade::Miss => "Not close",
    }
}

fn points_text(points: u32) -> String {
    format!("{points} {}", if points == 1 { "point" } else { "points" })
}

pub struct GuessMoves {
    game: Option<Game>,
    side: Color,
    /// The moves of the game on the board.
    ply: usize,
    /// A finished game's move looked at instead of its last.
    viewing: Option<usize>,
    guesses: Vec<Guessed>,
    /// A guess waiting for the engine's verdict.
    pending: Option<String>,
    probe: Option<Probe>,
    /// The engine's best lines for the position at a ply.
    best: Option<(usize, Vec<Value>)>,
    engine: PositionEngine,
    engine_ready: bool,
    error: Option<String>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_input: Entity<InputState>,
    pgn: Entity<TextareaState>,
    deadline: Option<Task<()>>,
    reply: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn engine_of(view: &mut GuessMoves) -> &mut PositionEngine {
    &mut view.engine
}

impl GuessMoves {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> GuessMoves {
        let board = cx.new(|_| {
            ChessBoard::new(view::board_state(
                &json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN }),
                &[],
                0,
                Color::White,
                None,
            ))
        });
        let move_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
        let pgn = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(6)
                .placeholder("1. e4 e5 2. Nf3 Nc6 …")
        });
        let events = cx.global::<AppCore>().events.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.guess(uci.clone(), cx);
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
            cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
                if event.name == "engine:analysis" {
                    this.engine_update(&event.payload, cx);
                }
            }),
        ];
        let side = if crate::prefs::get::<String>(cx, SIDE, "white".into()) == "black" {
            Color::Black
        } else {
            Color::White
        };
        let status = cx.global::<AppCore>().call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let ready =
                matches!(status.await, Ok(status) if status["ready"].as_bool() == Some(true));
            let _ = this.update(cx, |this, cx| {
                this.engine_ready = ready;
                cx.notify();
            });
        })
        .detach();
        GuessMoves {
            game: None,
            side,
            ply: 0,
            viewing: None,
            guesses: Vec::new(),
            pending: None,
            probe: None,
            best: None,
            engine: PositionEngine::default(),
            engine_ready: false,
            error: None,
            board,
            layout: PlayLayout::default(),
            move_input,
            pgn,
            deadline: None,
            reply: None,
            _subscriptions: subscriptions,
        }
    }

    /// Guess through a game's PGN as `side` (the side chosen last when none is given).
    pub fn load(&mut self, pgn: &str, side: Option<Color>, cx: &mut Context<Self>) -> bool {
        match Game::from_pgn(pgn) {
            Ok(game) => {
                if let Some(side) = side {
                    self.set_side(side, cx);
                }
                self.error = None;
                self.game = Some(game);
                self.restart(cx);
                true
            }
            Err(problem) => {
                self.error = Some(problem.into());
                cx.notify();
                false
            }
        }
    }

    fn set_side(&mut self, side: Color, cx: &mut Context<Self>) {
        self.side = side;
        crate::prefs::set(
            cx,
            SIDE,
            if side == Color::White {
                "white"
            } else {
                "black"
            },
        );
    }

    /// The game from its first move again.
    fn restart(&mut self, cx: &mut Context<Self>) {
        self.stop_engine(cx);
        self.ply = 0;
        self.viewing = None;
        self.guesses.clear();
        self.pending = None;
        self.reply = None;
        self.advance(cx);
    }

    /// Back to choosing a game.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.stop_engine(cx);
        self.reply = None;
        self.game = None;
        self.guesses.clear();
        self.pending = None;
        cx.notify();
    }

    fn finished(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|game| self.ply >= game.moves.len())
    }

    fn your_turn(&self) -> bool {
        self.pending.is_none()
            && self.game.as_ref().is_some_and(|game| {
                self.ply < game.moves.len() && game.turn_at(self.ply) == self.side
            })
    }

    /// Whether the engine can judge guesses now: installed, and no live Lichess game going on.
    fn engine_usable(&self, cx: &App) -> bool {
        self.engine_ready && cx.global::<OnlineHandle>().0.read(cx).assistance_allowed()
    }

    /// The next step at `ply`: the other side's move, the player's guess, or the end.
    fn advance(&mut self, cx: &mut Context<Self>) {
        let Some(game) = &self.game else {
            return;
        };
        if self.ply >= game.moves.len() {
            self.stop_engine(cx);
        } else if game.turn_at(self.ply) != self.side {
            let first = self.ply == 0;
            self.reply = Some(cx.spawn(async move |this, cx| {
                if !first {
                    cx.background_executor().timer(REPLY).await;
                }
                let _ = this.update(cx, |this, cx| this.other_move(cx));
            }));
        } else if self.engine_usable(cx) {
            self.start_probe(Probe::Best(self.ply), None, cx);
        }
        self.redraw(cx);
    }

    fn other_move(&mut self, cx: &mut Context<Self>) {
        let Some(san) = self.game.as_ref().and_then(|game| game.sans.get(self.ply)) else {
            return;
        };
        crate::sound::move_sound(&san.clone(), cx);
        self.ply += 1;
        self.advance(cx);
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = &self.game else {
            return;
        };
        let text = self.move_input.read(cx).value().trim().to_string();
        let typed = view::position_at(&game.setup, &game.moves, self.ply)
            .and_then(|position| view::typed_uci(&game.setup, &position, &text));
        if let Some(uci) = typed {
            self.move_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.guess(uci, cx);
        }
    }

    /// A guess at the move at `ply`: the move played scores at once; another waits for the engine.
    fn guess(&mut self, uci: String, cx: &mut Context<Self>) {
        let Some(played) = self
            .game
            .as_ref()
            .and_then(|game| game.moves.get(self.ply))
            .cloned()
        else {
            return self.redraw(cx);
        };
        if !self.your_turn() {
            return self.redraw(cx);
        }
        if guess::same_move(&uci, &played) || !self.engine_usable(cx) {
            self.stop_engine(cx);
            return self.judged(&uci, None, None, cx);
        }
        self.pending = Some(uci);
        if self.best.as_ref().is_some_and(|(ply, _)| *ply == self.ply) {
            self.judge_pending(cx);
        } else if self.probe.is_none() {
            self.start_probe(Probe::Best(self.ply), None, cx);
        }
        self.redraw(cx);
    }

    /// The best lines are known: score the guess from them, or search the position after it.
    fn judge_pending(&mut self, cx: &mut Context<Self>) {
        let (Some(uci), Some(game)) = (self.pending.clone(), self.game.clone()) else {
            return;
        };
        let lines = self.best.as_ref().map(|(_, lines)| lines.clone());
        let best = lines
            .as_ref()
            .and_then(|lines| lines.first())
            .and_then(|line| eval_of(line, self.side));
        let Some(best) = best else {
            return self.judged(&uci, None, None, cx);
        };
        let listed = lines.iter().flatten().find(|line| {
            line["pv"][0]
                .as_str()
                .is_some_and(|first| guess::same_move(first, &uci))
        });
        if let Some(line) = listed {
            let guessed = eval_of(line, self.side);
            return self.judged(&uci, Some(best), guessed, cx);
        }
        let mut history = game.moves[..self.ply].to_vec();
        history.push(uci.clone());
        let Some(after) = view::position_at(&game.setup, &history, history.len()) else {
            return self.judged(&uci, None, None, cx);
        };
        // A game the guess ends needs no search.
        if after.is_checkmate() {
            return self.judged(&uci, Some(best), Some(Eval::Mate(1)), cx);
        }
        if after.is_stalemate() || after.is_insufficient_material() {
            return self.judged(&uci, Some(best), Some(Eval::Cp(0)), cx);
        }
        self.start_probe(Probe::Guess, Some(history), cx);
    }

    /// Score a guess, show the move played, and go on.
    fn judged(
        &mut self,
        uci: &str,
        best: Option<Eval>,
        guessed: Option<Eval>,
        cx: &mut Context<Self>,
    ) {
        let Some(game) = &self.game else {
            return;
        };
        let Some(played) = game.moves.get(self.ply).cloned() else {
            return;
        };
        let mut history = game.moves[..self.ply].to_vec();
        history.push(uci.to_string());
        let san = view::sans(&game.setup, &history)
            .pop()
            .unwrap_or_else(|| uci.to_string());
        let played_san = game.sans.get(self.ply).cloned().unwrap_or_default();
        let judged = guess::judge(uci, &played, best, guessed);
        crate::sound::move_sound(&played_san, cx);
        self.guesses.push(Guessed {
            san,
            played: played_san,
            judged,
        });
        self.pending = None;
        self.best = None;
        self.ply += 1;
        self.advance(cx);
    }

    /// Ask the engine about the position at the game's `ply` (Best) or after `history` (Guess).
    fn start_probe(&mut self, probe: Probe, history: Option<Vec<String>>, cx: &mut Context<Self>) {
        let Some(game) = &self.game else {
            return;
        };
        let history = history.unwrap_or_else(|| game.moves[..self.ply].to_vec());
        let Some(position) = view::position_at(&game.setup, &history, history.len()) else {
            return;
        };
        let request = json!({
            "fen": kchess_domain::rules::make_fen(&position),
            "rootFen": game.setup["fen"],
            "moves": history,
            "lines": if probe == Probe::Guess { 1 } else { LINES },
            "infinite": true,
        });
        self.probe = Some(probe);
        self.engine.search(Some(request), engine_of, cx);
        // A look ends after a while even if the engine never reaches its depth.
        self.deadline = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PROBE_TIME).await;
            let _ = this.update(cx, |this, cx| this.probe_done(cx));
        }));
    }

    fn engine_update(&mut self, update: &Value, cx: &mut Context<Self>) {
        if self.probe.is_none() || !self.engine.receive(update) {
            return;
        }
        let reached = update["depth"].as_f64().unwrap_or_default() >= PROBE_DEPTH
            || update["done"] == json!(true);
        if reached || self.engine.error.is_some() {
            self.probe_done(cx);
        }
    }

    /// The engine has looked long enough: keep what it found and stop it.
    fn probe_done(&mut self, cx: &mut Context<Self>) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        let lines = self.engine.lines();
        self.stop_engine(cx);
        match probe {
            Probe::Best(ply) => {
                self.best = Some((ply, lines));
                if self.pending.is_some() && ply == self.ply {
                    self.judge_pending(cx);
                }
            }
            Probe::Guess => {
                let Some(uci) = self.pending.clone() else {
                    return;
                };
                let best = self
                    .best
                    .as_ref()
                    .and_then(|(_, lines)| lines.first())
                    .and_then(|line| eval_of(line, self.side));
                let guessed = lines.first().and_then(|line| eval_of(line, self.side));
                self.judged(&uci, best, guessed, cx);
            }
        }
        cx.notify();
    }

    fn stop_engine(&mut self, cx: &mut Context<Self>) {
        self.probe = None;
        self.deadline = None;
        if self.engine.wanted().is_some() {
            self.engine.search(None, engine_of, cx);
        }
    }

    /// Look at a finished game's move.
    fn view(&mut self, ply: usize, cx: &mut Context<Self>) {
        if self.finished() {
            self.viewing = Some(ply);
            self.redraw(cx);
        }
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let Some(game) = &self.game else {
            return cx.notify();
        };
        let ply = self.viewing.unwrap_or(self.ply);
        let movable = self.your_turn().then_some(self.side);
        let mut state = view::board_state(&game.setup, &game.moves, ply, self.side, movable);
        // The guess being judged, over the position it was made in.
        if let Some(uci) = &self.pending
            && let Some((from, to)) = crate::board::last_squares(uci)
        {
            state.arrows = vec![Arrow::hint(from, to)];
        }
        state.blindfold = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .blindfold;
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn start_from_field(&mut self, cx: &mut Context<Self>) {
        let text = self.pgn.read(cx).value().trim().to_string();
        if !text.is_empty() {
            self.load(&text, None, cx);
        }
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let Some(path) = paths.first() else {
                return;
            };
            let text = crate::shell::read_game_file(path);
            let _ = this.update_in(cx, |this, window, cx| match text {
                Some(text) => {
                    this.pgn
                        .update(cx, |field, cx| field.set_value(text.clone(), window, cx));
                    this.load(&text, None, cx);
                }
                None => {
                    this.error = Some("Choose a PGN file under 2 MB".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn side_pills(
        &self,
        id: &'static str,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let page = cx.entity().downgrade();
        widgets::pills(
            id,
            ["White", "Black"]
                .into_iter()
                .map(|label| Pill {
                    icon: None,
                    label: label.into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            usize::from(self.side == Color::Black),
            move |index, _, cx| {
                let _ = page.update(cx, |this, cx| {
                    let side = if index == 1 {
                        Color::Black
                    } else {
                        Color::White
                    };
                    if side != this.side {
                        this.set_side(side, cx);
                        if this.game.is_some() {
                            this.restart(cx);
                        }
                        cx.notify();
                    }
                });
            },
            false,
            palette,
        )
        .into_any_element()
    }

    fn render_home(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let empty = self.pgn.read(cx).value().trim().is_empty();
        let card = widgets::card(palette)
            .child(widgets::heading("Guess the move", palette))
            .child(
                v_flex()
                    .mt(px(12.))
                    .gap(px(14.))
                    .max_w(px(560.))
                    .child(run::field(
                        "Game",
                        Textarea::new(&self.pgn).aria_label("Game PGN"),
                        palette,
                    ))
                    .child(run::field(
                        "You guess",
                        self.side_pills("guess-side", cx, palette),
                        palette,
                    ))
                    .children(self.error.clone().map(|error| {
                        div()
                            .id("guess-error")
                            .test_support()
                            .aria_label(error.clone())
                            .relative()
                            .text_size(px(14.))
                            .text_color(palette.error)
                            .child(widgets::live_region("guess-error-spoken", error.clone()))
                            .child(error)
                    }))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new("guess-start")
                                    .primary()
                                    .label("Start")
                                    .disabled(empty)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.start_from_field(cx)),
                                    ),
                            )
                            .child(
                                Button::new("guess-file")
                                    .ghost()
                                    .icon(icon("folder-open"))
                                    .label("Open file…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_file(window, cx)
                                    })),
                            ),
                    ),
            );
        div()
            .id("guess-home")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .px(side_padding(window))
                    .pt(crate::play_layout::PAGE_TOP)
                    .pb(crate::play_layout::PAGE_BOTTOM)
                    .child(card),
            )
            .into_any_element()
    }

    /// The banner over the moves: the guess being judged, the last one's score, or the total.
    fn banner(&self) -> Option<widgets::StatusBanner> {
        if let Some(uci) = &self.pending {
            let game = self.game.as_ref()?;
            let mut history = game.moves[..self.ply].to_vec();
            history.push(uci.clone());
            let san = view::sans(&game.setup, &history).pop()?;
            return Some(
                widgets::StatusBanner::new(Tone::Plain, icon("target"), format!("{san}…").into())
                    .busy(true),
            );
        }
        if self.finished() {
            let judged: Vec<Judged> = self.guesses.iter().map(|guess| guess.judged).collect();
            let summary = guess::summarize(&judged);
            return Some(
                widgets::StatusBanner::new(
                    Tone::Win,
                    icon("trophy"),
                    summary.closeness.label().into(),
                )
                .detail(
                    format!(
                        "{} of {} ({}%) · {} of {} moves matched",
                        summary.points,
                        points_text(summary.most),
                        summary.percent,
                        summary.matched,
                        summary.guesses,
                    )
                    .into(),
                )
                .announce(),
            );
        }
        let last = self.guesses.last()?;
        let (tone, glyph, title, detail) = if last.judged.grade == Grade::Master {
            (
                Tone::Win,
                icon("check"),
                format!("{} ✓", last.played),
                format!(
                    "{} · {}",
                    grade_text(last.judged.grade),
                    points_text(last.judged.points)
                ),
            )
        } else {
            (
                if last.judged.points > 0 {
                    Tone::Plain
                } else {
                    Tone::Loss
                },
                icon(if last.judged.points > 0 { "info" } else { "x" }),
                format!("Played {}", last.played),
                format!(
                    "Your {}: {} · {}",
                    last.san,
                    if last.judged.loss.is_some() {
                        grade_text(last.judged.grade)
                    } else {
                        "Not the move played"
                    },
                    points_text(last.judged.points)
                ),
            )
        };
        Some(
            widgets::StatusBanner::new(tone, glyph, title.into())
                .detail(detail.into())
                .announce(),
        )
    }

    fn render_game(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let Some(game) = self.game.clone() else {
            return div().into_any_element();
        };
        let board_size = self.layout.board_size(window);
        let your_turn = self.your_turn();
        let finished = self.finished();
        let judged: Vec<Judged> = self.guesses.iter().map(|guess| guess.judged).collect();
        let summary = guess::summarize(&judged);
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(crate::board::cached(&self.board))
            .child(widgets::lone_entry(widgets::move_entry(
                &self.move_input,
                your_turn,
                palette,
            )));
        let page = cx.entity().downgrade();
        let shown = self.viewing.unwrap_or(self.ply);
        let moves = widgets::moves_list(
            "guess-moves",
            widgets::MoveSheet {
                sans: &game.sans[..self.ply.min(game.sans.len())],
                shown,
                first: view::first_move(&game.setup),
            },
            "No moves yet".into(),
            move |ply, _, cx| {
                let _ = page.update(cx, |this, cx| this.view(ply, cx));
            },
            crate::notation::figurine(cx),
            palette,
        );
        let score = format!(
            "{} · {} of {} matched",
            points_text(summary.points),
            summary.matched,
            summary.guesses
        );
        let panel = widgets::side_panel(palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                v_flex()
                    .child(widgets::heading(
                        format!("{} – {}", game.white, game.black),
                        palette,
                    ))
                    .child(
                        div()
                            .id("guess-score")
                            .test_support()
                            .aria_label(score.clone())
                            .child(widgets::hint(score, palette)),
                    ),
            )
            .child(run::field(
                "You guess",
                self.side_pills("guess-side", cx, palette),
                palette,
            ))
            .when(!self.engine_usable(cx), |panel| {
                panel.child(widgets::hint(
                    "Without the engine only the move played scores",
                    palette,
                ))
            })
            .children(self.banner())
            .child(div().flex_1().min_h_0().child(moves))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("guess-restart")
                            .outline()
                            .icon(icon("rotate-ccw"))
                            .label(if finished { "Again" } else { "Restart" })
                            .on_click(cx.listener(|this, _, _, cx| this.restart(cx))),
                    )
                    .child(
                        Button::new("guess-another")
                            .ghost()
                            .label("Another game")
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            );
        div()
            .size_full()
            .child(self.layout.render("guess", board_column, panel, window, cx))
            .into_any_element()
    }
}

impl Render for GuessMoves {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.game.is_some() {
            self.render_game(window, cx, &palette)
        } else {
            self.render_home(window, cx, &palette)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Game, eval_of};
    use kchess_domain::training::guess::Eval;
    use serde_json::json;
    use shakmaty::Color;

    #[test]
    fn a_game_is_its_mainline() {
        let game =
            Game::from_pgn("[White \"Carlsen\"]\n[Black \"?\"]\n\n1. e4 e5 (1... c5) 2. Nf3 Nc6 *")
                .unwrap();
        assert_eq!(game.sans, ["e4", "e5", "Nf3", "Nc6"]);
        assert_eq!((game.white.as_str(), game.black.as_str()), ("Carlsen", "?"));
        assert_eq!(game.turn_at(1), Color::Black);
    }

    #[test]
    fn only_standard_games_with_moves_are_guessed() {
        assert!(Game::from_pgn("[Variant \"Atomic\"]\n\n1. e4 *").is_err());
        assert!(Game::from_pgn("*").is_err());
    }

    #[test]
    fn lines_are_read_for_the_guessing_side() {
        let line = json!({ "cp": 31, "pv": ["e2e4"] });
        assert_eq!(eval_of(&line, Color::White), Some(Eval::Cp(31)));
        assert_eq!(eval_of(&line, Color::Black), Some(Eval::Cp(-31)));
        let mate = json!({ "mate": -2, "pv": ["e2e4"] });
        assert_eq!(eval_of(&mate, Color::Black), Some(Eval::Mate(2)));
    }
}
