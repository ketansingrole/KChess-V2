//! The game sessions as the recorded suites drove them: a transition took the state (and its
//! runtime) as JSON with its input and the host reads supplied so far, and answered either the
//! finished transition (`{done: {value, state, runtime, written, effects}}`) or the next read it
//! lacked (`{read: kind}`), the host asking again with one more read. This adapter runs the typed
//! sessions over that protocol: the reads are taken lazily, in order, and the first one missing
//! is the answer; the fields each transition wrote are worked out from what it did.

use std::collections::HashMap;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use kchess_domain::games::archive::{self, Identity};
use kchess_domain::games::computer::{self, ComputerGame, Effect};
use kchess_domain::games::local::{self, LocalClock, LocalGame};
use kchess_domain::games::{self, Clock, Setup};

use super::Out;

/// Why the reads could not answer a transition: a read missing, or malformed.
enum Problem {
    Missing(&'static str),
    Malformed(&'static str),
}

/// The host reads supplied so far, taken in order; the first problem is kept.
struct Reads<'a> {
    values: &'a Map<String, Value>,
    taken: HashMap<&'static str, usize>,
    problem: Option<Problem>,
    /// What `stored` and `source` answered, in order.
    stored: Vec<Option<Value>>,
    sources: Vec<Value>,
    has_play: Vec<bool>,
}

impl<'a> Reads<'a> {
    fn take(&mut self, kind: &'static str) -> Option<Value> {
        let index = self.taken.entry(kind).or_insert(0);
        let value = self
            .values
            .get(kind)
            .and_then(Value::as_array)
            .and_then(|list| list.get(*index))
            .cloned();
        *index += 1;
        if value.is_none() && self.problem.is_none() {
            self.problem = Some(Problem::Missing(kind));
        }
        value
    }

    fn typed<T: Default>(
        &mut self,
        kind: &'static str,
        read: impl FnOnce(&Value) -> Option<T>,
    ) -> T {
        let Some(value) = self.take(kind) else {
            return T::default();
        };
        read(&value).unwrap_or_else(|| {
            if self.problem.is_none() {
                self.problem = Some(Problem::Malformed(kind));
            }
            T::default()
        })
    }

    fn now(&mut self) -> f64 {
        self.typed("now", Value::as_f64)
    }

    fn now_or(&mut self, at: Option<f64>) -> f64 {
        at.unwrap_or_else(|| self.now())
    }
}

impl games::Host for Reads<'_> {
    fn now(&mut self) -> f64 {
        Reads::now(self)
    }
    fn allowed(&mut self) -> bool {
        self.typed("allowed", Value::as_bool)
    }
    fn ready(&mut self) -> bool {
        self.typed("ready", Value::as_bool)
    }
}

impl archive::Host for Reads<'_> {
    fn has_play(&mut self) -> bool {
        let has = self.typed("hasPlay", Value::as_bool);
        self.has_play.push(has);
        has
    }
    fn source(&mut self) -> Value {
        let source = self.take("source").unwrap_or(Value::Null);
        self.sources.push(source.clone());
        source
    }
    fn stored(&mut self, id: &str) -> Option<Value> {
        let games: Vec<Value> = self.typed("games", |v| v.as_array().cloned());
        let entry = games
            .into_iter()
            .find(|game| game.get("id").and_then(Value::as_str) == Some(id));
        self.stored.push(entry.clone());
        entry
    }
    fn now(&mut self) -> f64 {
        Reads::now(self)
    }
    fn new_id(&mut self) -> String {
        self.typed("id", |v| v.as_str().map(str::to_string))
    }
}

