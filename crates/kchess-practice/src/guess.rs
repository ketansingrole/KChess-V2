//! Guess the move: a game (a player's or a famous one found here, a master game from the explorer,
//! or a PGN) is played through and one side's moves are guessed. The local engine looks at each
//! position while you think, the move played included, so a guess scores against the move played
//! where it holds up and against the engine's best where it went wrong
//! (`kchess_domain::training::guess`); the other side's moves are played for you.

use kchess_contracts::calls;
use kchess_contracts::types::{AnalysisRequest, AnalysisUpdate, EngineLine};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_domain::training::guess::{self, Eval, Evals, Flaw, Grade, Judged};
use kchess_domain::variant::{Setup, Variant};
use serde_json::Value;
use shakmaty::{Color, Position as _};

use super::guess_find::{Found, GameFinder};
use super::run;
use crate::board::{Arrow, BoardEvent, ChessBoard};
use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::game::tree::TreeNode;
use crate::game::view;
use crate::live_game::LiveGameHandle;
use crate::position_engine::{EngineLook, PositionEngine};
use crate::theme::Palette;
use crate::voice::moves::{Access, Action, BoardBar, Turn, VoiceMoves, board_bar};
use crate::widgets::{self, Pill, Tone};
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, side_padding, tab_bar_height};
use kchess_ui::icon;

const SIDE: &str = "kchess:guess-side";
/// The other side's move follows a guess after this long.
const REPLY: Duration = Duration::from_millis(450);
/// An engine look at a position stops at this depth, or after this long of the engine's search.
const PROBE_DEPTH: f64 = 14.;
const PROBE_TIME: Duration = Duration::from_millis(2000);
/// The engine's best lines looked at for a guess before searching the guess itself.
const LINES: u32 = 3;

/// A game to guess through: its start, its moves and who played it.
#[derive(Clone)]
struct Game {
    setup: Setup,
    moves: Vec<String>,
    sans: Vec<String>,
    white: String,
    black: String,
    /// The year it was played, when its PGN says.
    year: Option<String>,
}

impl Game {
    /// The mainline of a standard game's PGN, or why it cannot be guessed.
    fn from_pgn(pgn: &str) -> Result<Game, &'static str> {
        if crate::game::pgn_variant(pgn) != Some(Variant::Standard) {
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
        let setup = Setup::standard_at(root.fen.clone());
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
        let year = header("Date")
            .get(..4)
            .filter(|year| year.chars().all(|digit| digit.is_ascii_digit()))
            .map(str::to_string);
        Ok(Game {
            setup,
            moves,
            sans,
            white: header("White"),
            black: header("Black"),
            year,
        })
    }

    fn turn_at(&self, ply: usize) -> Color {
        view::turn(view::position_at(&self.setup, &self.moves, ply).as_ref())
    }
}

/// A scored guess at a move of the game.
struct Guessed {
    /// The move of the game it was a guess at.
    ply: usize,
    san: String,
    played: String,
    /// The engine's best move there, as it was written.
    best: Option<String>,
    judged: Judged,
}

/// What the engine is looking at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    /// The position before the guess, at this ply: its best lines.
    Best(usize),
    /// The position after the move played at this ply, when its best lines leave it out.
    Played(usize),
    /// The position after the guess being judged.
    Guess,
}

/// A line's score from the guessing side's point of view; lines are from White's.
fn eval_of(line: &EngineLine, side: Color) -> Option<Eval> {
    let eval = if let Some(mate) = line.mate {
        Eval::Mate(mate as i32)
    } else {
        Eval::Cp(line.cp? as i32)
    };
    Some(eval.for_side(side == Color::White))
}

