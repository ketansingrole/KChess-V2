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

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(source: HistoryKind, moves: &[&str], result: &str, reason: &str) -> Snapshot {
        Snapshot {
            source,
            setup: Setup::standard(),
            moves: moves.iter().map(|m| m.to_string()).collect(),
            white: "You".into(),
            black: "Stockfish".into(),
            result: result.into(),
            reason: reason.into(),
            time_control: "-".into(),
            clock_summary: None,
        }
    }

    /// The entry history holds for `latest`'s game, saved earlier under another name.
    fn stored(latest: &Snapshot, moves: &[&str], result: &str, reason: &str) -> PlayedGame {
        PlayedGame {
            id: "game".into(),
            source: latest.source,
            started_at: 1.,
            updated_at: 2.,
            white: "Earlier".into(),
            black: latest.black.clone(),
            result: result.into(),
            reason: reason.into(),
            finished: false,
            setup: latest.setup.clone(),
            moves: moves.iter().map(|m| m.to_string()).collect(),
            time_control: latest.time_control.clone(),
            clock_summary: None,
        }
    }

    /// What a reset saves of the game `latest` when history holds `previous`.
    fn reset(previous: PlayedGame, latest: &Snapshot) -> PlayedGame {
        let games = [previous];
        let host = Host {
            has_play: true,
            source: Some(latest),
            games: &games,
            now: 10.,
            new_id: "next".into(),
        };
        let mut state = State {
            identity: Identity {
                id: "game".into(),
                started_at: 1.,
            },
            had_play: true,
        };
        let mut effects = state.reset(&host);
        assert_eq!(state.identity.id, "next");
        match effects.pop() {
            Some(Effect::Save(game)) if effects.is_empty() => *game,
            other => panic!("{other:?} {effects:?}"),
        }
    }

    /// A reset closes the entry of the game as it stands: an entry with the same moves keeps its
    /// own details and takes the result the game reached since, or keeps the result it had; an
    /// unfinished game is "stopped" except on the clock, which has no result of its own.
    #[test]
    fn a_reset_closes_the_entry_with_the_games_result() {
        let moves = ["e2e4", "e7e5"];
        let cases = [
            // (source, stored result and reason, the game's, what is saved)
            (
                HistoryKind::Computer,
                ("*", "In progress"),
                ("1-0", "Checkmate"),
                ("1-0", "Checkmate"),
            ),
            (
                HistoryKind::Computer,
                ("0-1", "You resigned"),
                ("*", "In progress"),
                ("0-1", "You resigned"),
            ),
            (
                HistoryKind::Computer,
                ("*", "In progress"),
                ("*", "In progress"),
                ("*", STOPPED),
            ),
            (
                HistoryKind::Clock,
                ("*", "Clock"),
                ("*", "Clock stopped"),
                ("*", "Clock stopped"),
            ),
        ];
        for (source, (was, was_why), (now, now_why), (result, reason)) in cases {
            let latest = snapshot(source, &moves, now, now_why);
            let saved = reset(stored(&latest, &moves, was, was_why), &latest);
            let expected = PlayedGame {
                result: result.into(),
                reason: reason.into(),
                finished: true,
                ..stored(&latest, &moves, was, was_why)
            };
            assert_eq!(saved, expected, "{source:?} {was} {now}");
        }
    }

    /// An entry whose moves differ from the game's is saved afresh from the game.
    #[test]
    fn a_reset_saves_a_changed_game_from_the_game() {
        let latest = snapshot(HistoryKind::Computer, &["d2d4"], "*", "In progress");
        let saved = reset(stored(&latest, &["e2e4"], "*", "In progress"), &latest);
        assert_eq!(saved.moves, ["d2d4"]);
        assert_eq!(saved.white, "You");
        assert_eq!(saved.reason, STOPPED);
        assert_eq!(saved.updated_at, 10.);
        assert!(saved.finished);
    }
}
