//! Two people at one board, with optional clocks.

use serde::{Deserialize, Serialize};

use super::{Board, Clock, Clocking, Setup, Times};
use crate::js;
use crate::records::games::{GameEnd, opponent, pgn_result};

/// Both sides' clocks, as a session keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalClock {
    pub white: Clock,
    pub black: Clock,
}

impl LocalClock {
    pub fn side(&self, color: &str) -> Clock {
        if color == "white" {
            self.white
        } else {
            self.black
        }
    }
}

/// The session document and its pause flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub setup: Setup,
    pub moves: Vec<String>,
    pub clock: Option<LocalClock>,
    pub times: Option<Times>,
    pub result: Option<GameEnd>,
    pub paused: bool,
}

/// What the game keeps beside the state: when the turn began.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub turn_started: f64,
}

/// A saved session: every field may be missing when it is read back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub setup: Option<Setup>,
    pub moves: Option<Vec<String>>,
    pub clock: Option<LocalClock>,
    pub times: Option<Times>,
    pub result: Option<GameEnd>,
}

impl State {
    /// A fresh state from a saved session, its clocks paused.
    pub fn from_saved(saved: Saved) -> State {
        State {
            setup: saved.setup.unwrap_or_else(Setup::standard),
            moves: saved.moves.unwrap_or_default(),
            clock: saved.clock,
            times: saved.times,
            result: saved.result,
            paused: true,
        }
    }
}

/// The game as the archive stores it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Archived {
    pub source: &'static str,
    pub setup: Setup,
    pub moves: Vec<String>,
    pub white: &'static str,
    pub black: &'static str,
    pub result: &'static str,
    pub reason: String,
    pub time_control: String,
}

/// Two people at one board.
#[derive(Clone, Debug)]
pub struct LocalGame {
    state: State,
    runtime: Runtime,
    board: Board,
}

