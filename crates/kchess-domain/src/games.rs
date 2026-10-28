//! Game sessions: the computer game, the
//! board game with clocks, and the archive identity and completion rules. The TypeScript
//! classes keep their API and their host; every decision is made here.
//!
//! A transition takes the session state (and its private runtime) as JSON, plus its input, and
//! returns the new state with the effects the host must perform, in order. Host reads (`now`,
//! `allowed`, `ready`, the archive's `source`, …) are asked for one at a time: when a transition
//! needs a read the host has not supplied yet, it answers `{"read": kind}` and the host calls
//! its getter once and asks again. So each read happens once, when the transition reaches it,
//! exactly as the TypeScript read it. The last argument of a transition is the list of reads
//! supplied so far.

mod archive;
mod computer;
mod local;
pub mod otb_clock;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use shakmaty::variant::VariantPosition;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::api;
use crate::js;
use crate::position::Info;
use crate::replay;
use crate::rules;

/// The standard starting position's FEN (`INITIAL_FEN`).
pub(crate) const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Why a transition stops early: it needs a host read it does not have yet, or it failed.
#[derive(Debug)]
pub(crate) enum Stop {
    Read(&'static str),
    Error(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Stop::Error(message)
    }
}

pub(crate) type Flow<T> = Result<T, Stop>;

/// The host reads supplied so far, in the order they are taken.
pub(crate) struct Reads<'a> {
    values: &'a Map<String, Value>,
    taken: HashMap<&'static str, usize>,
}

impl<'a> Reads<'a> {
    fn take(&mut self, kind: &'static str) -> Flow<Value> {
        let index = self.taken.entry(kind).or_insert(0);
        let value = self
            .values
            .get(kind)
            .and_then(Value::as_array)
            .and_then(|list| list.get(*index))
            .cloned();
        *index += 1;
        value.ok_or(Stop::Read(kind))
    }

    fn typed<T>(&mut self, kind: &'static str, read: impl FnOnce(&Value) -> Option<T>) -> Flow<T> {
        let value = self.take(kind)?;
        read(&value).ok_or_else(|| Stop::Error(format!("the host's {kind} read is malformed")))
    }

    pub(crate) fn now(&mut self) -> Flow<f64> {
        self.typed("now", Value::as_f64)
    }
    /// `at` when the host gave it, else the host's clock.
    pub(crate) fn now_or(&mut self, at: Option<f64>) -> Flow<f64> {
        match at {
            Some(at) => Ok(at),
            None => self.now(),
        }
    }
    pub(crate) fn allowed(&mut self) -> Flow<bool> {
        self.typed("allowed", Value::as_bool)
    }
    pub(crate) fn ready(&mut self) -> Flow<bool> {
        self.typed("ready", Value::as_bool)
    }
    pub(crate) fn has_play(&mut self) -> Flow<bool> {
        self.typed("hasPlay", Value::as_bool)
    }
    pub(crate) fn id(&mut self) -> Flow<String> {
        self.typed("id", |v| v.as_str().map(str::to_string))
    }
    pub(crate) fn source(&mut self) -> Flow<Value> {
        self.take("source")
    }
    pub(crate) fn games(&mut self) -> Flow<Vec<Value>> {
        self.typed("games", |v| v.as_array().cloned())
    }
}

/// What a transition returns when it completes.
#[derive(Default)]
pub(crate) struct Done {
    pub value: Value,
    pub state: Option<Value>,
    pub runtime: Option<Value>,
    /// The state fields the transition assigned, each replaced even when its value is unchanged.
    pub written: Vec<&'static str>,
    pub effects: Vec<Value>,
}