/// A finished transition, as the protocol wrote it.
#[derive(Default)]
struct Done {
    value: Value,
    state: Option<Value>,
    runtime: Option<Value>,
    written: Vec<&'static str>,
    effects: Vec<Value>,
}

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Run one transition over the protocol: `args` are its inputs followed by the reads.
fn transition(
    args: &[Value],
    body: impl FnOnce(&[Value], &mut Reads) -> Result<Done, String>,
) -> Out {
    let (reads, inputs) = args.split_last().ok_or("the reads are missing")?;
    let values = reads.as_object().ok_or("the reads must be an object")?;
    let mut reads = Reads {
        values,
        taken: HashMap::new(),
        problem: None,
        stored: Vec::new(),
        sources: Vec::new(),
        has_play: Vec::new(),
    };
    let done = body(inputs, &mut reads);
    match reads.problem {
        Some(Problem::Missing(kind)) => return Ok(json!({ "read": kind })),
        Some(Problem::Malformed(kind)) => {
            return Err(format!("the host's {kind} read is malformed"));
        }
        None => {}
    }
    let done = done?;
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

/// Argument `i` as a typed value; a missing or null argument is an error.
fn arg<T: DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T, String> {
    let value = args
        .get(i)
        .filter(|value| !value.is_null())
        .ok_or_else(|| format!("{name} is missing"))?;
    serde_json::from_value(value.clone()).map_err(|error| format!("{name}: {error}"))
}

/// Argument `i` as a typed value when given (null is absent).
fn optional_arg<T: DeserializeOwned>(args: &[Value], i: usize) -> Result<Option<T>, String> {
    match args.get(i).filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| error.to_string()),
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "computerState" => optional_arg(args, 0)
            .map(|saved| to_value(&computer::State::from_saved(saved.unwrap_or_default()))),
        "computerInit" => transition(args, |_, reads| {
            Ok(Done {
                runtime: Some(to_value(&computer::Runtime::start(reads))),
                ..Done::default()
            })
        }),
        "computerView" => transition(args, computer_view),
        "computerAvailability" => computer_change(args, |game, _, reads| {
            let had_times = game.state().times.is_some();
            let effects = game.availability_changed(reads);
            let mut written = Vec::new();
            if !effects.is_empty() && had_times {
                written.push("times");
            }
            if !effects.is_empty() {
                written.extend(["epoch", "thinking"]);
            }
            Ok((Value::Null, effects, written))
        }),
        "computerExpire" => computer_step(args, |game, inputs, reads| {
            let at = reads.now_or(inputs.get(2).and_then(Value::as_f64));
            let effects = game.expire(at);
            Ok((json!(!effects.is_empty()), effects))
        }),
        "computerMove" => computer_step(args, |game, inputs, reads| {
            let uci: String = arg(inputs, 2, "uci")?;
            let (played, effects) = game.play(reads, &uci);
            Ok((json!(played), effects))
        }),
        "computerSearchStart" => computer_change(args, |game, _, reads| {
            let request = game.search_start(reads)?;
            let written = if request.is_some() {
                vec!["thinking"]
            } else {
                vec![]
            };
            Ok((to_value(&request), Vec::new(), written))
        }),
        "computerSearchReply" => computer_step(args, |game, inputs, reads| {
            let reply: Reply = arg(inputs, 2, "reply")?;
            let effects = game.search_reply(reads, reply.epoch, reply.count, &reply.reply);
            Ok((Value::Null, effects))
        }),
        "computerSearchFailed" => computer_change(args, |game, inputs, _| {
            let epoch = epoch_arg(inputs)?;
            Ok((Value::Null, vec![game.search_failed(epoch)], Vec::new()))
        }),
        "computerSearchEnd" => computer_change(args, |game, inputs, _| {
            let epoch = epoch_arg(inputs)?;
            let written = if epoch == game.state().epoch {
                vec!["thinking"]
            } else {
                vec![]
            };
            game.search_end(epoch);
            Ok((Value::Null, Vec::new(), written))
        }),
        "computerStart" => computer_change(args, |game, inputs, reads| {
            let setup: Setup = arg(inputs, 2, "setup")?;
            let clock: Option<Clock> = optional_arg(inputs, 3)?;
            let effects = game.start(reads, setup, clock)?;
            let fields = [
                "setup", "clock", "moves", "ply", "resigned", "flagged", "times",
            ];
            let written = after_cancel(&effects, &fields);
            Ok((Value::Null, effects, written))
        }),
        "computerTakeback" => computer_change(args, |game, _, reads| {
            let effects = game.takeback(reads);
            let written = after_cancel(&effects, &["resigned", "flagged", "moves", "ply"]);
            Ok((Value::Null, effects, written))
        }),
        "computerResign" => computer_change(args, |game, _, _| {
            let effects = game.resign();
            let written = after_cancel(&effects, &["resigned", "ply"]);
            Ok((Value::Null, effects, written))
        }),
        "localState" => optional_arg(args, 0)
            .map(|saved| to_value(&local::State::from_saved(saved.unwrap_or_default()))),
        "localInit" => transition(args, |_, reads| {
            let runtime = local::Runtime {
                turn_started: reads.now(),
            };
            Ok(Done {
                runtime: Some(to_value(&runtime)),
                ..Done::default()
            })
        }),
        "localView" => transition(args, local_view),
        "localExpire" => local_step(args, |game, inputs, reads| {
            let at = reads.now_or(inputs.get(2).and_then(Value::as_f64));
            let flagged = game.expire(at);
            Ok((json!(flagged), flagged))
        }),
        "localMove" => local_step(args, |game, inputs, reads| {
            let uci: String = arg(inputs, 2, "uci")?;
            let (san, flagged) = game.play(&uci, || reads.now());
            Ok((json!(san), flagged))
        }),
        "localTakeback" => local_step(args, |game, _, reads| {
            game.takeback(|| reads.now());
            Ok((Value::Null, false))
        }),
        "localStart" => local_change(args, Method::Start, |game, inputs, reads| {
            let setup: Setup = arg(inputs, 2, "setup")?;
            let clock: Option<LocalClock> = optional_arg(inputs, 3)?;
            game.start(setup, clock, || reads.now())?;
            Ok((Value::Null, false))
        }),
        "localResign" => local_step(args, |game, inputs, _| {
            let color: String = arg(inputs, 2, "color")?;
            game.resign(&color);
            Ok((Value::Null, false))
        }),
        "localAgreeDraw" => local_step(args, |game, _, _| {
            game.agree_draw();
            Ok((Value::Null, false))
        }),
        "localTogglePause" => local_step(args, |game, _, reads| {
            let at = reads.now();
            Ok((Value::Null, game.toggle_pause(at)))
        }),
        "archiveState" => transition(args, |inputs, reads| {
            let identity: Option<Identity> = optional_arg(inputs, 0)?;
            Ok(Done {
                value: to_value(&archive::State::new(identity, reads)),
                ..Done::default()
            })
        }),
        "archiveInit" => archive_step(args, |archive, inputs, reads| {
            let resume: bool = arg(inputs, 1, "resume")?;
            let effects = archive.init(reads, resume);
            let written = if resume { vec![] } else { vec!["identity"] };
            Ok((effects, written))
        }),
        "archiveSave" => archive_step(args, |archive, inputs, reads| {
            let finished: bool = arg(inputs, 1, "finished")?;
            let had_play = archive.had_play;
            let effects = archive.save(reads, finished);
            Ok((effects, save_written(had_play, reads, 0)))
        }),
        "archiveReset" => archive_step(args, |archive, _, reads| {
            let had_play = archive.had_play;
            let effects = archive.reset(reads);
            let previous = reads.stored.first().cloned().flatten();
            let latest = reads.sources.first().cloned().unwrap_or(Value::Null);
            let kept =
                previous.is_some_and(|previous| previous.get("moves") == latest.get("moves"));
            let mut written = if kept {
                Vec::new()
            } else {
                save_written(had_play, reads, 1)
            };
            written.extend(["hadPlay", "identity"]);
            Ok((effects, written))
        }),
        _ => return None,
    })
}