impl LocalGame {
    /// The game of a state and its runtime; an illegal setup is refused.
    pub fn new(state: State, runtime: Runtime) -> Result<LocalGame, String> {
        let board = Board::new(state.setup.clone(), state.moves.clone())?;
        Ok(LocalGame {
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

    fn set_board(&mut self, board: Board) {
        self.state.moves = board.moves().to_vec();
        self.board = board;
    }

    /// The decided result, else how the position ended; none while the game goes on.
    pub fn result(&self) -> Option<&GameEnd> {
        self.state.result.as_ref().or(self.board.end())
    }

    pub fn over(&self) -> bool {
        self.result().is_some()
    }

    /// A clock is set, the game goes on, and it has started.
    pub fn running(&self) -> bool {
        self.state.times.is_some()
            && !self.over()
            && !self.state.paused
            && !self.state.moves.is_empty()
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

    /// The side on the clock has run out of time at `at`: whether its flag fell now.
    pub fn expire(&mut self, at: f64) -> bool {
        let Some(turn) = self.clocking().out_of_time(at).map(str::to_string) else {
            return false;
        };
        if let Some(times) = self.state.times.as_mut() {
            times.set(&turn, 0.0);
        }
        self.state.result = Some(match self.board.timeout_winner(&turn) {
            Some(winner) => GameEnd {
                winner: Some(winner.to_string()),
                reason: "Time out".into(),
            },
            None => GameEnd {
                winner: None,
                reason: "Time out, but mate was impossible".into(),
            },
        });
        true
    }

    fn increment(&self, color: &str) -> f64 {
        self.state
            .clock
            .map_or(0.0, |clock| clock.side(color).increment)
    }

    /// Play a move, the clock read as `now` when it gets that far: its SAN, or none when it is
    /// not legal now, and whether a flag fell.
    pub fn play(&mut self, uci: &str, now: impl FnOnce() -> f64) -> (Option<String>, bool) {
        if self.over() {
            return (None, false);
        }
        let at = now();
        if self.expire(at) {
            return (None, true);
        }
        let Some((board, san)) = self.board.with_move(uci) else {
            return (None, false);
        };
        let mover = self.board.turn();
        let increment = if self.state.moves.is_empty() {
            0.0
        } else {
            self.increment(mover) * 1000.0
        };
        if let Some(times) = self.clocking().charged(mover, at, increment) {
            self.state.times = Some(times);
        }
        self.runtime.turn_started = at;
        self.state.paused = false;
        self.set_board(board);
        (Some(san), false)
    }

    /// A side's clock ran out: a time-out ends the game for good.
    fn timed_out(&self) -> bool {
        self.state
            .times
            .is_some_and(|times| times.white <= 0.0 || times.black <= 0.0)
    }

    /// Undo the last move. A time-out is final: undoing it left the flagged side on
    /// 0 ms, and it lost on time again as soon as it was back on move.
    pub fn takeback(&mut self, now: impl FnOnce() -> f64) {
        if self.state.moves.is_empty() || self.timed_out() {
            return;
        }
        let mut moves = self.state.moves.clone();
        moves.pop();
        if let Ok(board) = Board::new(self.state.setup.clone(), moves) {
            self.set_board(board);
        }
        self.state.result = None;
        self.runtime.turn_started = now();
    }

    /// A new game from `setup` with `clock`; an illegal setup or clock is refused.
    pub fn start(
        &mut self,
        setup: Setup,
        clock: Option<LocalClock>,
        now: impl FnOnce() -> f64,
    ) -> Result<(), String> {
        if let Some(clock) = clock {
            clock.white.checked()?;
            clock.black.checked()?;
        }
        let board = Board::new(setup.clone(), Vec::new())?;
        self.state.setup = setup;
        self.state.clock = clock;
        self.set_board(board);
        self.state.result = None;
        self.state.paused = true;
        self.state.times = clock.map(|clock| Times {
            white: clock.white.minutes * 60000.0,
            black: clock.black.minutes * 60000.0,
        });
        self.runtime.turn_started = now();
        Ok(())
    }

    pub fn resign(&mut self, color: &str) {
        if self.over() {
            return;
        }
        let side = if color == "white" { "White" } else { "Black" };
        self.state.result = Some(GameEnd {
            winner: Some(opponent(color).to_string()),
            reason: format!("{side} resigned"),
        });
    }

    pub fn agree_draw(&mut self) {
        if self.over() {
            return;
        }
        self.state.result = Some(GameEnd {
            winner: None,
            reason: "Draw agreed".into(),
        });
    }

    /// Pause or resume the clocks at `at`: whether a flag fell instead.
    pub fn toggle_pause(&mut self, at: f64) -> bool {
        if self.state.times.is_none() || self.over() {
            return false;
        }
        if self.expire(at) {
            return true;
        }
        if !self.state.paused {
            self.state.times = Some(self.clocking().times_at(at));
        }
        self.state.paused = !self.state.paused;
        self.runtime.turn_started = at;
        false
    }

    /// The session to save, with each side's remaining time at `at`.
    pub fn snapshot(&self, at: f64) -> Saved {
        Saved {
            setup: Some(self.state.setup.clone()),
            moves: Some(self.state.moves.clone()),
            clock: self.state.clock,
            times: self.state.times.map(|_| self.clocking().times_at(at)),
            result: self.state.result.clone(),
        }
    }

    /// The game as the archive stores it.
    pub fn archive_snapshot(&self) -> Archived {
        let result = self.result();
        let winner = result.and_then(|result| result.winner.as_deref());
        let time_control = match self.state.clock {
            Some(clock) => format!(
                "{}+{} / {}+{}",
                js::number_to_string(clock.white.minutes * 60.0),
                js::number_to_string(clock.white.increment),
                js::number_to_string(clock.black.minutes * 60.0),
                js::number_to_string(clock.black.increment)
            ),
            None => "-".to_string(),
        };
        Archived {
            source: "board",
            setup: self.state.setup.clone(),
            moves: self.state.moves.clone(),
            white: "White",
            black: "Black",
            result: pgn_result(result.is_some(), winner),
            reason: result.map_or_else(|| "In progress".to_string(), |r| r.reason.clone()),
            time_control,
        }
    }
}