/// Run one transition: `args` are the inputs followed by the reads supplied so far.
pub(crate) fn transition(
    args: &[Value],
    body: impl FnOnce(&[Value], &mut Reads) -> Flow<Done>,
) -> Result<Value, String> {
    let (reads, inputs) = args.split_last().ok_or("the reads are missing")?;
    let values = reads.as_object().ok_or("the reads must be an object")?;
    let mut reads = Reads {
        values,
        taken: HashMap::new(),
    };
    match body(inputs, &mut reads) {
        Ok(done) => {
            let mut out = Map::new();
            out.insert("value".into(), done.value);
            if let Some(state) = done.state {
                out.insert("state".into(), state);
            }
            if let Some(runtime) = done.runtime {
                out.insert("runtime".into(), runtime);
            }
            out.insert("written".into(), json!(done.written));
            out.insert("effects".into(), Value::Array(done.effects));
            Ok(json!({ "done": out }))
        }
        Err(Stop::Read(kind)) => Ok(json!({ "read": kind })),
        Err(Stop::Error(message)) => Err(message),
    }
}

/// One game session during a transition: its state, its runtime and what the transition did.
pub(crate) struct Session<S, R> {
    pub state: S,
    pub rt: R,
    pub fx: Vec<Value>,
    pub written: Vec<&'static str>,
}

impl<S: Serialize + DeserializeOwned, R: Serialize + DeserializeOwned> Session<S, R> {
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

    pub(crate) fn wrote(&mut self, key: &'static str) {
        self.written.push(key);
    }

    /// A transition that changes the game: `body` returns the value the host receives.
    pub(crate) fn step(
        args: &[Value],
        body: impl FnOnce(&mut Self, &[Value], &mut Reads) -> Flow<Value>,
    ) -> Result<Value, String> {
        transition(args, |inputs, reads| {
            let mut game = Self::load(inputs)?;
            let value = body(&mut game, inputs, reads)?;
            game.done(value)
        })
    }

    /// A getter of the game, asked by its third input, without changing it.
    pub(crate) fn query(
        args: &[Value],
        get: impl FnOnce(&Self, &mut Reads, &Value) -> Flow<Value>,
    ) -> Result<Value, String> {
        transition(args, |inputs, reads| {
            let game = Self::load(inputs)?;
            let query: Value = arg(inputs, 2, "query")?;
            Ok(Done {
                value: get(&game, reads, &query)?,
                ..Done::default()
            })
        })
    }
}

/// The clock the board games (local and computer) share: each side's time, charged to the side to
/// move while the game says its clock runs.
pub(crate) trait Clocked {
    fn setup(&self) -> &Setup;
    fn moves(&self) -> &[String];
    fn times(&self) -> Option<Times>;
    /// When the side to move started its turn, on the host's clock.
    fn turn_started(&self) -> f64;
    /// `running`: the side to move is losing time now.
    fn running(&self) -> Result<bool, String>;

    fn info(&self) -> Result<Rc<Info>, String> {
        position_info(self.setup(), self.moves())
    }

    fn turn(&self) -> Result<&'static str, String> {
        Ok(self.info()?.turn)
    }

    fn draw(&self) -> Result<Option<String>, String> {
        draw_reason(self.setup(), self.moves())
    }

    fn remaining_at(&self, color: &str, at: f64) -> Result<f64, String> {
        let left = self.times().map_or(0.0, |times| times.get(color));
        if self.running()? && self.turn()? == color {
            Ok((left - (at - self.turn_started())).max(0.0))
        } else {
            Ok(left)
        }
    }

    /// `remaining(color, at?)`: `at` defaults to the host's clock, read when needed.
    fn remaining(&self, reads: &mut Reads, color: &str, at: Option<f64>) -> Flow<f64> {
        let at = reads.now_or(at)?;
        Ok(self.remaining_at(color, at)?)
    }

    /// The `remaining` view: `{ color, at? }`.
    fn remaining_view(&self, reads: &mut Reads, query: &Value) -> Flow<Value> {
        let color = query
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("white");
        let at = query.get("at").and_then(Value::as_f64);
        Ok(json!(self.remaining(reads, color, at)?))
    }

    /// Both sides' time at `at`: what a clock keeps when it stops.
    fn times_at(&self, at: f64) -> Result<Times, String> {
        Ok(Times {
            white: self.remaining_at("white", at)?,
            black: self.remaining_at("black", at)?,
        })
    }

    /// A snapshot's times: each side's remaining time now, or null without a clock.
    fn saved_times(&self, reads: &mut Reads) -> Flow<Value> {
        Ok(match self.times() {
            Some(_) => {
                let white = self.remaining(reads, "white", None)?;
                let black = self.remaining(reads, "black", None)?;
                json!({ "white": white, "black": black })
            }
            None => Value::Null,
        })
    }

    /// The side to move, when its time has run out at `at` on a running clock.
    fn out_of_time(&self, at: f64) -> Result<Option<&'static str>, String> {
        if !self.running()? {
            return Ok(None);
        }
        let turn = self.turn()?;
        Ok((self.remaining_at(turn, at)? <= 0.0).then_some(turn))
    }

    /// The clock once `mover` moves at `at` and gains `increment` milliseconds; none without one.
    fn charged(&self, mover: &str, at: f64, increment: f64) -> Result<Option<Times>, String> {
        let Some(mut times) = self.times() else {
            return Ok(None);
        };
        let spent = if self.running()? {
            at - self.turn_started()
        } else {
            0.0
        };
        let left = (times.get(mover) - spent).max(0.0);
        times.set(mover, left + increment);
        Ok(Some(times))
    }
}

