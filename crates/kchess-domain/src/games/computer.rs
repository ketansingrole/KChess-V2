//! A game against the engine: its clock, the engine's replies and the cancellation of work that
//! is no longer wanted.

use serde::{Deserialize, Serialize};

use super::{Board, Clock, Clocking, Host, Setup, Times};
use crate::js;
use crate::records::games::{opponent, pgn_result};
use crate::training::engine_levels;
use crate::variant::Variant;

/// The session document and the engine turn's bookkeeping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub moves: Vec<String>,
    pub ply: usize,
    pub level: String,
    pub color: String,
    pub resigned: bool,
    pub setup: Setup,
    pub clock: Option<Clock>,
    pub times: Option<Times>,
    pub flagged: Option<String>,
    pub thinking: bool,
    pub epoch: u64,
}

/// What the game keeps beside the state: when the turn began, whether the engine may play, and
/// whether the game was closed (a closed game ignores every input).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub turn_started: f64,
    pub permitted: bool,
    pub disposed: bool,
}

impl Runtime {
    /// The runtime of a new game: the clock and availability as the host reports them now.
    pub fn start(host: &mut dyn Host) -> Runtime {
        let turn_started = host.now();
        let permitted = host.allowed();
        Runtime {
            turn_started,
            permitted,
            disposed: false,
        }
    }
}

/// A saved session: every field may be missing when it is read back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub moves: Option<Vec<String>>,
    pub ply: Option<usize>,
    pub level: Option<String>,
    pub color: Option<String>,
    pub resigned: Option<bool>,
    pub setup: Option<Setup>,
    pub clock: Option<Clock>,
    pub times: Option<Times>,
    pub flagged: Option<String>,
}

impl State {
    /// A fresh state from a saved session.
    pub fn from_saved(saved: Saved) -> State {
        State {
            moves: saved.moves.unwrap_or_default(),
            ply: saved.ply.unwrap_or(0),
            level: saved.level.unwrap_or_else(|| "club".into()),
            color: saved.color.unwrap_or_else(|| "white".into()),
            resigned: saved.resigned.unwrap_or(false),
            setup: saved.setup.unwrap_or_else(Setup::standard),
            clock: saved.clock,
            times: saved.times,
            flagged: saved.flagged,
            thinking: false,
            epoch: 0,
        }
    }
}

/// What the game asks its caller to do, in order.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Effect {
    StopEngine,
    Flagged,
    Moved {
        san: String,
        computer: bool,
    },
    /// It is the engine's turn: start a search (`search_start`).
    ComputerTurn,
    /// The engine's search failed.
    Failed,
    /// A search cancelled meanwhile failed: expected, and silent.
    #[serde(rename = "debug")]
    Cancelled,
}

/// The options of an engine search.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SearchOptions {
    /// The start, when it is not the usual one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub chess960: bool,
    /// A variant not played by standard rules: Fairy-Stockfish plays it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movetime: Option<f64>,
}

/// An engine search to send; its reply must still match the epoch and move count.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchRequest {
    pub epoch: u64,
    pub count: usize,
    pub moves: Vec<String>,
    pub level: String,
    pub options: SearchOptions,
}

/// Whether the player won, lost or drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Win,
    Loss,
    Draw,
}

/// The result from the player's side.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: Kind,
    pub title: &'static str,
    pub detail: String,
}

/// The game as the archive stores it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Archived {
    pub source: &'static str,
    pub setup: Setup,
    pub moves: Vec<String>,
    pub white: String,
    pub black: String,
    pub result: &'static str,
    pub reason: String,
    pub time_control: String,
}

/// A game against the engine.
#[derive(Clone, Debug)]
pub struct ComputerGame {
    state: State,
    runtime: Runtime,
    board: Board,
}

