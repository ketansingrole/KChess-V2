//! `LocalGame` (`crates/kchess-wasm/js/gameSession.ts`): two people at one board, with optional clocks
//! and a stand-alone chess clock for a real board.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Clock, Clocked, Done, Flow, Reads, Session, Setup, Times, arg, board_result, number_text,
    opponent, optional_arg, pgn_result, timeout_winner, to_value, transition, with_move,
};

/// Both sides' clocks, as a session keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalClock {
    pub white: Clock,
    pub black: Clock,
}

/// A decided result: the winner, when there is one, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<String>,
    pub reason: String,
}

/// `LocalState`: the session document and its pause flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub setup: Setup,
    pub moves: Vec<String>,
    pub clock: Option<LocalClock>,
    pub times: Option<Times>,
    pub result: Option<GameResult>,
    pub paused: bool,
}

/// The private field the TypeScript class kept beside its state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub turn_started: f64,
}

/// A saved session as `localState` accepts it: every field may be missing.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    setup: Option<Setup>,
    moves: Option<Vec<String>>,
    clock: Option<LocalClock>,
    times: Option<Times>,
    result: Option<GameResult>,
}

/// `localState(saved?)`: a fresh state from a saved session.
pub fn state(args: &[Value]) -> Result<Value, String> {
    let saved: Saved = optional_arg(args, 0)?.unwrap_or_default();
    to_value(&State {
        setup: saved.setup.unwrap_or_else(|| Setup {
            variant: "standard".into(),
            fen: super::INITIAL_FEN.into(),
        }),
        moves: saved.moves.unwrap_or_default(),
        clock: saved.clock,
        times: saved.times,
        result: saved.result,
        paused: true,
    })
}

/// One board game during a transition.
type Game = Session<State, Runtime>;

impl Clocked for Game {
    fn setup(&self) -> &Setup {
        &self.state.setup
    }
    fn moves(&self) -> &[String] {
        &self.state.moves
    }
    fn times(&self) -> Option<Times> {
        self.state.times
    }
    fn turn_started(&self) -> f64 {
        self.rt.turn_started
    }

    /// `running`: a clock is set, the game goes on, and it has started.
    fn running(&self) -> Result<bool, String> {
        Ok(self.state.times.is_some()
            && !self.over()?
            && !self.state.paused
            && !self.state.moves.is_empty())
    }
}

impl Game {
    /// `result`: the decided result, else how the position ended, else null.
    fn result(&self) -> Result<Value, String> {
        if let Some(result) = &self.state.result {
            return to_value(result);
        }
        let info = self.info()?;
        let draw = self.draw()?;
        board_result(&self.state.setup.variant, &info.fen, draw.as_deref())
    }

    fn over(&self) -> Result<bool, String> {
        Ok(!self.result()?.is_null())
    }

    /// `expire(at)`: the side on the clock has run out of time.
    fn expire_at(&mut self, at: f64) -> Result<bool, String> {
        let Some(turn) = self.out_of_time(at)? else {
            return Ok(false);
        };
        if let Some(times) = self.state.times.as_mut() {
            times.set(turn, 0.0);
        }
        self.wrote("times");
        let info = self.info()?;
        let result = match timeout_winner(&self.state.setup.variant, &info.fen, turn)? {
            Some(winner) => GameResult {
                winner: Some(winner),
                reason: "Time out".into(),
            },
            None => GameResult {
                winner: None,
                reason: "Time out, but mate was impossible".into(),
            },
        };
        self.state.result = Some(result);
        self.wrote("result");
        self.fx.push(json!({ "type": "flagged" }));
        Ok(true)
    }

    fn clock_increment(&self, color: &str) -> f64 {
        match self.state.clock {
            Some(clock) if color == "white" => clock.white.increment,
            Some(clock) => clock.black.increment,
            None => 0.0,
        }
    }

    /// `move(uci)`: the move's SAN, or none when the move is not legal now.
    fn play(&mut self, reads: &mut Reads, uci: &str) -> Flow<Option<String>> {
        if self.over()? {
            return Ok(None);
        }
        let at = reads.now()?;
        if self.expire_at(at)? {
            return Ok(None);
        }
        let Some((moves, san)) = with_move(&self.state.setup, &self.state.moves, uci) else {
            return Ok(None);
        };
        let mover = self.turn()?;
        let increment = if self.state.moves.is_empty() {
            0.0
        } else {
            self.clock_increment(mover) * 1000.0
        };
        if let Some(times) = self.charged(mover, at, increment)? {
            self.state.times = Some(times);
            self.wrote("times");
        }
        self.rt.turn_started = at;
        self.state.paused = false;
        self.wrote("paused");
        self.state.moves = moves;
        self.wrote("moves");
        Ok(Some(san))
    }

    fn takeback(&mut self, reads: &mut Reads) -> Flow<()> {
        if self.state.moves.is_empty() {
            return Ok(());
        }
        self.state.moves.pop();
        self.wrote("moves");
        self.state.result = None;
        self.wrote("result");
        self.rt.turn_started = reads.now()?;
        Ok(())
    }