/// The moves once `uci` is played after `moves`, with its SAN; none when it is not legal there.
pub(crate) fn with_move(
    setup: &Setup,
    moves: &[String],
    uci: &str,
) -> Option<(Vec<String>, String)> {
    let mut next = moves.to_vec();
    next.push(uci.to_string());
    let texts: Vec<&str> = next.iter().map(String::as_str).collect();
    let replayed = replay::replay_setup(&setup.variant, &setup.fen, &texts)?;
    if replayed.played.len() != moves.len() + 1 {
        return None;
    }
    let san = replayed.played.last()?.san.clone();
    Some((next, san))
}

/// A `{ variant, fen }` setup, as the TypeScript `GameSetup` carries it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub variant: String,
    pub fen: String,
}

impl Setup {
    pub(crate) fn json(&self) -> Value {
        json!({ "variant": self.variant, "fen": self.fen })
    }

    /// `isStandardStart`: the ordinary game, from the usual start.
    pub(crate) fn is_standard_start(&self) -> bool {
        self.variant == "standard" && self.fen == INITIAL_FEN
    }
}

/// A clock for one side: minutes and the increment in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    pub minutes: f64,
    pub increment: f64,
}

/// Each side's remaining time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Times {
    pub white: f64,
    pub black: f64,
}

impl Times {
    pub(crate) fn get(&self, color: &str) -> f64 {
        if color == "white" {
            self.white
        } else {
            self.black
        }
    }

    pub(crate) fn set(&mut self, color: &str, value: f64) {
        if color == "white" {
            self.white = value;
        } else {
            self.black = value;
        }
    }
}

/// Argument `i` as a typed value; a missing or null argument is an error.
pub(crate) fn arg<T: DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T, String> {
    let value = args
        .get(i)
        .filter(|value| !value.is_null())
        .ok_or_else(|| format!("{name} is missing"))?;
    serde_json::from_value(value.clone()).map_err(|error| format!("{name}: {error}"))
}

/// Argument `i` as a typed value when the host gave one (null is absent).
pub(crate) fn optional_arg<T: DeserializeOwned>(
    args: &[Value],
    i: usize,
) -> Result<Option<T>, String> {
    match args.get(i).filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| error.to_string()),
    }
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

/// The position a setup reaches after `moves` (`setupPositionAfter`): the standard start when
/// the setup is not a legal position.
pub(crate) fn position_after<S: AsRef<str>>(
    setup: &Setup,
    moves: &[S],
) -> Result<VariantPosition, String> {
    let texts: Vec<&str> = moves.iter().map(|m| m.as_ref()).collect();
    replay::replay_setup(&setup.variant, &setup.fen, &texts)
        .and_then(|replayed| rules::setup_start(&setup.variant, &replayed.fen))
        .or_else(|| rules::setup_start("standard", INITIAL_FEN))
        .ok_or_else(|| "the standard start is not a position".to_string())
}

/// Values derived from a setup and its moves, kept for the few games most recently asked about. A
/// clock ticks several times a second and asks for the turn, the result and the draw each time;
/// without this, every question replays the whole game. A changed setup or move list is a miss.
pub(crate) struct Memo<T>(RefCell<VecDeque<(Setup, Vec<String>, T)>>);

