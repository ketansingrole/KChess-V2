//! Game sessions (`core/src/domain/gameSession.ts`, `gameArchive.ts`): the computer game, the
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

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use shakmaty::variant::VariantPosition;
use std::collections::HashMap;

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
pub(crate) fn position_after(setup: &Setup, moves: &[String]) -> Result<VariantPosition, String> {
    let texts: Vec<&str> = moves.iter().map(String::as_str).collect();
    replay::replay_setup(&setup.variant, &setup.fen, &texts)
        .and_then(|replayed| rules::setup_start(&setup.variant, &replayed.fen))
        .or_else(|| rules::setup_start("standard", INITIAL_FEN))
        .ok_or_else(|| "the standard start is not a position".to_string())
}

/// Call a rule this crate already provides through another module's dispatcher.
fn rule(method: &str, args: &[Value]) -> Result<Value, String> {
    crate::records::call(method, args)
        .or_else(|| crate::training::call(method, args))
        .unwrap_or_else(|| Err(format!("missing rule {method}")))
}

/// `setupDrawReason(setup, moves)`: the repetition or fifty-move draw already reached.
pub(crate) fn setup_draw_reason(setup: &Setup, moves: &[String]) -> Result<Option<String>, String> {
    let args = json!([setup.json(), moves]).to_string();
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
        _ => return None,
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
        let start = position_after(&illegal, &[]).expect("the standard start");
        assert_eq!(rules::make_fen(&start), INITIAL_FEN);
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