    fn start(&mut self, reads: &mut Reads, setup: Setup, clock: Option<LocalClock>) -> Flow<()> {
        self.state.setup = setup;
        self.wrote("setup");
        self.state.clock = clock;
        self.wrote("clock");
        self.state.moves = Vec::new();
        self.wrote("moves");
        self.state.result = None;
        self.wrote("result");
        self.state.paused = true;
        self.wrote("paused");
        self.state.times = clock.map(|clock| Times {
            white: clock.white.minutes * 60000.0,
            black: clock.black.minutes * 60000.0,
        });
        self.wrote("times");
        self.rt.turn_started = reads.now()?;
        Ok(())
    }

    fn resign(&mut self, color: &str) -> Flow<()> {
        if self.over()? {
            return Ok(());
        }
        let side = if color == "white" { "White" } else { "Black" };
        self.state.result = Some(GameResult {
            winner: Some(opponent(color)?),
            reason: format!("{side} resigned"),
        });
        self.wrote("result");
        Ok(())
    }

    fn agree_draw(&mut self) -> Flow<()> {
        if self.over()? {
            return Ok(());
        }
        self.state.result = Some(GameResult {
            winner: None,
            reason: "Draw agreed".into(),
        });
        self.wrote("result");
        Ok(())
    }

    fn toggle_pause(&mut self, reads: &mut Reads) -> Flow<()> {
        let at = reads.now()?;
        if self.state.times.is_none() || self.over()? || self.expire_at(at)? {
            return Ok(());
        }
        if !self.state.paused {
            self.state.times = Some(self.times_at(at)?);
            self.wrote("times");
        }
        self.state.paused = !self.state.paused;
        self.wrote("paused");
        self.rt.turn_started = at;
        Ok(())
    }

    /// `snapshot()`: the saved session, with each side's remaining time.
    fn snapshot(&self, reads: &mut Reads) -> Flow<Value> {
        let times = self.saved_times(reads)?;
        Ok(json!({
            "setup": self.state.setup.json(),
            "moves": self.state.moves,
            "clock": to_value(&self.state.clock)?,
            "times": times,
            "result": to_value(&self.state.result)?,
        }))
    }

    /// `archiveSnapshot()`: the game as the archive stores it.
    fn archive_snapshot(&self) -> Result<Value, String> {
        let result = self.result()?;
        let over = !result.is_null();
        let winner = result.get("winner").and_then(Value::as_str);
        let pgn = pgn_result(over, winner)?;
        let reason = result
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("In progress");
        let time_control = match self.state.clock {
            Some(clock) => format!(
                "{}+{} / {}+{}",
                number_text(clock.white.minutes * 60.0),
                number_text(clock.white.increment),
                number_text(clock.black.minutes * 60.0),
                number_text(clock.black.increment)
            ),
            None => "-".to_string(),
        };
        Ok(json!({
            "source": "board",
            "setup": self.state.setup.json(),
            "moves": self.state.moves,
            "white": "White",
            "black": "Black",
            "result": pgn,
            "reason": reason,
            "timeControl": time_control,
        }))
    }

    /// The getters, by name.
    fn view(&self, reads: &mut Reads, query: &Value) -> Flow<Value> {
        let name = query
            .get("view")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(match name {
            "over" => json!(self.over()?),
            "result" => self.result()?,
            "running" => json!(self.running()?),
            "remaining" => self.remaining_view(reads, query)?,
            "snapshot" => self.snapshot(reads)?,
            "archiveSnapshot" => self.archive_snapshot()?,
            other => {
                return Err(super::Stop::Error(format!(
                    "unknown local game view {other}"
                )));
            }
        })
    }
}

/// The runtime of a new game: the clock as the host reports it now.
pub fn init(args: &[Value]) -> Result<Value, String> {
    transition(args, |_, reads| {
        let runtime = Runtime {
            turn_started: reads.now()?,
        };
        Ok(Done {
            runtime: Some(to_value(&runtime)?),
            ..Done::default()
        })
    })
}

/// A getter of the game (`localView`), without changing it.
pub fn view(args: &[Value]) -> Result<Value, String> {
    Game::query(args, |game, reads, query| game.view(reads, query))
}

pub fn expire(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, inputs, reads| {
        let at = reads.now_or(inputs.get(2).and_then(Value::as_f64))?;
        Ok(json!(game.expire_at(at)?))
    })
}

pub fn play(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, inputs, reads| {
        let uci: String = arg(inputs, 2, "uci")?;
        Ok(json!(game.play(reads, &uci)?))
    })
}

pub fn takeback(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, _, reads| {
        game.takeback(reads)?;
        Ok(Value::Null)
    })
}

pub fn start(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, inputs, reads| {
        let setup: Setup = arg(inputs, 2, "setup")?;
        let clock: Option<LocalClock> = optional_arg(inputs, 3)?;
        game.start(reads, setup, clock)?;
        Ok(Value::Null)
    })
}

pub fn resign(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, inputs, _| {
        let color: String = arg(inputs, 2, "color")?;
        game.resign(&color)?;
        Ok(Value::Null)
    })
}

pub fn agree_draw(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, _, _| {
        game.agree_draw()?;
        Ok(Value::Null)
    })
}

pub fn toggle_pause(args: &[Value]) -> Result<Value, String> {
    Game::step(args, |game, _, reads| {
        game.toggle_pause(reads)?;
        Ok(Value::Null)
    })
}