impl ComputerGame {
    /// The game of a state and its runtime; an illegal setup is refused.
    pub fn new(state: State, runtime: Runtime) -> Result<ComputerGame, String> {
        let board = Board::new(state.setup.clone(), state.moves.clone())?;
        Ok(ComputerGame {
            state,
            runtime,
            board,
        })
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Show the position after `ply` moves (at most every move).
    pub fn show_ply(&mut self, ply: usize) {
        self.state.ply = ply.min(self.state.moves.len());
    }

    /// Play `color` from the next game on.
    pub fn set_color(&mut self, color: &str) {
        self.state.color = color.to_string();
    }

    /// Play at `level` from the next search on.
    pub fn set_level(&mut self, level: &str) {
        self.state.level = level.to_string();
    }

    /// Play on `clock` from the next game on.
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.state.clock = clock;
    }

    fn set_board(&mut self, board: Board) {
        self.state.moves = board.moves().to_vec();
        self.board = board;
    }

    /// Resigned, flagged, the position ended, or a draw reached.
    pub fn over(&self) -> bool {
        self.state.resigned
            || self.state.flagged.is_some()
            || self.board.info().end
            || self.board.draw().is_some()
    }

    /// A clock is set, both sides have moved and the game goes on.
    pub fn running(&self) -> bool {
        self.runtime.permitted
            && self.state.times.is_some()
            && !self.over()
            && self.state.moves.len() >= 2
    }

