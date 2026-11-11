//! One stable identity per game session, and how the session's game is saved to Game history
//! while it is played, when it is reset, and when it is stopped.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::js;

/// The archive entry an unfinished session keeps updating.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: String,
    pub started_at: f64,
}

/// The identity and whether the session has had any play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub identity: Identity,
    pub had_play: bool,
}

/// What the archive asks its host at the moment it needs it.
pub trait Host {
    /// The game has moves.
    fn has_play(&mut self) -> bool;
    /// The game as its archive snapshot describes it.
    fn source(&mut self) -> Value;
    /// The game already in history with this id, if any.
    fn stored(&mut self, id: &str) -> Option<Value>;
    /// Milliseconds since the epoch.
    fn now(&mut self) -> f64;
    /// A new unique id.
    fn new_id(&mut self) -> String;
}

/// What the archive asks its caller to write to history.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Save(Value),
    Remove(String),
}

/// `value` is the string `text`.
fn is(value: Option<&Value>, text: &str) -> bool {
    value.and_then(Value::as_str) == Some(text)
}

/// A stored game with some fields replaced: `{ ...game, ...fields }`.
fn with_fields(game: &Value, fields: Vec<(&str, Option<Value>)>) -> Value {
    let mut out: Map<String, Value> = game.as_object().cloned().unwrap_or_default();
    for (key, value) in fields {
        if let Some(value) = value {
            out.insert(key.to_string(), value);
        }
    }
    Value::Object(out)
}

/// A new identity, with the host's id and clock.
fn new_identity(host: &mut dyn Host) -> Identity {
    let id = host.new_id();
    let started_at = host.now();
    Identity { id, started_at }
}

impl State {
    /// The identity kept, or a new one made by the host.
    pub fn new(identity: Option<Identity>, host: &mut dyn Host) -> State {
        State {
            identity: identity.unwrap_or_else(|| new_identity(host)),
            had_play: false,
        }
    }

    /// Store the game as it stands, or remove it when there is nothing to keep.
    pub fn save(&mut self, host: &mut dyn Host, finished: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.save_into(host, finished, &mut fx);
        fx
    }

    fn save_into(&mut self, host: &mut dyn Host, finished: bool, fx: &mut Vec<Effect>) {
        if !host.has_play() {
            if self.had_play {
                fx.push(Effect::Remove(self.identity.id.clone()));
            }
            self.had_play = false;
            return;
        }
        // The entry starts with its first move, not when the previous game was replaced: a
        // session can wait days for that move, and History orders games by `startedAt`.
        if !self.had_play && host.stored(&self.identity.id).is_none() {
            self.identity.started_at = host.now();
        }
        self.had_play = true;
        let snapshot = host.source();
        let open = is(snapshot.get("result"), "*");
        let from_clock = is(snapshot.get("source"), "clock");
        let reason = if finished && open && !from_clock {
            Some(json!("Stopped before the game ended"))
        } else {
            snapshot.get("reason").cloned()
        };
        let updated_at = host.now();
        let game = with_fields(
            &snapshot,
            vec![
                ("id", Some(json!(self.identity.id))),
                ("startedAt", Some(json!(self.identity.started_at))),
                ("reason", reason),
                ("updatedAt", Some(json!(updated_at))),
                ("finished", Some(json!(finished || !open))),
            ],
        );
        fx.push(Effect::Save(game));
    }

    /// Keep the game as far as it went, then start a new entry.
    pub fn reset(&mut self, host: &mut dyn Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        let previous = host.stored(&self.identity.id);
        let latest = host.source();
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
                fx.push(Effect::Save(with_fields(
                    &previous,
                    vec![
                        ("result", result),
                        ("finished", Some(json!(true))),
                        ("reason", reason),
                    ],
                )));
            }
            _ => self.save_into(host, true, &mut fx),
        }
        self.had_play = false;
        self.identity = new_identity(host);
        fx
    }

    /// Close the entry being replaced and start afresh, unless the game is `resume`d.
    pub fn init(&mut self, host: &mut dyn Host, resume: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        if resume {
            return fx;
        }
        let open = host
            .stored(&self.identity.id)
            .filter(|previous| js::falsy(previous.get("finished")));
        if let Some(previous) = open {
            fx.push(Effect::Save(with_fields(
                &previous,
                vec![("finished", Some(json!(true)))],
            )));
        }
        self.identity = new_identity(host);
        fx
    }
}