impl<T: Clone> Memo<T> {
    /// Room for both clocked games (local and computer) and the moves just before a takeback.
    const CAPACITY: usize = 4;

    pub(crate) const fn new() -> Self {
        Self(RefCell::new(VecDeque::new()))
    }

    /// The value for this setup and these moves; computed again only when they changed.
    pub(crate) fn get(
        &self,
        setup: &Setup,
        moves: &[String],
        compute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        {
            let mut entries = self.0.borrow_mut();
            let known = entries.iter().position(|(known_setup, known_moves, _)| {
                known_setup == setup && known_moves.as_slice() == moves
            });
            // The most recently asked about is dropped last.
            if let Some(entry) = known.and_then(|i| entries.remove(i)) {
                let value = entry.2.clone();
                entries.push_back(entry);
                return Ok(value);
            }
        }
        let value = compute()?;
        let mut entries = self.0.borrow_mut();
        if entries.len() == Self::CAPACITY {
            entries.pop_front();
        }
        entries.push_back((setup.clone(), moves.to_vec(), value.clone()));
        Ok(value)
    }
}

thread_local! {
    /// The position a setup and its moves reach.
    pub(crate) static POSITIONS: Memo<Rc<Info>> = const { Memo::new() };
    /// `setupDrawReason` of a setup and its moves.
    pub(crate) static DRAWS: Memo<Option<String>> = const { Memo::new() };
}

/// `position::info` of the position a setup and its moves reach.
pub(crate) fn position_info(setup: &Setup, moves: &[String]) -> Result<Rc<Info>, String> {
    POSITIONS.with(|memo| {
        memo.get(setup, moves, || {
            Ok(Rc::new(crate::position::info(&position_after(
                setup, moves,
            )?)))
        })
    })
}

/// `setupDrawReason` of a setup and its moves.
pub(crate) fn draw_reason(setup: &Setup, moves: &[String]) -> Result<Option<String>, String> {
    DRAWS.with(|memo| memo.get(setup, moves, || setup_draw_reason(setup, moves)))
}

/// Call a rule this crate already provides through another module's dispatcher.
fn rule(method: &str, args: &[Value]) -> Result<Value, String> {
    crate::records::call(method, args)
        .or_else(|| crate::training::call(method, args))
        .unwrap_or_else(|| Err(format!("missing rule {method}")))
}

/// `setupDrawReason(setup, moves)`: the repetition or fifty-move draw already reached.
pub(crate) fn setup_draw_reason<S: AsRef<str>>(
    setup: &Setup,
    moves: &[S],
) -> Result<Option<String>, String> {
    let texts: Vec<&str> = moves.iter().map(|m| m.as_ref()).collect();
    let args = json!([setup.json(), texts]).to_string();
    let out = api::call("setupDrawReason", &args)?;
    let value: Value = serde_json::from_str(&out).map_err(|error| error.to_string())?;
    Ok(value.as_str().map(str::to_string))
}

/// `opponent(color)`.
pub(crate) fn opponent(color: &str) -> Result<String, String> {
    rule("opponent", &[json!(color)]).map(|value| value.as_str().unwrap_or_default().to_string())
}

/// `timeoutWinner(position, flagged)`.
pub(crate) fn timeout_winner(
    variant: &str,
    fen: &str,
    flagged: &str,
) -> Result<Option<String>, String> {
    let value = rule(
        "timeoutWinner",
        &[json!({ "variant": variant, "fen": fen }), json!(flagged)],
    )?;
    Ok(value.as_str().map(str::to_string))
}

/// `boardResult(position, variant, draw)`: the result object, or null while the game goes on.
pub(crate) fn board_result(variant: &str, fen: &str, draw: Option<&str>) -> Result<Value, String> {
    rule(
        "boardResult",
        &[
            json!({ "variant": variant, "fen": fen }),
            json!(variant),
            json!(draw),
        ],
    )
}

/// `pgnResult(over, winner)`.
pub(crate) fn pgn_result(over: bool, winner: Option<&str>) -> Result<String, String> {
    let value = rule("pgnResult", &[json!(over), json!(winner)])?;
    Ok(value.as_str().unwrap_or("*").to_string())
}

