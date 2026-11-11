//! Two people at one board, with optional clocks.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::Color;

use super::{Board, Clock, Clocking, Outcome, Setup, Times};
use crate::js;
use crate::records::games::GameEnd;

/// The session a board game is saved as, and the Game history kinds of the board game and of
/// the chess clock.
pub const SESSION: &str = "local";
pub const HISTORY: &str = "board";
pub const CLOCK_HISTORY: &str = "clock";
/// Its PGN's `Event`.
pub const EVENT: &str = "Over the board (KChess)";

/// Both sides' clocks, as a session keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalClock {
    pub white: Clock,
    pub black: Clock,
}

impl LocalClock {
    pub fn side(&self, color: Color) -> Clock {
        match color {
            Color::White => self.white,
            Color::Black => self.black,
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

    /// A game from a saved session, or a new one; a session on an illegal setup is refused.
    pub fn resume(saved: Option<&Value>, now: f64) -> Result<LocalGame, String> {
        let saved: Saved = match saved {
            Some(saved) => {
                Saved::deserialize(saved).map_err(|error| format!("the saved game: {error}"))?
            }
            None => Saved::default(),
        };
        LocalGame::new(State::from_saved(saved), Runtime { turn_started: now })
    }

    /// The game's start.
    pub fn setup(&self) -> &Setup {
        &self.state.setup
    }

    /// The game's moves, in UCI.
    pub fn moves(&self) -> &[String] {
        &self.state.moves
    }

    /// The clocks chosen, none when untimed.
    pub fn clock(&self) -> Option<LocalClock> {
        self.state.clock
    }

    /// The clocks run in this game.
    pub fn timed(&self) -> bool {
        self.state.times.is_some()
    }

    /// The clocks are paused.
    pub fn paused(&self) -> bool {
        self.state.paused
    }

    /// The result once the game has ended, from the winner it names.
    pub fn outcome(&self) -> Option<Outcome> {
        let result = self.result()?;
        Some(result.winner.map_or(Outcome::Draw, Outcome::Wins))
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

    fn clocking(&self) -> Clocking {
        Clocking {
            times: self.state.times,
            turn_started: self.runtime.turn_started,
            running: self.running(),
            turn: self.board.turn(),
        }
    }

    /// A side's clock in milliseconds at `at`.
    pub fn remaining(&self, color: Color, at: f64) -> f64 {
        self.clocking().remaining(color, at)
    }

    /// The side on the clock has run out of time at `at`: whether its flag fell now.
    pub fn expire(&mut self, at: f64) -> bool {
        let Some(turn) = self.clocking().out_of_time(at) else {
            return false;
        };
        if let Some(times) = self.state.times.as_mut() {
            times.set(turn, 0.0);
        }
        self.state.result = Some(match self.board.timeout_winner(turn) {
            Some(winner) => GameEnd {
                winner: Some(winner),
                reason: "Time out".into(),
            },
            None => GameEnd {
                winner: None,
                reason: "Time out, but mate was impossible".into(),
            },
        });
        true
    }

    fn increment(&self, color: Color) -> f64 {
        self.state
            .clock
            .map_or(0.0, |clock| clock.side(color).increment)
    }

    /// Play a move at `at`: its SAN, or none when it is not legal now, and whether a flag fell.
    pub fn play(&mut self, uci: &str, at: f64) -> (Option<String>, bool) {
        if self.over() {
            return (None, false);
        }
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
    pub fn takeback(&mut self, now: f64) {
        if self.state.moves.is_empty() || self.timed_out() {
            return;
        }
        let mut moves = self.state.moves.clone();
        moves.pop();
        if let Ok(board) = Board::new(self.state.setup.clone(), moves) {
            self.set_board(board);
        }
        self.state.result = None;
        self.runtime.turn_started = now;
    }

    /// A new game from `setup` with `clock`; an illegal setup or clock is refused.
    pub fn start(
        &mut self,
        setup: Setup,
        clock: Option<LocalClock>,
        now: f64,
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
        self.runtime.turn_started = now;
        Ok(())
    }

    pub fn resign(&mut self, color: Color) {
        if self.over() {
            return;
        }
        let side = match color {
            Color::White => "White",
            Color::Black => "Black",
        };
        self.state.result = Some(GameEnd {
            winner: Some(!color),
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
            result: Outcome::tag(self.outcome()),
            reason: result.map_or_else(|| "In progress".to_string(), |r| r.reason.clone()),
            time_control,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clock(minutes: f64, increment: f64) -> Clock {
        Clock { minutes, increment }
    }

    fn one_minute() -> Option<LocalClock> {
        Some(LocalClock {
            white: clock(1., 0.),
            black: clock(1., 0.),
        })
    }

    /// Taking back after a flag fell left that side on 0 ms: it lost on time again as soon as it
    /// was back on move. As on Lichess, a time-out is final.
    #[test]
    fn a_time_out_is_not_taken_back() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        game.play("e2e4", 1.);
        game.play("e7e5", 2.);
        assert!(game.expire(200_000.));
        game.takeback(200_001.);
        assert!(game.over());
        assert_eq!(game.moves(), ["e2e4", "e7e5"]);
    }

    #[test]
    fn both_sides_move_and_a_resignation_ends_the_game() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        assert_eq!(game.play("e2e4", 1.).0.as_deref(), Some("e4"));
        assert_eq!(game.play("e7e5", 2.).0.as_deref(), Some("e5"));
        assert_eq!(game.play("e2e4", 3.).0, None);
        game.resign(Color::White);
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner, Some(Color::Black));
        assert_eq!(Outcome::tag(game.outcome()), "0-1");
        assert_eq!(Outcome::tag(game.outcome()), game.archive_snapshot().result);
    }

    /// A flag falls only when the time is up, and the snapshots describe the finished game.
    #[test]
    fn a_flag_falls_when_the_time_is_up() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        assert!(!game.over());
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        let (_, flagged) = game.play("e2e4", 1_000.);
        assert!(!flagged);
        assert!(!game.expire(30_000.));
        assert!(game.expire(100_000.));
        assert!(game.over());
        assert_eq!(json!(game.snapshot(100_000.))["moves"], json!(["e2e4"]));
        let archived = json!(game.archive_snapshot());
        assert_eq!(archived["source"], "board");
        assert_eq!(archived["result"], "1-0");
    }

    #[test]
    fn a_clock_runs_from_the_first_move_and_pauses() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        assert_eq!(game.remaining(Color::White, 10_000.), 60_000.);
        game.play("e2e4", 1_000.);
        assert!(game.remaining(Color::Black, 11_000.) < 60_000.);
        game.toggle_pause(11_000.);
        let paused = game.remaining(Color::Black, 11_000.);
        assert_eq!(game.remaining(Color::Black, 40_000.), paused);
    }

    #[test]
    fn a_flag_falls_once_the_side_to_move_runs_out() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        game.play("e2e4", 1_000.);
        assert!(!game.expire(30_000.));
        assert!(!game.over());
        assert!(game.expire(62_000.));
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner, Some(Color::White));
    }