#[derive(serde::Deserialize)]
struct Epoch {
    epoch: u64,
}

#[derive(serde::Deserialize)]
struct Reply {
    epoch: u64,
    count: usize,
    #[serde(rename = "move")]
    reply: String,
}

fn epoch_arg(inputs: &[Value]) -> Result<u64, String> {
    Ok(arg::<Epoch>(inputs, 2, "epoch")?.epoch)
}

fn computer_load(inputs: &[Value]) -> Result<ComputerGame, String> {
    let state: computer::State = arg(inputs, 0, "state")?;
    let runtime: computer::Runtime = arg(inputs, 1, "runtime")?;
    ComputerGame::new(state, runtime)
}

/// The fields a move, a flag (with the cancel it makes) or a cancel assigned, in order.
fn written_by(before: &computer::State, effects: &[Effect]) -> Vec<&'static str> {
    let mut written = Vec::new();
    let mut i = 0;
    while i < effects.len() {
        match (&effects[i], effects.get(i + 1)) {
            (Effect::StopEngine, Some(Effect::Flagged)) => {
                written.extend(["times", "flagged", "epoch", "thinking"]);
                i += 1;
            }
            (Effect::StopEngine, _) => written.extend(["epoch", "thinking"]),
            (Effect::Moved { .. }, _) => {
                if before.times.is_some() {
                    written.push("times");
                }
                written.extend(["moves", "ply"]);
            }
            _ => {}
        }
        i += 1;
    }
    written
}

