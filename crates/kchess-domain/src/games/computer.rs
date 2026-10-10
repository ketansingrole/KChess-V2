//! `ComputerGame` (`crates/kchess-wasm/js/gameSession.ts`): a game against the engine, its clock, the
//! engine's replies and the cancellation of work that is no longer wanted.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Clock, Done, Flow, Reads, Setup, Times, arg, board_result, engine_level, number_text, opponent,
    optional_arg, outcome_winner, pgn_result, position_after, setup_draw_reason, timeout_winner,
    to_value, transition,
};
use crate::position::{self, Info};
use crate::replay;

/// `ComputerState`: the session document and the engine turn's bookkeeping.
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

/// The private fields the TypeScript class kept beside its state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub turn_started: f64,
    pub permitted: bool,
    pub disposed: bool,
}

/// A saved session as `computerState` accepts it: every field may be missing.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    moves: Option<Vec<String>>,
    ply: Option<usize>,
    level: Option<String>,
    color: Option<String>,
    resigned: Option<bool>,
    setup: Option<Setup>,
    clock: Option<Clock>,
    times: Option<Times>,
    flagged: Option<String>,
}

#[derive(Deserialize)]
struct Epoch {
    epoch: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    epoch: u64,
    count: usize,
    #[serde(rename = "move")]
    reply: String,
}

/// `computerState(saved?)`: a fresh state from a saved session.
pub fn state(args: &[Value]) -> Result<Value, String> {
    let saved: Saved = optional_arg(args, 0)?.unwrap_or_default();
    to_value(&State {
        moves: saved.moves.unwrap_or_default(),
        ply: saved.ply.unwrap_or(0),
        level: saved.level.unwrap_or_else(|| "club".into()),
        color: saved.color.unwrap_or_else(|| "white".into()),
        resigned: saved.resigned.unwrap_or(false),
        setup: saved.setup.unwrap_or_else(|| Setup {
            variant: "standard".into(),
            fen: super::INITIAL_FEN.into(),
        }),
        clock: saved.clock,
        times: saved.times,
        flagged: saved.flagged,
        thinking: false,
        epoch: 0,
    })
}

