//! `LocalGame` (`core/src/domain/gameSession.ts`): two people at one board, with optional clocks
//! and a stand-alone chess clock for a real board.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Clock, Done, Flow, Reads, Setup, Times, arg, board_result, number_text, opponent, optional_arg,
    pgn_result, position_after, setup_draw_reason, timeout_winner, to_value, transition,
};
use crate::position::{self, Info};
use crate::replay;

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
struct Game {
    state: State,
    rt: Runtime,
    fx: Vec<Value>,
    written: Vec<&'static str>,
}

impl Game {
    fn load(args: &[Value]) -> Result<Self, String> {
        Ok(Self {
            state: arg(args, 0, "state")?,
            rt: arg(args, 1, "runtime")?,
            fx: Vec::new(),
            written: Vec::new(),
        })
    }

    fn done(self, value: Value) -> Flow<Done> {
        Ok(Done {
            value,
            state: Some(to_value(&self.state)?),
            runtime: Some(to_value(&self.rt)?),
            written: self.written,
            effects: self.fx,
        })
    }

    fn position(&self) -> Result<shakmaty::variant::VariantPosition, String> {
        position_after(&self.state.setup, &self.state.moves)
    }

    fn info(&self) -> Result<Info, String> {
        Ok(position::info(&self.position()?))
    }

    fn turn(&self) -> Result<&'static str, String> {
        Ok(self.info()?.turn)
    }

    fn draw(&self) -> Result<Option<String>, String> {
        setup_draw_reason(&self.state.setup, &self.state.moves)
    }

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

    /// `running`: a clock is set, the game goes on, and it has started.
    fn running(&self) -> Result<bool, String> {
        Ok(self.state.times.is_some()
            && !self.over()?
            && !self.state.paused
            && !self.state.moves.is_empty())
    }

    fn remaining_at(&self, color: &str, at: f64) -> Result<f64, String> {
        let left = self.state.times.map_or(0.0, |times| times.get(color));
        if self.running()? && self.turn()? == color {
            Ok((left - (at - self.rt.turn_started)).max(0.0))
        } else {
            Ok(left)
        }
    }

    /// `remaining(color, at?)`: `at` defaults to the host's clock, read when needed.
    fn remaining(&self, reads: &mut Reads, color: &str, at: Option<f64>) -> Flow<f64> {
        let at = match at {
            Some(at) => at,
            None => reads.now()?,
        };
        Ok(self.remaining_at(color, at)?)
    }

    /// `expire(at)`: the side on the clock has run out of time.
    fn expire_at(&mut self, at: f64) -> Result<bool, String> {
        if !self.running()? {
            return Ok(false);
        }
        let turn = self.turn()?;
        if self.remaining_at(turn, at)? > 0.0 {
            return Ok(false);
        }
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

    fn wrote(&mut self, key: &'static str) {
        self.written.push(key);
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
        let mut moves = self.state.moves.clone();
        moves.push(uci.to_string());
        let texts: Vec<&str> = moves.iter().map(String::as_str).collect();
        let Some(replayed) =
            replay::replay_setup(&self.state.setup.variant, &self.state.setup.fen, &texts)
        else {
            return Ok(None);
        };
        if replayed.played.len() != self.state.moves.len() + 1 {
            return Ok(None);
        }
        let Some(san) = replayed.played.last().map(|played| played.san.clone()) else {
            return Ok(None);
        };
        let mover = self.turn()?;
        if let Some(mut times) = self.state.times {
            let spent = if self.running()? {
                at - self.rt.turn_started
            } else {
                0.0
            };
            let increment = if self.state.moves.is_empty() {
                0.0
            } else {
                self.clock_increment(mover) * 1000.0
            };
            let left = (times.get(mover) - spent).max(0.0);
            times.set(mover, left + increment);
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
            let white = self.remaining_at("white", at)?;
            let black = self.remaining_at("black", at)?;
            self.state.times = Some(Times { white, black });
            self.wrote("times");
        }
        self.state.paused = !self.state.paused;
        self.wrote("paused");
        self.rt.turn_started = at;
        Ok(())
    }

    /// `snapshot()`: the saved session, with each side's remaining time.
    fn snapshot(&self, reads: &mut Reads) -> Flow<Value> {
        let times = match self.state.times {
            Some(_) => {
                let white = self.remaining(reads, "white", None)?;
                let black = self.remaining(reads, "black", None)?;
                json!({ "white": white, "black": black })
            }
            None => Value::Null,
        };
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
            "remaining" => {
                let color = query
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("white");
                let at = query.get("at").and_then(Value::as_f64);
                json!(self.remaining(reads, color, at)?)
            }
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

/// A transition that changes the game: `body` returns the value the host receives.
fn step(
    args: &[Value],
    body: impl FnOnce(&mut Game, &[Value], &mut Reads) -> Flow<Value>,
) -> Result<Value, String> {
    transition(args, |inputs, reads| {
        let mut game = Game::load(inputs)?;
        let value = body(&mut game, inputs, reads)?;
        game.done(value)
    })
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
    transition(args, |inputs, reads| {
        let game = Game::load(inputs)?;
        let query: Value = arg(inputs, 2, "query")?;
        let value = game.view(reads, &query)?;
        Ok(Done {
            value,
            ..Done::default()
        })
    })
}

pub fn expire(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, reads| {
        let at = match inputs.get(2).and_then(Value::as_f64) {
            Some(at) => at,
            None => reads.now()?,
        };
        Ok(json!(game.expire_at(at)?))
    })
}

pub fn play(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, reads| {
        let uci: String = arg(inputs, 2, "uci")?;
        Ok(json!(game.play(reads, &uci)?))
    })
}

pub fn takeback(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, reads| {
        game.takeback(reads)?;
        Ok(Value::Null)
    })
}

pub fn start(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, reads| {
        let setup: Setup = arg(inputs, 2, "setup")?;
        let clock: Option<LocalClock> = optional_arg(inputs, 3)?;
        game.start(reads, setup, clock)?;
        Ok(Value::Null)
    })
}

pub fn resign(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, _| {
        let color: String = arg(inputs, 2, "color")?;
        game.resign(&color)?;
        Ok(Value::Null)
    })
}

pub fn agree_draw(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, _| {
        game.agree_draw()?;
        Ok(Value::Null)
    })
}

pub fn toggle_pause(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, reads| {
        game.toggle_pause(reads)?;
        Ok(Value::Null)
    })
}