    #[test]
    fn an_agreed_draw_ends_the_game_without_a_winner() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.agree_draw();
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner, None);
    }

    #[test]
    fn a_takeback_undoes_the_last_move() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.takeback(2.);
        // White is to move again, so e4 is legal once more and e5 is not.
        assert_eq!(game.play("e7e5", 3.).0, None);
        assert_eq!(game.play("e2e4", 4.).0.as_deref(), Some("e4"));
    }

    #[test]
    fn checkmate_ends_the_game() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        for (ply, uci) in ["f2f3", "e7e5", "g2g4", "d8h4"].iter().enumerate() {
            assert!(!game.over());
            game.play(uci, ply as f64);
        }
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner, Some(Color::Black));
    }

    /// A board game as the library stores it today reads back exactly, its winner named as
    /// before.
    #[test]
    fn a_stored_session_reads_and_writes_as_before() {
        let stored = json!({
            "setup": { "variant": "standard", "fen": crate::rules::INITIAL_FEN },
            "moves": ["f2f3", "e7e5", "g2g4", "d8h4"],
            "clock": null,
            "times": null,
            "result": { "winner": "black", "reason": "Checkmate" },
        });
        let game = LocalGame::resume(Some(&stored), 0.).unwrap();
        assert_eq!(game.outcome(), Some(Outcome::Wins(Color::Black)));
        assert_eq!(json!(game.snapshot(0.)), stored);
        let drawn = json!({ "result": { "reason": "Draw agreed" } });
        let game = LocalGame::resume(Some(&drawn), 0.).unwrap();
        assert_eq!(
            json!(game.snapshot(0.))["result"],
            json!({ "reason": "Draw agreed" })
        );
    }

    #[test]
    fn a_saved_game_resumes_where_it_was_left() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.play("e7e5", 2.);
        let saved = json!(game.snapshot(3.));
        let mut resumed = LocalGame::resume(Some(&saved), 3.).unwrap();
        assert_eq!(resumed.play("e2e4", 4.).0, None);
        assert_eq!(resumed.play("g1f3", 5.).0.as_deref(), Some("Nf3"));
    }

    /// `1e306` minutes made both times infinite, stored as `null`: every later move failed to
    /// read the state, and the game could only be reset. Negative minutes flagged at once.
    #[test]
    fn a_clock_no_game_can_have_is_refused_at_the_start() {
        let mut game = LocalGame::resume(None, 0.).unwrap();
        for (minutes, increment) in [(1e306, 0.), (-5., 0.), (5., -1.), (181., 0.), (5., 1e9)] {
            let clock = LocalClock {
                white: clock(minutes, increment),
                black: clock(5., 0.),
            };
            assert!(
                game.start(Setup::standard(), Some(clock), 0.).is_err(),
                "{minutes} {increment}"
            );
            assert_eq!(game.play("e2e4", 1.).0.as_deref(), Some("e4"));
            game.takeback(2.);
        }
        let clock = LocalClock {
            white: clock(180., 180.),
            black: clock(0., 0.),
        };
        game.start(Setup::standard(), Some(clock), 0.)
            .expect("the longest and shortest clocks");
    }
}