/// What a computer transition hands back: its value, effects and the fields it wrote.
type Stepped = (Value, Vec<Effect>, Vec<&'static str>);

/// A transition that changes the game, its written fields worked out from its effects.
fn computer_step(
    args: &[Value],
    body: impl FnOnce(&mut ComputerGame, &[Value], &mut Reads) -> Result<(Value, Vec<Effect>), String>,
) -> Out {
    computer_change(args, |game, inputs, reads| {
        let before = game.state().clone();
        let (value, effects) = body(game, inputs, reads)?;
        let written = written_by(&before, &effects);
        Ok((value, effects, written))
    })
}

fn computer_change(
    args: &[Value],
    body: impl FnOnce(&mut ComputerGame, &[Value], &mut Reads) -> Result<Stepped, String>,
) -> Out {
    transition(args, |inputs, reads| {
        let mut game = computer_load(inputs)?;
        let (value, effects, written) = body(&mut game, inputs, reads)?;
        Ok(Done {
            value,
            state: Some(to_value(game.state())),
            runtime: Some(to_value(game.runtime())),
            written,
            effects: effects.iter().map(to_value).collect(),
        })
    })
}

/// The fields written by a cancel and what follows it, when it happened.
fn after_cancel(effects: &[Effect], fields: &[&'static str]) -> Vec<&'static str> {
    if effects.is_empty() {
        return Vec::new();
    }
    let mut written = vec!["epoch", "thinking"];
    written.extend_from_slice(fields);
    written
}

fn computer_view(inputs: &[Value], reads: &mut Reads) -> Result<Done, String> {
    let game = computer_load(inputs)?;
    let query: Value = arg(inputs, 2, "query")?;
    let value = match query
        .get("view")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "over" => json!(game.over()),
        "winner" => json!(game.winner()),
        "result" => to_value(&game.result()),
        "running" => json!(game.running()),
        "draw" => json!(game.draw()),
        "remaining" => {
            let color = query
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("white");
            let at = reads.now_or(query.get("at").and_then(Value::as_f64));
            json!(game.remaining(color, at))
        }
        "snapshot" => {
            let mut saved = to_value(&game.snapshot(0.0));
            if game.state().times.is_some() {
                let white = game.remaining("white", reads.now());
                let black = game.remaining("black", reads.now());
                saved["times"] = json!({ "white": white, "black": black });
            }
            saved
        }
        "archiveSnapshot" => to_value(&game.archive_snapshot()?),
        other => return Err(format!("unknown computer view {other}")),
    };
    Ok(Done {
        value,
        ..Done::default()
    })
}

fn local_load(inputs: &[Value]) -> Result<LocalGame, String> {
    let state: local::State = arg(inputs, 0, "state")?;
    let runtime: local::Runtime = arg(inputs, 1, "runtime")?;
    LocalGame::new(state, runtime)
}

/// The fields a board transition assigned, from what it changed.
fn local_written(
    method: Method,
    before: &local::State,
    after: &local::State,
    flagged: bool,
) -> Vec<&'static str> {
    if flagged {
        return vec!["times", "result"];
    }
    match method {
        Method::Start => vec!["setup", "clock", "moves", "result", "paused", "times"],
        _ if after.moves.len() > before.moves.len() => {
            let mut written = if before.times.is_some() {
                vec!["times"]
            } else {
                vec![]
            };
            written.extend(["paused", "moves"]);
            written
        }
        _ if after.moves.len() < before.moves.len() => vec!["moves", "result"],
        _ if before.paused != after.paused => {
            let mut written = if before.paused { vec![] } else { vec!["times"] };
            written.push("paused");
            written
        }
        _ if before.result.is_none() && after.result.is_some() => vec!["result"],
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Start,
    Other,
}

fn local_step(
    args: &[Value],
    body: impl FnOnce(&mut LocalGame, &[Value], &mut Reads) -> Result<(Value, bool), String>,
) -> Out {
    local_change(args, Method::Other, body)
}

fn local_change(
    args: &[Value],
    method: Method,
    body: impl FnOnce(&mut LocalGame, &[Value], &mut Reads) -> Result<(Value, bool), String>,
) -> Out {
    transition(args, |inputs, reads| {
        let mut game = local_load(inputs)?;
        let before = game.state().clone();
        let (value, flagged) = body(&mut game, inputs, reads)?;
        Ok(Done {
            value,
            state: Some(to_value(game.state())),
            runtime: Some(to_value(game.runtime())),
            written: local_written(method, &before, game.state(), flagged),
            effects: if flagged {
                vec![json!({ "type": "flagged" })]
            } else {
                Vec::new()
            },
        })
    })
}

fn local_view(inputs: &[Value], reads: &mut Reads) -> Result<Done, String> {
    let game = local_load(inputs)?;
    let query: Value = arg(inputs, 2, "query")?;
    let value = match query
        .get("view")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "over" => json!(game.over()),
        "result" => to_value(&game.result()),
        "running" => json!(game.running()),
        "remaining" => {
            let color = query
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("white");
            let at = reads.now_or(query.get("at").and_then(Value::as_f64));
            json!(game.remaining(color, at))
        }
        "snapshot" => {
            let mut saved = to_value(&game.snapshot(0.0));
            if game.state().times.is_some() {
                let white = game.remaining("white", reads.now());
                let black = game.remaining("black", reads.now());
                saved["times"] = json!({ "white": white, "black": black });
            }
            saved
        }
        "archiveSnapshot" => to_value(&game.archive_snapshot()),
        other => return Err(format!("unknown local game view {other}")),
    };
    Ok(Done {
        value,
        ..Done::default()
    })
}

/// What `save` wrote: `hadPlay`, after the identity when the entry started with this move.
/// `stored` is the index of its `stored` answer among the transition's.
fn save_written(had_play: bool, reads: &Reads, stored: usize) -> Vec<&'static str> {
    let has_play = reads.has_play.first().copied().unwrap_or(false);
    if has_play && !had_play && reads.stored.get(stored).is_some_and(Option::is_none) {
        vec!["identity", "hadPlay"]
    } else {
        vec!["hadPlay"]
    }
}

fn archive_step(
    args: &[Value],
    body: impl FnOnce(
        &mut archive::State,
        &[Value],
        &mut Reads,
    ) -> Result<(Vec<archive::Effect>, Vec<&'static str>), String>,
) -> Out {
    transition(args, |inputs, reads| {
        let mut state: archive::State = arg(inputs, 0, "state")?;
        let (effects, written) = body(&mut state, inputs, reads)?;
        let effects = effects
            .into_iter()
            .map(|effect| match effect {
                archive::Effect::Save(game) => json!({ "type": "save", "game": game }),
                archive::Effect::Remove(id) => json!({ "type": "remove", "id": id }),
            })
            .collect();
        Ok(Done {
            state: Some(to_value(&state)),
            written,
            effects,
            ..Done::default()
        })
    })
}