/// The winner the position itself names, if any (`position.outcome()?.winner`).
pub(crate) fn outcome_winner(info: &Info) -> Option<String> {
    info.outcome
        .as_ref()
        .and_then(|outcome| outcome.get("winner"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// `engineLevelInfo(level)`: the object for a ladder level; an error for an unknown one.
pub(crate) fn engine_level(level: &str) -> Result<Value, String> {
    match crate::training::call("engineLevelInfo", &[json!(level)]) {
        Some(Ok(Value::Null)) | None => Err(format!("unknown engine level {level}")),
        Some(result) => result,
    }
}

/// `JS` number text, as a template string prints it.
pub(crate) fn number_text(n: f64) -> String {
    js::number_to_string(n)
}

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "computerState" => computer::state(args),
        "computerInit" => computer::init(args),
        "computerView" => computer::view(args),
        "computerAvailability" => computer::availability(args),
        "computerExpire" => computer::expire(args),
        "computerMove" => computer::play(args),
        "computerSearchStart" => computer::search_start(args),
        "computerSearchReply" => computer::search_reply(args),
        "computerSearchFailed" => computer::search_failed(args),
        "computerSearchEnd" => computer::search_end(args),
        "computerStart" => computer::start(args),
        "computerTakeback" => computer::takeback(args),
        "computerResign" => computer::resign(args),
        "computerDispose" => computer::dispose(args),
        "localState" => local::state(args),
        "localInit" => local::init(args),
        "localView" => local::view(args),
        "localExpire" => local::expire(args),
        "localMove" => local::play(args),
        "localTakeback" => local::takeback(args),
        "localStart" => local::start(args),
        "localResign" => local::resign(args),
        "localAgreeDraw" => local::agree_draw(args),
        "localTogglePause" => local::toggle_pause(args),
        "archiveState" => archive::state(args),
        "archiveInit" => archive::init(args),
        "archiveSave" => archive::save(args),
        "archiveReset" => archive::reset(args),
        _ => return otb_clock::call(method, args),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setup_reaches_the_standard_start_when_illegal() {
        let illegal = Setup {
            variant: "standard".into(),
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".into(),
        };
        let start = position_after(&illegal, &[] as &[String]).expect("the standard start");
        assert_eq!(rules::make_fen(&start), INITIAL_FEN);
    }

    #[test]
    fn a_memo_is_worked_out_again_when_the_setup_or_moves_change() {
        let start = Setup {
            variant: "standard".into(),
            fen: INITIAL_FEN.into(),
        };
        let memo = Memo::new();
        let computed = std::cell::Cell::new(0);
        let get = |setup: &Setup, moves: &[String]| {
            memo.get(setup, moves, || {
                computed.set(computed.get() + 1);
                Ok(moves.len())
            })
        };
        let e4 = vec!["e2e4".to_string()];
        assert_eq!(get(&start, &e4), Ok(1));
        assert_eq!(get(&start, &e4), Ok(1));
        assert_eq!(computed.get(), 1);
        assert_eq!(get(&start, &[]), Ok(0));
        assert_eq!(get(&start, &["d2d4".to_string()]), Ok(1));
        let chess960 = Setup {
            variant: "chess960".into(),
            ..start.clone()
        };
        assert_eq!(get(&chess960, &["d2d4".to_string()]), Ok(1));
        assert_eq!(computed.get(), 4);
        // The four most recent are kept: e4 is still known, the empty start was dropped.
        assert_eq!(get(&start, &e4), Ok(1));
        assert_eq!(computed.get(), 4);
        let extra = vec!["g1f3".to_string()];
        assert_eq!(get(&start, &extra), Ok(1));
        assert_eq!(get(&start, &[]), Ok(0));
        assert_eq!(computed.get(), 6);
    }

    #[test]
    fn a_missing_read_asks_the_host_for_it() {
        let empty = Map::new();
        let mut reads = Reads {
            values: &empty,
            taken: HashMap::new(),
        };
        assert!(matches!(reads.now(), Err(Stop::Read("now"))));
    }
}