fn grade_text(grade: Grade) -> &'static str {
    match grade {
        Grade::Master | Grade::Played => "The move played",
        Grade::Better => "Better than the move played",
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
    best: Option<(usize, Vec<EngineLine>)>,
    /// The engine's evaluation after the move played at a ply (none when it found none).
    played_eval: Option<(usize, Option<Eval>)>,
    engine: PositionEngine,
    /// Whether the engine is installed; none until the core answers. A guess made before then
    /// waits for the answer: judged at once, it scored as if there were no engine (Windows CI,
    /// 2026-11-10: `d5`, not `d5?!`).
    engine_ready: Option<bool>,
    error: Option<String>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    move_box: crate::game::surface::MoveBox,
    /// A guess said aloud, beside the move box as on the game pages.
    voice: VoiceMoves,
    pgn: Entity<TextareaState>,
    finder: Entity<GameFinder>,
    look: EngineLook,
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
                &Setup::standard(),
                &[],
                0,
                Color::White,
                None,
            ))
        });
        let (move_box, typed) = crate::game::surface::MoveBox::new(Self::typed_move, window, cx);
        let pgn = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(6)
                .placeholder("1. e4 e5 2. Nf3 Nc6 …")
        });
        let finder = cx.new(|cx| GameFinder::new(window, cx));
        let events = cx.global::<AppCore>().events.clone();
        let subscriptions = vec![
            cx.subscribe(&finder, |this, _, found: &Found, cx| match found {
                Found::Game { pgn, side } => {
                    this.load(pgn, Some(*side), cx);
                }
            }),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.guess(uci.clone(), cx);
                }
            }),
            typed,
            cx.subscribe(
                &crate::voice::handle(cx),
                |this, _, event: &crate::voice::VoiceEvent, cx| this.voice_heard(event, cx),
            ),
            cx.observe(&crate::voice::handle(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
                if let ClientEvent::Event(CoreEvent::Analysis(update)) = event {
                    this.engine_update(update, cx);
                }
            }),
        ];
        let side = if crate::prefs::get::<String>(cx, SIDE, "white".into()) == "black" {
            Color::Black
        } else {
            Color::White
        };
        let status = cx
            .global::<AppCore>()
            .request(calls::EngineStatus { variant: None });
        cx.spawn(async move |this, cx| {
            let ready = matches!(status.await, Ok(status) if status.ready);
            let _ = this.update(cx, |this, cx| this.engine_known(ready, cx));
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
            played_eval: None,
            engine: PositionEngine::default(),
            engine_ready: None,
            error: None,
            board,
            layout: PlayLayout::under(tab_bar_height::<crate::Tab>),
            move_box,
            voice: VoiceMoves::new(crate::voice::VoiceSource::Guess, cx),
            pgn,
            finder,
            look: EngineLook::default(),
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
        self.best = None;
        self.played_eval = None;
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
        self.engine_ready == Some(true)
            && cx
                .global::<LiveGameHandle>()
                .0
                .read(cx)
                .assistance_allowed()
    }

    /// Whether the core has said if the engine is installed; `forget` makes it unsaid again.
    #[cfg(any(test, feature = "test-support"))]
    pub fn engine_status_known(&mut self, forget: bool) -> bool {
        if forget {
            self.engine_ready = None;
        }
        self.engine_ready.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn answer_engine_status(&mut self, ready: bool, cx: &mut Context<Self>) {
        self.engine_known(ready, cx);
    }

    /// The core has said whether the engine is installed: look at the position to guess, or
    /// judge the guess that waited for this.
    fn engine_known(&mut self, ready: bool, cx: &mut Context<Self>) {
        self.engine_ready = Some(ready);
        if let Some(uci) = self.pending.clone() {
            if self.engine_usable(cx) {
                self.judge_pending(cx);
            } else {
                self.judged(&uci, Evals::default(), cx);
            }
        } else if self.your_turn() && self.probe.is_none() && self.engine_usable(cx) {
            self.start_probe(Probe::Best(self.ply), None, cx);
        }
        self.redraw(cx);
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
        let position = view::position_at(&game.setup, &game.moves, self.ply);
        if let Some(uci) = self.move_box.take(position.as_ref(), window, cx) {
            self.guess(uci, cx);
        }
    }

    /// The position to guess from, as voice reads a move in it.
    fn setup_now(&self) -> Setup {
        self.game
            .as_ref()
            .and_then(|game| view::position_at(&game.setup, &game.moves, self.ply))
            .map(|position| Setup {
                variant: Variant::Standard,
                fen: kchess_domain::rules::make_fen(&position),
            })
            .unwrap_or_else(|| Setup::standard_at(""))
    }

    /// A spoken move is a guess at this position's move, taken while it is yours to guess.
    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let (context, line) = match &self.game {
            Some(game) => (
                format!(
                    "{}:{}:{}:{}",
                    game.white,
                    game.black,
                    self.ply,
                    self.pending.is_some()
                ),
                Some(game.sans[..self.ply.min(game.sans.len())].to_vec()),
            ),
            None => (String::new(), None),
        };
        let listening = self.your_turn();
        self.voice.sync(
            &context,
            line,
            kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Game).to_vec(),
            listening,
            cx,
        );
    }

    fn voice_heard(&mut self, event: &crate::voice::VoiceEvent, cx: &mut Context<Self>) {
        let setup = self.setup_now();
        let taking = self.your_turn();
        let Some(heard) = self.voice.heard(event, &setup.fen, taking, cx) else {
            return;
        };
        let turn = Turn {
            taking,
            idle: Some(if self.finished() {
                "The game is over"
            } else {
                "Wait for your move to guess"
            }),
            immediate: false,
        };
        if let Action::Play(uci) = self.voice.hear(&heard, &setup, turn, |_| None, cx) {
            self.guess(uci, cx);
        }
        cx.notify();
    }

    /// `text` said clearly, with voice on: as the recognizer's phrase reaches the page.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hear_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        let heard = self.voice.control.hear_clearly(text);
        self.voice_heard(&heard, cx);
    }

    /// `uci` from the position at `ply` as `position::canonical_uci` writes it, so guesses, the
    /// moves played and the engine's lines compare (`guess::same_move`).
    fn canonical(&self, uci: &str) -> String {
        self.game
            .as_ref()
            .and_then(|game| view::position_at(&game.setup, &game.moves, self.ply))
            .and_then(|position| kchess_domain::position::canonical_uci(&position, uci))
            .unwrap_or_else(|| uci.to_string())
    }

    /// A guess at the move at `ply`: the move played scores at once; another waits for the engine.
    fn guess(&mut self, uci: String, cx: &mut Context<Self>) {
        let uci = self.canonical(&uci);
        if !self.your_turn() {
            return self.redraw(cx);
        }
        if self.engine_ready.is_none() {
            // Judged once the core says whether the engine is there (`engine_known`).
            self.pending = Some(uci);
            return self.redraw(cx);
        }
        if !self.engine_usable(cx) {
            self.stop_engine(cx);
            return self.judged(&uci, Evals::default(), cx);
        }
        self.pending = Some(uci);
        self.judge_pending(cx);
        self.redraw(cx);
    }

    /// The evaluation, for the guessing side, of a position the game ends in, without a search.
    fn ended(game: &Game, history: &[String]) -> Option<Eval> {
        let after = view::position_at(&game.setup, history, history.len())?;
        if after.is_checkmate() {
            Some(Eval::Mate(1))
        } else if after.is_stalemate() || after.is_insufficient_material() {
            Some(Eval::Cp(0))
        } else {
            None
        }
    }

    /// The evaluation, for the guessing side, of `uci` at the current ply from the best lines,
    /// or of the position it ends the game in.
    fn known_eval(&self, uci: &str) -> Option<Eval> {
        let game = self.game.as_ref()?;
        let listed = self.best.as_ref().and_then(|(ply, lines)| {
            (*ply == self.ply).then(|| {
                lines.iter().find(|line| {
                    line.pv
                        .first()
                        .is_some_and(|first| guess::same_move(&self.canonical(first), uci))
                })
            })?
        });
        if let Some(line) = listed {
            return eval_of(line, self.side);
        }
        let mut history = game.moves[..self.ply].to_vec();
        history.push(uci.to_string());
        Self::ended(game, &history)
    }

    /// The best lines are in: the move played is judged from them, or searched while you think.
    fn look_at_played(&mut self, ply: usize, cx: &mut Context<Self>) {
        let Some(played) = self
            .game
            .as_ref()
            .and_then(|game| game.moves.get(ply))
            .cloned()
        else {
            return;
        };
        if let Some(eval) = self.known_eval(&played) {
            self.played_eval = Some((ply, Some(eval)));
            return;
        }
        let Some(game) = &self.game else {
            return;
        };
        let mut history = game.moves[..ply].to_vec();
        history.push(played);
        self.start_probe(Probe::Played(ply), Some(history), cx);
    }

    /// Score the guess waiting once the engine has looked at the position and the move played,
    /// searching the position after the guess when neither tells how good it is.
    fn judge_pending(&mut self, cx: &mut Context<Self>) {
        let (Some(uci), Some(game)) = (self.pending.clone(), self.game.clone()) else {
            return;
        };
        let have_best = self.best.as_ref().is_some_and(|(ply, _)| *ply == self.ply);
        let played_eval = self
            .played_eval
            .filter(|(ply, _)| *ply == self.ply)
            .map(|(_, eval)| eval);
        if !have_best || played_eval.is_none() {
            // Still looking: the probe under way picks the guess up when it is done.
            if self.probe.is_none() {
                if have_best {
                    self.look_at_played(self.ply, cx);
                    if self.probe.is_none() {
                        return self.judge_pending(cx);
                    }
                } else {
                    self.start_probe(Probe::Best(self.ply), None, cx);
                }
            }
            return;
        }
        let best = self
            .best
            .as_ref()
            .and_then(|(_, lines)| lines.first())
            .and_then(|line| eval_of(line, self.side));
        let played = played_eval.flatten();
        let Some(best) = best else {
            return self.judged(&uci, Evals::default(), cx);
        };
        let evals = Evals {
            best: Some(best),
            guessed: None,
            played,
        };
        if guess::same_move(&uci, &self.canonical(&game.moves[self.ply])) {
            return self.judged(
                &uci,
                Evals {
                    guessed: played,
                    ..evals
                },
                cx,
            );
        }
        if let Some(guessed) = self.known_eval(&uci) {
            return self.judged(
                &uci,
                Evals {
                    guessed: Some(guessed),
                    ..evals
                },
                cx,
            );
        }
        let mut history = game.moves[..self.ply].to_vec();
        history.push(uci);
        self.start_probe(Probe::Guess, Some(history), cx);
    }

    /// Score a guess, show the move played, and go on.
    fn judged(&mut self, uci: &str, evals: Evals, cx: &mut Context<Self>) {
        let Some(game) = &self.game else {
            return;
        };
        let Some(played) = game
            .moves
            .get(self.ply)
            .map(|played| self.canonical(played))
        else {
            return;
        };
        let mut history = game.moves[..self.ply].to_vec();
        history.push(uci.to_string());
        let san = view::sans(&game.setup, &history)
            .pop()
            .unwrap_or_else(|| uci.to_string());
        let played_san = game.sans.get(self.ply).cloned().unwrap_or_default();
        let judged = guess::judge(uci, &played, evals);
        let best = self
            .best
            .as_ref()
            .and_then(|(_, lines)| lines.first())
            .and_then(|line| line.pv.first())
            .and_then(|best| {
                let mut line = game.moves[..self.ply].to_vec();
                line.push(best.clone());
                // Named only when it is a legal move there.
                let mut sans = view::sans(&game.setup, &line);
                (sans.len() == line.len()).then(|| sans.pop()).flatten()
            });
        crate::sound::move_sound(&played_san, cx);
        self.guesses.push(Guessed {
            ply: self.ply,
            san,
            played: played_san,
            best,
            judged,
        });
        self.pending = None;
        self.best = None;
        self.played_eval = None;
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
        let request = AnalysisRequest {
            fen: kchess_domain::rules::make_fen(&position),
            lines: if matches!(probe, Probe::Best(_)) {
                LINES
            } else {
                1
            },
            infinite: Some(true),
            root_fen: Some(game.setup.fen.clone()),
            moves: Some(history),
            client_id: None,
            variant: None,
        };
        self.probe = Some(probe);
        self.engine.search(Some(request), engine_of, cx);
        // A look ends after a while even if the engine never reaches its depth.
        self.look.start(PROBE_TIME, Self::probe_done, cx);
    }

    fn engine_update(&mut self, update: &AnalysisUpdate, cx: &mut Context<Self>) {
        if self.probe.is_none() || !self.engine.receive(update) {
            return;
        }
        self.look.reported(Self::probe_done, cx);
        let reached = update.depth >= PROBE_DEPTH || update.done;
        if reached || self.engine.error.is_some() {
            self.probe_done(cx);
        }
    }

    /// The engine has looked long enough: keep what it found and stop it.
    fn probe_done(&mut self, cx: &mut Context<Self>) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        let lines = self.engine.lines().to_vec();
        self.stop_engine(cx);
        match probe {
            Probe::Best(ply) => {
                self.best = Some((ply, lines));
                if ply == self.ply {
                    // The move played is looked at while you think, then a guess waiting judged.
                    self.look_at_played(ply, cx);
                    if self.probe.is_none() && self.pending.is_some() {
                        self.judge_pending(cx);
                    }
                }
            }
            Probe::Played(ply) => {
                let eval = lines.first().and_then(|line| eval_of(line, self.side));
                self.played_eval = Some((ply, eval));
                if ply == self.ply && self.pending.is_some() {
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
                let played = self.played_eval.and_then(|(_, eval)| eval);
                let guessed = lines.first().and_then(|line| eval_of(line, self.side));
                self.judged(
                    &uci,
                    Evals {
                        best,
                        guessed,
                        played,
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn stop_engine(&mut self, cx: &mut Context<Self>) {
        self.probe = None;
        self.look.clear();
        self.engine.search(None, engine_of, cx);
    }

    /// Look at a finished game's move.
    fn view(&mut self, ply: usize, cx: &mut Context<Self>) {
        if self.finished() {
            self.viewing = Some(ply);
            self.redraw(cx);
        }
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let Some(game) = &self.game else {
            return cx.notify();
        };
        let ply = self.viewing.unwrap_or(self.ply);
        let movable = self.your_turn().then_some(self.side);
        let mut state = view::board_state(&game.setup, &game.moves, ply, self.side, movable);
        // The guess being judged, over the position it was made in: the player's own move, just
        // made, so it is drawn and not said as a hint.
        if let Some(uci) = &self.pending
            && let Some((from, to)) = crate::board::last_squares(uci)
        {
            state.arrows = vec![Arrow::brush("blue", from, to)];
        }
        state.blindfold = crate::settings::frontend(cx).blindfold;
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
        kchess_board::export::choose_game_file(window, cx, |this, text, window, cx| match text {
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
            .child(widgets::heading("Your own game", palette))
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
                    .pt(kchess_nav::play_layout::PAGE_TOP)
                    .pb(kchess_nav::play_layout::PAGE_BOTTOM)
                    .gap(px(16.))
                    .child(self.finder.clone())
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
                        "{} of {} ({}%) · {} of {} moves matched{}",
                        summary.points,
                        points_text(summary.most),
                        summary.percent,
                        summary.matched,
                        summary.guesses,
                        if summary.flawed > 0 {
                            format!(
                                " · better than the game {} of {} times it went wrong",
                                summary.better, summary.flawed
                            )
                        } else {
                            String::new()
                        },
                    )
                    .into(),
                )
                .announce(),
            );
        }
        let last = self.guesses.last()?;
        let judged = last.judged;
        let points = points_text(judged.points);
        // What the engine makes of the move played, when it went wrong: told after the guess.
        let verdict = judged.flaw.filter(|flaw| flaw.went_wrong()).map(|flaw| {
            format!(
                "{} by today's engines{}",
                flaw.label(),
                last.best
                    .as_ref()
                    .map(|best| format!(", {best} was best"))
                    .unwrap_or_default()
            )
        });
        let played = format!(
            "{}{}",
            last.played,
            judged.flaw.map(Flaw::glyph).unwrap_or_default()
        );
        let (tone, glyph, title, detail) = match judged.grade {
            Grade::Master => (
                Tone::Win,
                icon("check"),
                format!("{played} ✓"),
                format!("{} · {points}", grade_text(judged.grade)),
            ),
            Grade::Played => (
                Tone::Plain,
                icon("info"),
                format!("{played} ✓"),
                format!("{} · {points}", verdict.unwrap_or_default()),
            ),
            Grade::Better => (
                Tone::Win,
                icon("check"),
                format!("Played {played}"),
                format!(
                    "Your {} is better: {} · {points}",
                    last.san,
                    verdict.unwrap_or_default()
                ),
            ),
            grade => (
                if judged.points > 0 {
                    Tone::Plain
                } else {
                    Tone::Loss
                },
                icon(if judged.points > 0 { "info" } else { "x" }),
                format!("Played {played}"),
                format!(
                    "{}Your {}: {} · {points}",
                    verdict
                        .map(|verdict| format!("{verdict}. "))
                        .unwrap_or_default(),
                    last.san,
                    if judged.loss.is_some() {
                        grade_text(grade)
                    } else {
                        "Not the move played"
                    },
                ),
            ),
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
        // The column holds the board and the move box's row.
        let (board_size, panel_height) = self
            .layout
            .board_and_panel(widgets::STACK_GAP + widgets::PLAYER_LINE, window);
        let your_turn = self.your_turn();
        let finished = self.finished();
        let judged: Vec<Judged> = self.guesses.iter().map(|guess| guess.judged).collect();
        let summary = guess::summarize(&judged);
        let entry = widgets::move_entry(&self.move_box.input, your_turn, palette);
        let (voice, voice_line) = board_bar(
            &self.voice,
            cx.entity().downgrade(),
            Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.guess(uci.to_string(), cx),
            },
            BoardBar {
                id: "guess-voice",
                hint: "Say “E four” or “Knight F three”",
                active: your_turn,
                listening: your_turn,
                grammar: kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Game)
                    .to_vec(),
            },
            cx,
        );
        let board_column = v_flex()
            .id("guess-column")
            .test_support()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(crate::board::cached(&self.board))
            .child(widgets::input_row(entry, voice))
            .children(voice_line)
            .children(self.move_box.message(palette));
        let page = cx.entity().downgrade();
        let shown = self.viewing.unwrap_or(self.ply);
        // The moves so far, a guessed one marked as the engine found it.
        let mut sans = game.sans[..self.ply.min(game.sans.len())].to_vec();
        for guessed in &self.guesses {
            if let (Some(san), Some(flaw)) = (sans.get_mut(guessed.ply), guessed.judged.flaw) {
                san.push_str(flaw.glyph());
            }
        }
        let moves = widgets::moves_list(
            "guess-moves",
            widgets::MoveSheet {
                sans: &sans,
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
            .id("guess-panel")
            .test_support()
            .h(panel_height)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                v_flex()
                    .child(widgets::heading(
                        match &game.year {
                            Some(year) => format!("{} – {} · {year}", game.white, game.black),
                            None => format!("{} – {}", game.white, game.black),
                        },
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
            .when(
                self.engine_ready.is_some() && !self.engine_usable(cx),
                |panel| {
                    panel.child(widgets::hint(
                        "Without the engine only the move played scores",
                        palette,
                    ))
                },
            )
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

/// A PGN's moves and start as guessing would play them through, or why it cannot be guessed;
/// Visualise reads the famous games with it too.
pub fn loaded(pgn: &str) -> Result<(Vec<String>, Setup), &'static str> {
    Game::from_pgn(pgn).map(|game| (game.moves, game.setup))
}

#[cfg(test)]
mod tests {
    use super::{EngineLine, Game, eval_of};

    /// A game file chosen in the dialog is read into the PGN field.
    #[gpui_kit::test]
    fn a_game_file_chosen_fills_the_field(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("game.pgn");
        std::fs::write(&path, "1. e4 e5 *").expect("a game file");
        let mut app = kchess_board::testing::Started::new(cx);
        let guess = app.with_window(|window, cx| {
            let guess = cx.new(|cx| super::GuessMoves::new(window, cx));
            guess.update(cx, |guess, cx| guess.choose_file(window, cx));
            guess
        });
        assert!(app.cx().did_prompt_for_paths());
        app.cx()
            .simulate_path_prompt_response(|_| Some(vec![path.clone()]));
        app.cx().run_until_parked();
        let field = app.with_window(|_, cx| guess.read(cx).pgn.read(cx).value().to_string());
        assert_eq!(field, "1. e4 e5 *");
    }
    use kchess_domain::training::guess::Eval;
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
        let line = |cp, mate| EngineLine {
            rank: 1,
            depth: 10.,
            cp,
            mate,
            pv: vec!["e2e4".into()],
        };
        let ahead = line(Some(31.), None);
        assert_eq!(eval_of(&ahead, Color::White), Some(Eval::Cp(31)));
        assert_eq!(eval_of(&ahead, Color::Black), Some(Eval::Cp(-31)));
        let mate = line(None, Some(-2.));
        assert_eq!(eval_of(&mate, Color::Black), Some(Eval::Mate(2)));
    }

    /// The engine judges guesses only once it is installed and while no live Lichess game holds
    /// engine help back.
    #[gpui_kit::test]
    fn the_engine_judges_only_when_installed_and_allowed(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = kchess_board::testing::Started::new(cx);
        app.wait_for_engine_help();
        let page = app.with_window(|window, cx| cx.new(|cx| super::GuessMoves::new(window, cx)));
        let usable =
            |app: &mut kchess_board::testing::Started, ready: Option<bool>, allowed: bool| {
                app.with_window(|_, cx| {
                    kchess_board::live_game::handle(cx)
                        .update(cx, |live, cx| live.set(allowed, false, cx));
                    page.update(cx, |page, cx| {
                        page.engine_ready = ready;
                        page.engine_usable(cx)
                    })
                })
            };
        assert!(usable(&mut app, Some(true), true));
        assert!(!usable(&mut app, Some(true), false), "a live game");
        assert!(!usable(&mut app, Some(false), true), "not installed");
        assert!(!usable(&mut app, None, true), "not known yet");
    }
}