    fn clocking(&self) -> Clocking<'_> {
        Clocking {
            times: self.state.times,
            turn_started: self.runtime.turn_started,
            running: self.running(),
            turn: self.board.turn(),
        }
    }

    /// A side's clock in milliseconds at `at`.
    pub fn remaining(&self, color: &str, at: f64) -> f64 {
        self.clocking().remaining(color, at)
    }

    /// The repetition or fifty-move draw reached, if any.
    pub fn draw(&self) -> Option<&str> {
        self.board.draw()
    }

    /// The side that won, or none for a draw or a game going on.
    pub fn winner(&self) -> Option<String> {
        if !self.over() {
            return None;
        }
        if self.state.resigned {
            return Some(opponent(&self.state.color).to_string());
        }
        match &self.state.flagged {
            Some(flagged) => self.board.timeout_winner(flagged).map(str::to_string),
            None => self.board.outcome_winner().map(str::to_string),
        }
    }

    /// The result from the player's side, once the game is over.
    pub fn result(&self) -> Option<Verdict> {
        if !self.over() {
            return None;
        }
        if self.state.resigned {
            return Some(Verdict {
                kind: Kind::Loss,
                title: "Stockfish won",
                detail: "You resigned".into(),
            });
        }
        let winner = self.winner();
        let won = winner.as_deref() == Some(self.state.color.as_str());
        let detail = if self.state.flagged.is_some() {
            if winner.is_none() {
                "Time ran out, but mate was impossible"
            } else if won {
                "Stockfish ran out of time"
            } else {
                "You ran out of time"
            }
            .to_string()
        } else {
            self.board
                .end()
                .map_or("Game over", |end| end.reason.as_str())
                .to_string()
        };
        let (kind, title) = if winner.is_none() {
            (Kind::Draw, "Draw")
        } else if won {
            (Kind::Win, "You won")
        } else {
            (Kind::Loss, "Stockfish won")
        };
        Some(Verdict {
            kind,
            title,
            detail,
        })
    }

    /// Stop the engine and invalidate any reply it may still send.
    fn cancel(&mut self, fx: &mut Vec<Effect>) {
        self.state.epoch += 1;
        self.state.thinking = false;
        fx.push(Effect::StopEngine);
    }

    fn expire_into(&mut self, at: f64, fx: &mut Vec<Effect>) -> bool {
        let Some(turn) = self.clocking().out_of_time(at).map(str::to_string) else {
            return false;
        };
        if let Some(times) = self.state.times.as_mut() {
            times.set(&turn, 0.0);
        }
        self.state.flagged = Some(turn);
        self.cancel(fx);
        fx.push(Effect::Flagged);
        true
    }

    /// Flag the side on the clock when its time is up at `at`.
    pub fn expire(&mut self, at: f64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.expire_into(at, &mut fx);
        fx
    }

    /// Play a legal move, charge the clock and report it.
    fn commit(&mut self, uci: &str, at: f64, computer: bool, fx: &mut Vec<Effect>) -> bool {
        let Some((board, san)) = self.board.with_move(uci) else {
            return false;
        };
        let mover = self.board.turn();
        let increment = if self.state.moves.len() >= 2 {
            self.state.clock.map_or(0.0, |clock| clock.increment) * 1000.0
        } else {
            0.0
        };
        if let Some(times) = self.clocking().charged(mover, at, increment) {
            self.state.times = Some(times);
        }
        self.runtime.turn_started = at;
        self.set_board(board);
        self.state.ply = self.state.moves.len();
        fx.push(Effect::Moved { san, computer });
        true
    }

    /// The player's move, when it is theirs to make: whether it was played.
    pub fn play(&mut self, host: &mut dyn Host, uci: &str) -> (bool, Vec<Effect>) {
        let mut fx = Vec::new();
        if self.runtime.disposed || !host.allowed() || !host.ready() {
            return (false, fx);
        }
        if self.state.ply != self.state.moves.len()
            || self.over()
            || self.state.thinking
            || self.board.turn() != self.state.color.as_str()
        {
            return (false, fx);
        }
        let at = host.now();
        if self.expire_into(at, &mut fx) || !self.commit(uci, at, false, &mut fx) {
            return (false, fx);
        }
        fx.push(Effect::ComputerTurn);
        (true, fx)
    }

    /// The start of an engine turn, when one is due: the request to send.
    pub fn search_start(&mut self, host: &mut dyn Host) -> Result<Option<SearchRequest>, String> {
        if self.runtime.disposed || self.state.thinking {
            return Ok(None);
        }
        if self.board.turn() == self.state.color.as_str() || self.over() {
            return Ok(None);
        }
        if !host.ready() || !host.allowed() {
            return Ok(None);
        }
        self.state.thinking = true;
        let left = self.remaining(self.board.turn(), host.now());
        let movetime = match self.state.times {
            Some(_) => {
                let time = level_info(&self.state.level)?.time;
                let increment = self.state.clock.map_or(0.0, |clock| clock.increment);
                let budget = (left / 30.0 + increment * 800.0).floor();
                Some(50.0_f64.max(5000.0_f64.min(time).min(budget)))
            }
            None => None,
        };
        let setup = &self.state.setup;
        let options = SearchOptions {
            fen: (!setup.is_standard_start()).then(|| setup.fen.clone()),
            chess960: setup.variant == Variant::Chess960,
            variant: (!setup.variant.standard_rules()).then_some(setup.variant),
            movetime,
        };
        Ok(Some(SearchRequest {
            epoch: self.state.epoch,
            count: self.state.moves.len(),
            moves: self.state.moves.clone(),
            level: self.state.level.clone(),
            options,
        }))
    }

    /// The engine's reply to the search of `epoch` at `count` moves, if it is still wanted: the
    /// move is played (or the clock flags).
    pub fn search_reply(
        &mut self,
        host: &mut dyn Host,
        epoch: u64,
        count: usize,
        reply: &str,
    ) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed
            || epoch != self.state.epoch
            || count != self.state.moves.len()
            || self.over()
            || !host.allowed()
        {
            return fx;
        }
        let at = host.now();
        if !self.expire_into(at, &mut fx) {
            self.commit(reply, at, true, &mut fx);
        }
        fx
    }

    /// The search of `epoch` failed: reported unless it was cancelled meanwhile.
    pub fn search_failed(&self, epoch: u64) -> Effect {
        if epoch == self.state.epoch && !self.runtime.disposed {
            Effect::Failed
        } else {
            Effect::Cancelled
        }
    }

    /// The end of the search of `epoch`, however it finished.
    pub fn search_end(&mut self, epoch: u64) {
        if epoch == self.state.epoch {
            self.state.thinking = false;
        }
    }

    /// A new game from `setup` on `clock`; an illegal setup or clock is refused.
    pub fn start(
        &mut self,
        host: &mut dyn Host,
        setup: Setup,
        clock: Option<Clock>,
    ) -> Result<Vec<Effect>, String> {
        let clock = clock.map(Clock::checked).transpose()?;
        let mut fx = Vec::new();
        if self.runtime.disposed {
            return Ok(fx);
        }
        let board = Board::new(setup.clone(), Vec::new())?;
        self.cancel(&mut fx);
        self.state.setup = setup;
        self.state.clock = clock;
        self.set_board(board);
        self.state.ply = 0;
        self.state.resigned = false;
        self.state.flagged = None;
        self.state.times = clock.map(|clock| {
            let ms = clock.minutes * 60000.0;
            Times {
                white: ms,
                black: ms,
            }
        });
        self.runtime.turn_started = host.now();
        fx.push(Effect::ComputerTurn);
        Ok(fx)
    }

    /// Undo back to the player's last turn. A time-out is final: undoing it left the flagged
    /// side on 0 ms, to lose on time again on its next move.
    pub fn takeback(&mut self, host: &mut dyn Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed || self.state.moves.is_empty() || self.state.flagged.is_some() {
            return fx;
        }
        self.cancel(&mut fx);
        self.state.resigned = false;
        self.state.flagged = None;
        let mut kept = self.state.moves.len() - 1;
        while kept > 0 && self.board.turn_after(kept) != self.state.color.as_str() {
            kept -= 1;
        }
        let moves = self.state.moves[..kept].to_vec();
        if let Ok(board) = Board::new(self.state.setup.clone(), moves) {
            self.set_board(board);
        }
        self.state.ply = self.state.moves.len();
        self.runtime.turn_started = host.now();
        fx.push(Effect::ComputerTurn);
        fx
    }

    pub fn resign(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed || self.state.moves.is_empty() || self.over() {
            return fx;
        }
        self.cancel(&mut fx);
        self.state.resigned = true;
        self.state.ply = self.state.moves.len();
        fx
    }

    /// Assistance was allowed or stopped: freeze the clocks and cancel the engine when it stops.
    pub fn availability_changed(&mut self, host: &mut dyn Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        let permitted = host.allowed();
        let at = host.now();
        if self.runtime.permitted && !permitted {
            if self.state.times.is_some() {
                self.state.times = Some(self.clocking().times_at(at));
            }
            self.cancel(&mut fx);
        }
        if permitted != self.runtime.permitted {
            self.runtime.turn_started = at;
        }
        self.runtime.permitted = permitted;
        fx
    }

    /// The session to save, with each side's remaining time at `at`.
    pub fn snapshot(&self, at: f64) -> Saved {
        Saved {
            moves: Some(self.state.moves.clone()),
            ply: Some(self.state.ply),
            level: Some(self.state.level.clone()),
            color: Some(self.state.color.clone()),
            resigned: Some(self.state.resigned),
            setup: Some(self.state.setup.clone()),
            clock: self.state.clock,
            times: self.state.times.map(|_| self.clocking().times_at(at)),
            flagged: self.state.flagged.clone(),
        }
    }

    /// The game as the archive stores it.
    pub fn archive_snapshot(&self) -> Result<Archived, String> {
        let engine = format!("Stockfish ({})", level_info(&self.state.level)?.label);
        let (white, black) = if self.state.color == "white" {
            ("You".to_string(), engine)
        } else {
            (engine, "You".to_string())
        };
        let winner = self.winner();
        let reason = self
            .result()
            .map_or_else(|| "In progress".to_string(), |verdict| verdict.detail);
        let time_control = match self.state.clock {
            Some(clock) => format!(
                "{}+{}",
                js::number_to_string(clock.minutes * 60.0),
                js::number_to_string(clock.increment)
            ),
            None => "-".to_string(),
        };
        Ok(Archived {
            source: "computer",
            setup: self.state.setup.clone(),
            moves: self.state.moves.clone(),
            white,
            black,
            result: pgn_result(self.over(), winner.as_deref()),
            reason,
            time_control,
        })
    }
}

/// The ladder entry for an engine level; an error for an unknown one.
fn level_info(level: &str) -> Result<&'static engine_levels::EngineLevelInfo, String> {
    engine_levels::info(level)?.ok_or_else(|| format!("unknown engine level {level}"))
}
