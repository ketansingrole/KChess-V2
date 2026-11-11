//! One stable identity per game session, and how the session's game is saved to Game history
//! while it is played, when it is reset, and when it is stopped.

use serde::{Deserialize, Serialize};

use crate::library::PlayedGame;
use crate::misc::constants::HistoryKind;
use crate::variant::Setup;

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

/// A session's game as Game history describes it, before it has an entry of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub source: HistoryKind,
    pub setup: Setup,
    pub moves: Vec<String>,
    pub white: String,
    pub black: String,
    pub result: String,
    pub reason: String,
    pub time_control: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_summary: Option<String>,
}

/// What the frontend tells the archive.
pub struct Host<'a> {
    /// The game has moves.
    pub has_play: bool,
    /// The game as it stands; none before the frontend can describe it.
    pub source: Option<&'a Snapshot>,
    /// The games already in history.
    pub games: &'a [PlayedGame],
    /// Milliseconds since the epoch.
    pub now: f64,
    /// A new unique id, for the entry a new game gets.
    pub new_id: String,
}

impl Host<'_> {
    /// The game already in history with this id, if any.
    pub fn stored(&self, id: &str) -> Option<&PlayedGame> {
        self.games.iter().find(|game| game.id == id)
    }
}

/// What the archive asks its caller to write to history.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Save(Box<PlayedGame>),
    Remove(String),
}

const STOPPED: &str = "Stopped before the game ended";

/// A new identity, with the host's id and clock.
fn new_identity(host: &Host) -> Identity {
    Identity {
        id: host.new_id.clone(),
        started_at: host.now,
    }
}

impl State {
    /// The identity kept, or a new one made by the host.
    pub fn new(identity: Option<Identity>, host: &Host) -> State {
        State {
            identity: identity.unwrap_or_else(|| new_identity(host)),
            had_play: false,
        }
    }

    /// Store the game as it stands, or remove it when there is nothing to keep.
    pub fn save(&mut self, finished: bool, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.save_into(host, finished, &mut fx);
        fx
    }

    fn save_into(&mut self, host: &Host, finished: bool, fx: &mut Vec<Effect>) {
        if !host.has_play {
            if self.had_play {
                fx.push(Effect::Remove(self.identity.id.clone()));
            }
            self.had_play = false;
            return;
        }
        let Some(snapshot) = host.source else {
            return;
        };
        // The entry starts with its first move, not when the previous game was replaced: a
        // session can wait days for that move, and History orders games by `startedAt`.
        if !self.had_play && host.stored(&self.identity.id).is_none() {
            self.identity.started_at = host.now;
        }
        self.had_play = true;
        let open = snapshot.result == "*";
        let reason = if finished && open && snapshot.source != HistoryKind::Clock {
            STOPPED.to_string()
        } else {
            snapshot.reason.clone()
        };
        fx.push(Effect::Save(Box::new(PlayedGame {
            id: self.identity.id.clone(),
            source: snapshot.source,
            started_at: self.identity.started_at,
            updated_at: host.now,
            white: snapshot.white.clone(),
            black: snapshot.black.clone(),
            result: snapshot.result.clone(),
            reason,
            finished: finished || !open,
            setup: snapshot.setup.clone(),
            moves: snapshot.moves.clone(),
            time_control: snapshot.time_control.clone(),
            clock_summary: snapshot.clock_summary.clone(),
        })));
    }

    /// Keep the game as far as it went, then start a new entry.
    pub fn reset(&mut self, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        let previous = host.stored(&self.identity.id);
        match (previous, host.source) {
            (Some(previous), Some(latest)) if previous.moves == latest.moves => {
                let open = previous.result == "*";
                let result = if open {
                    latest.result.clone()
                } else {
                    previous.result.clone()
                };
                let reason = if result == "*" && latest.source != HistoryKind::Clock {
                    STOPPED.to_string()
                } else if open {
                    latest.reason.clone()
                } else {
                    previous.reason.clone()
                };
                fx.push(Effect::Save(Box::new(PlayedGame {
                    result,
                    finished: true,
                    reason,
                    ..previous.clone()
                })));
            }
            _ => self.save_into(host, true, &mut fx),
        }
        self.had_play = false;
        self.identity = new_identity(host);
        fx
    }

    /// Close the entry being replaced and start afresh, unless the game is `resume`d.
    pub fn init(&mut self, resume: bool, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        if resume {
            // A resumed game was saved before, so taking back its only move removes that entry.
            if host.stored(&self.identity.id).is_some() {
                self.had_play = true;
            }
            return fx;
        }
        if let Some(previous) = host
            .stored(&self.identity.id)
            .filter(|previous| !previous.finished)
        {
            fx.push(Effect::Save(Box::new(PlayedGame {
                finished: true,
                ..previous.clone()
            })));
        }
        self.identity = new_identity(host);
        fx
    }
}