/// One computer game during a transition: its state, runtime and the effects it has produced.
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

    /// `over`: resigned, flagged, the position ended, or a draw reached.
    fn over(&self) -> Result<bool, String> {
        if self.state.resigned || self.state.flagged.is_some() {
            return Ok(true);
        }
        if self.info()?.end {
            return Ok(true);
        }
        Ok(self.draw()?.is_some())
    }

    /// `winner`: the side that won, or none for a draw or a game going on.
    fn winner(&self) -> Result<Option<String>, String> {
        if !self.over()? {
            return Ok(None);
        }
        if self.state.resigned {
            return Ok(Some(opponent(&self.state.color)?));
        }
        let info = self.info()?;
        match &self.state.flagged {
            Some(flagged) => timeout_winner(&self.state.setup.variant, &info.fen, flagged),
            None => Ok(outcome_winner(&info)),
        }
    }

    /// `result`: the result from the player's side, or null while the game goes on.
    fn result(&self) -> Result<Value, String> {
        if !self.over()? {
            return Ok(Value::Null);
        }
        if self.state.resigned {
            return Ok(json!({
                "kind": "loss",
                "title": "Stockfish won",
                "detail": "You resigned",
            }));
        }
        let winner = self.winner()?;
        let won = winner.as_deref() == Some(self.state.color.as_str());
        let detail = if self.state.flagged.is_some() {
            if winner.is_none() {
                "Time ran out, but mate was impossible".to_string()
            } else if won {
                "Stockfish ran out of time".to_string()
            } else {
                "You ran out of time".to_string()
            }
        } else {
            let info = self.info()?;
            let draw = self.draw()?;
            let reason = board_result(&self.state.setup.variant, &info.fen, draw.as_deref())?;
            reason
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Game over")
                .to_string()
        };
        let (kind, title) = if winner.is_none() {
            ("draw", "Draw")
        } else if won {
            ("win", "You won")
        } else {
            ("loss", "Stockfish won")
        };
        Ok(json!({ "kind": kind, "title": title, "detail": detail }))
    }

    /// `running`: a clock is set, both sides have moved and the game goes on.
    fn running(&self) -> Result<bool, String> {
        Ok(self.rt.permitted
            && self.state.times.is_some()
            && !self.over()?
            && self.state.moves.len() >= 2)
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

    /// Stop the engine and invalidate any reply it may still send.
    fn wrote(&mut self, key: &'static str) {
        self.written.push(key);
    }

    fn cancel(&mut self) {
        self.state.epoch += 1;
        self.wrote("epoch");
        self.state.thinking = false;
        self.wrote("thinking");
        self.fx.push(json!({ "type": "stopEngine" }));
    }

    /// `expire(at)`: flag the side on the clock when its time is up.
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
        self.state.flagged = Some(turn.to_string());
        self.wrote("flagged");
        self.cancel();
        self.fx.push(json!({ "type": "flagged" }));
        Ok(true)
    }

    /// `commit(uci, at, computer)`: play a legal move, charge the clock and report it.
    fn commit(&mut self, uci: &str, at: f64, computer: bool) -> Result<bool, String> {
        let mut moves = self.state.moves.clone();
        moves.push(uci.to_string());
        let texts: Vec<&str> = moves.iter().map(String::as_str).collect();
        let Some(replayed) =
            replay::replay_setup(&self.state.setup.variant, &self.state.setup.fen, &texts)
        else {
            return Ok(false);
        };
        if replayed.played.len() != self.state.moves.len() + 1 {
            return Ok(false);
        }
        let Some(san) = replayed.played.last().map(|played| played.san.clone()) else {
            return Ok(false);
        };
        let mover = self.turn()?;
        if let Some(mut times) = self.state.times {
            let spent = if self.running()? {
                at - self.rt.turn_started
            } else {
                0.0
            };
            let increment = if self.state.moves.len() >= 2 {
                self.state.clock.map_or(0.0, |clock| clock.increment) * 1000.0
            } else {
                0.0
            };
            let left = (times.get(mover) - spent).max(0.0);
            times.set(mover, left + increment);
            self.state.times = Some(times);
            self.wrote("times");
        }
        self.rt.turn_started = at;
        self.state.moves = moves;
        self.wrote("moves");
        self.state.ply = self.state.moves.len();
        self.wrote("ply");
        self.fx
            .push(json!({ "type": "moved", "san": san, "computer": computer }));
        Ok(true)
    }

    /// `move(uci)`: the player's move, when it is theirs to make.
    fn play(&mut self, reads: &mut Reads, uci: &str) -> Flow<bool> {
        if self.rt.disposed {
            return Ok(false);
        }
        if !reads.allowed()? {
            return Ok(false);
        }
        if !reads.ready()? {
            return Ok(false);
        }
        if self.state.ply != self.state.moves.len()
            || self.over()?
            || self.state.thinking
            || self.turn()? != self.state.color.as_str()
        {
            return Ok(false);
        }
        let at = reads.now()?;
        if self.expire_at(at)? {
            return Ok(false);
        }
        if !self.commit(uci, at, false)? {
            return Ok(false);
        }
        self.fx.push(json!({ "type": "computerTurn" }));
        Ok(true)
    }

    /// The start of an engine turn, when one is due: the request to send and its epoch.
    fn search_start(&mut self, reads: &mut Reads) -> Flow<Option<Value>> {
        if self.rt.disposed || self.state.thinking {
            return Ok(None);
        }
        if self.turn()? == self.state.color.as_str() || self.over()? {
            return Ok(None);
        }
        if !reads.ready()? || !reads.allowed()? {
            return Ok(None);
        }
        let epoch = self.state.epoch;
        let count = self.state.moves.len();
        self.state.thinking = true;
        self.wrote("thinking");
        let turn = self.turn()?;
        let left = self.remaining(reads, turn, None)?;
        let movetime = match self.state.times {
            Some(_) => {
                let level = engine_level(&self.state.level)?;
                let time = level
                    .get("time")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "the engine level has no think time".to_string())?;
                let increment = self.state.clock.map_or(0.0, |clock| clock.increment);
                let budget = (left / 30.0 + increment * 800.0).floor();
                Some(50.0_f64.max(5000.0_f64.min(time).min(budget)))
            }
            None => None,
        };
        let mut options = serde_json::Map::new();
        if !self.state.setup.is_standard_start() {
            options.insert("fen".into(), json!(self.state.setup.fen));
        }
        if self.state.setup.variant == "chess960" {
            options.insert("chess960".into(), json!(true));
        }
        if let Some(movetime) = movetime {
            options.insert("movetime".into(), json!(movetime));
        }
        Ok(Some(json!({
            "epoch": epoch,
            "count": count,
            "moves": self.state.moves,
            "level": self.state.level,
            "options": Value::Object(options),
        })))
    }

    /// The engine's reply, if it is still wanted: the move is played (or the clock flags).
    fn search_reply(&mut self, reads: &mut Reads, reply: &Reply) -> Flow<()> {
        if self.rt.disposed
            || reply.epoch != self.state.epoch
            || reply.count != self.state.moves.len()
            || self.over()?
        {
            return Ok(());
        }
        if !reads.allowed()? {
            return Ok(());
        }
        let at = reads.now()?;
        if !self.expire_at(at)? {
            self.commit(&reply.reply, at, true)?;
        }
        Ok(())
    }

    /// The engine turn failed: report it unless the engine turn was cancelled meanwhile.
    fn search_failed(&mut self, epoch: u64) {
        if epoch == self.state.epoch && !self.rt.disposed {
            self.fx.push(json!({ "type": "failed" }));
        } else {
            self.fx.push(json!({ "type": "debug" }));
        }
    }

    /// The end of an engine turn, however it finished.
    fn search_end(&mut self, epoch: u64) {
        if epoch == self.state.epoch {
            self.state.thinking = false;
            self.wrote("thinking");
        }
    }

    fn start(&mut self, reads: &mut Reads, setup: Setup, clock: Option<Clock>) -> Flow<()> {
        if self.rt.disposed {
            return Ok(());
        }
        self.cancel();
        self.state.setup = setup;
        self.wrote("setup");
        self.state.clock = clock;
        self.wrote("clock");
        self.state.moves = Vec::new();
        self.wrote("moves");
        self.state.ply = 0;
        self.wrote("ply");
        self.state.resigned = false;
        self.wrote("resigned");
        self.state.flagged = None;
        self.wrote("flagged");
        self.state.times = clock.map(|clock| {
            let ms = clock.minutes * 60000.0;
            Times {
                white: ms,
                black: ms,
            }
        });
        self.wrote("times");
        self.rt.turn_started = reads.now()?;
        self.fx.push(json!({ "type": "computerTurn" }));
        Ok(())
    }

    /// `takeback()`: undo back to the player's last turn.
    fn takeback(&mut self, reads: &mut Reads) -> Flow<()> {
        if self.rt.disposed || self.state.moves.is_empty() {
            return Ok(());
        }
        self.cancel();
        self.state.resigned = false;
        self.wrote("resigned");
        self.state.flagged = None;
        self.wrote("flagged");
        let mut kept = self.state.moves.clone();
        kept.pop();
        while !kept.is_empty()
            && position::info(&position_after(&self.state.setup, &kept)?).turn
                != self.state.color.as_str()
        {
            kept.pop();
        }
        self.state.moves = kept;
        self.wrote("moves");
        self.state.ply = self.state.moves.len();
        self.wrote("ply");
        self.rt.turn_started = reads.now()?;
        self.fx.push(json!({ "type": "computerTurn" }));
        Ok(())
    }

    fn resign(&mut self) -> Flow<()> {
        if self.rt.disposed || self.state.moves.is_empty() || self.over()? {
            return Ok(());
        }
        self.cancel();
        self.state.resigned = true;
        self.wrote("resigned");
        self.state.ply = self.state.moves.len();
        self.wrote("ply");
        Ok(())
    }

    fn dispose(&mut self) {
        if self.rt.disposed {
            return;
        }
        self.rt.disposed = true;
        if self.state.thinking {
            self.cancel();
        } else {
            self.state.epoch += 1;
            self.wrote("epoch");
        }
    }

    /// `availabilityChanged()`: freeze the clocks and cancel the engine when assistance stops.
    fn availability(&mut self, reads: &mut Reads) -> Flow<()> {
        let permitted = reads.allowed()?;
        let at = reads.now()?;
        if self.rt.permitted && !permitted {
            if self.state.times.is_some() {
                let white = self.remaining_at("white", at)?;
                let black = self.remaining_at("black", at)?;
                self.state.times = Some(Times { white, black });
                self.wrote("times");
            }
            self.cancel();
        }
        if permitted != self.rt.permitted {
            self.rt.turn_started = at;
        }
        self.rt.permitted = permitted;
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
        let clock = match self.state.clock {
            Some(clock) => json!({ "minutes": clock.minutes, "increment": clock.increment }),
            None => Value::Null,
        };
        Ok(json!({
            "moves": self.state.moves,
            "ply": self.state.ply,
            "level": self.state.level,
            "color": self.state.color,
            "resigned": self.state.resigned,
            "setup": self.state.setup.json(),
            "clock": clock,
            "times": times,
            "flagged": self.state.flagged,
        }))
    }

    /// `archiveSnapshot()`: the game as the archive stores it.
    fn archive_snapshot(&self) -> Result<Value, String> {
        let level = engine_level(&self.state.level)?;
        let label = level
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let engine = format!("Stockfish ({label})");
        let (white, black) = if self.state.color == "white" {
            ("You".to_string(), engine.clone())
        } else {
            (engine.clone(), "You".to_string())
        };
        let over = self.over()?;
        let winner = self.winner()?;
        let result = pgn_result(over, winner.as_deref())?;
        let reason = self
            .result()?
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("In progress")
            .to_string();
        let time_control = match self.state.clock {
            Some(clock) => format!(
                "{}+{}",
                number_text(clock.minutes * 60.0),
                number_text(clock.increment)
            ),
            None => "-".to_string(),
        };
        Ok(json!({
            "source": "computer",
            "setup": self.state.setup.json(),
            "moves": self.state.moves,
            "white": white,
            "black": black,
            "result": result,
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
            "winner" => json!(self.winner()?),
            "result" => self.result()?,
            "running" => json!(self.running()?),
            "draw" => json!(self.draw()?),
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
            other => return Err(super::Stop::Error(format!("unknown computer view {other}"))),
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

/// The runtime of a new game: the clock and availability as the host reports them now.
pub fn init(args: &[Value]) -> Result<Value, String> {
    transition(args, |_, reads| {
        let turn_started = reads.now()?;
        let permitted = reads.allowed()?;
        let runtime = Runtime {
            turn_started,
            permitted,
            disposed: false,
        };
        Ok(Done {
            runtime: Some(to_value(&runtime)?),
            ..Done::default()
        })
    })
}

/// A getter of the game (`computerView`), without changing it.
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

pub fn availability(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, reads| {
        game.availability(reads)?;
        Ok(Value::Null)
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

pub fn search_start(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, reads| {
        Ok(game.search_start(reads)?.unwrap_or(Value::Null))
    })
}

pub fn search_reply(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, reads| {
        let reply: Reply = arg(inputs, 2, "reply")?;
        game.search_reply(reads, &reply)?;
        Ok(Value::Null)
    })
}

pub fn search_failed(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, _| {
        let epoch: Epoch = arg(inputs, 2, "epoch")?;
        game.search_failed(epoch.epoch);
        Ok(Value::Null)
    })
}

pub fn search_end(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, _| {
        let epoch: Epoch = arg(inputs, 2, "epoch")?;
        game.search_end(epoch.epoch);
        Ok(Value::Null)
    })
}

pub fn start(args: &[Value]) -> Result<Value, String> {
    step(args, |game, inputs, reads| {
        let setup: Setup = arg(inputs, 2, "setup")?;
        let clock: Option<Clock> = optional_arg(inputs, 3)?;
        game.start(reads, setup, clock)?;
        Ok(Value::Null)
    })
}

pub fn takeback(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, reads| {
        game.takeback(reads)?;
        Ok(Value::Null)
    })
}

pub fn resign(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, _| {
        game.resign()?;
        Ok(Value::Null)
    })
}

pub fn dispose(args: &[Value]) -> Result<Value, String> {
    step(args, |game, _, _| {
        game.dispose();
        Ok(Value::Null)
    })
}
