//! `GameArchive` (`core/src/domain/gameArchive.ts`): one stable identity per game session, and
//! how the session's game is saved while it is played, when it is reset, and when it is stopped.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Done, Flow, Reads, arg, optional_arg, to_value, transition};
use crate::js;

/// The archive entry an unfinished session keeps updating (`ArchiveIdentity`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: String,
    pub started_at: f64,
}

/// `ArchiveState`: the identity and whether the session has had any play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub identity: Identity,
    pub had_play: bool,
}

/// One archive transition: its state and the host effects it produced.
struct Archive {
    state: State,
    fx: Vec<Value>,
    written: Vec<&'static str>,
}

/// `value` is the string `text`.
fn is(value: Option<&Value>, text: &str) -> bool {
    value.and_then(Value::as_str) == Some(text)
}

/// The archived game entry with this identity, if any.
fn entry_of<'a>(games: &'a [Value], id: &str) -> Option<&'a Value> {
    games
        .iter()
        .find(|game| game.get("id").and_then(Value::as_str) == Some(id))
}

/// A stored game with some fields replaced: `{ ...game, ...fields }`.
fn with_fields(game: &Value, fields: Vec<(&str, Option<Value>)>) -> Map<String, Value> {
    let mut out = game.as_object().cloned().unwrap_or_default();
    for (key, value) in fields {
        if let Some(value) = value {
            out.insert(key.to_string(), value);
        }
    }
    out
}

impl Archive {
    fn load(args: &[Value]) -> Result<Self, String> {
        Ok(Self {
            state: arg(args, 0, "state")?,
            fx: Vec::new(),
            written: Vec::new(),
        })
    }

    fn done(self) -> Flow<Done> {
        Ok(Done {
            value: Value::Null,
            state: Some(to_value(&self.state)?),
            runtime: None,
            written: self.written,
            effects: self.fx,
        })
    }

    /// `newIdentity()`: a new entry, with the host's id and clock.
    fn new_identity(&mut self, reads: &mut Reads) -> Flow<()> {
        let id = reads.id()?;
        let started_at = reads.now()?;
        self.state.identity = Identity { id, started_at };
        self.written.push("identity");
        Ok(())
    }

    /// `save(finished)`: store the game as it stands, or remove it when there is nothing to keep.
    fn save(&mut self, reads: &mut Reads, finished: bool) -> Flow<()> {
        if !reads.has_play()? {
            if self.state.had_play {
                self.fx
                    .push(json!({ "type": "remove", "id": self.state.identity.id }));
            }
            self.state.had_play = false;
            self.written.push("hadPlay");
            return Ok(());
        }
        self.state.had_play = true;
        self.written.push("hadPlay");
        let snapshot = reads.source()?;
        let open = is(snapshot.get("result"), "*");
        let from_clock = is(snapshot.get("source"), "clock");
        let reason = if finished && open && !from_clock {
            Some(json!("Stopped before the game ended"))
        } else {
            snapshot.get("reason").cloned()
        };
        let updated_at = reads.now()?;
        let identity = self.state.identity.clone();
        let game = with_fields(
            &snapshot,
            vec![
                ("id", Some(json!(identity.id))),
                ("startedAt", Some(json!(identity.started_at))),
                ("reason", reason),
                ("updatedAt", Some(json!(updated_at))),
                ("finished", Some(json!(finished || !open))),
            ],
        );
        self.fx
            .push(json!({ "type": "save", "game": Value::Object(game) }));
        Ok(())
    }

    /// `reset()`: keep the game as far as it went, then start a new entry.
    fn reset(&mut self, reads: &mut Reads) -> Flow<()> {
        let games = reads.games()?;
        let previous = entry_of(&games, &self.state.identity.id);
        let latest = reads.source()?;
        match previous {
            Some(previous) if previous.get("moves") == latest.get("moves") => {
                let result = if is(previous.get("result"), "*") {
                    latest.get("result").cloned()
                } else {
                    previous.get("result").cloned()
                };
                let stopped = is(result.as_ref(), "*") && !is(latest.get("source"), "clock");
                let reason = if stopped {
                    Some(json!("Stopped before the game ended"))
                } else if is(previous.get("result"), "*") {
                    latest.get("reason").cloned()
                } else {
                    previous.get("reason").cloned()
                };
                let game = with_fields(
                    previous,
                    vec![
                        ("result", result),
                        ("finished", Some(json!(true))),
                        ("reason", reason),
                    ],
                );
                self.fx
                    .push(json!({ "type": "save", "game": Value::Object(game) }));
            }
            _ => self.save(reads, true)?,
        }
        self.state.had_play = false;
        self.written.push("hadPlay");
        self.new_identity(reads)
    }

    /// The constructor's `resume = false`: close the entry being replaced, then start afresh.
    fn init(&mut self, reads: &mut Reads, resume: bool) -> Flow<()> {
        if resume {
            return Ok(());
        }
        let games = reads.games()?;
        let open = entry_of(&games, &self.state.identity.id)
            .filter(|previous| js::falsy(previous.get("finished")));
        if let Some(previous) = open {
            let game = with_fields(previous, vec![("finished", Some(json!(true)))]);
            self.fx
                .push(json!({ "type": "save", "game": Value::Object(game) }));
        }
        self.new_identity(reads)
    }
}

/// `archiveState(identity?, host)`: the identity is kept, or a new one is made from the host.
pub fn state(args: &[Value]) -> Result<Value, String> {
    transition(args, |inputs, reads| {
        let identity: Option<Identity> = optional_arg(inputs, 0)?;
        let identity = match identity {
            Some(identity) => identity,
            None => {
                let id = reads.id()?;
                let started_at = reads.now()?;
                Identity { id, started_at }
            }
        };
        Ok(Done {
            value: to_value(&State {
                identity,
                had_play: false,
            })?,
            ..Done::default()
        })
    })
}

/// `new GameArchive(state, host, resume)`.
pub fn init(args: &[Value]) -> Result<Value, String> {
    transition(args, |inputs, reads| {
        let mut archive = Archive::load(inputs)?;
        let resume: bool = arg(inputs, 1, "resume")?;
        archive.init(reads, resume)?;
        archive.done()
    })
}

/// `save(finished)`.
pub fn save(args: &[Value]) -> Result<Value, String> {
    transition(args, |inputs, reads| {
        let mut archive = Archive::load(inputs)?;
        let finished: bool = arg(inputs, 1, "finished")?;
        archive.save(reads, finished)?;
        archive.done()
    })
}

/// `reset()`.
pub fn reset(args: &[Value]) -> Result<Value, String> {
    transition(args, |inputs, reads| {
        let mut archive = Archive::load(inputs)?;
        archive.reset(reads)?;
        archive.done()
    })
}
