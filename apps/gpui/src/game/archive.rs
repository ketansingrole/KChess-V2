//! `GameArchive` (`crates/kchess-wasm/js/gameArchive.ts`): a played game's place in Game history.
//! The rules decide when the game is saved, replaced or dropped
//! (`crates/kchess-domain/src/games/archive.rs`); the page answers the reads and writes what they
//! say to the profile.

use serde_json::{Value, json};

use super::{assign_written, transition};

/// What the page tells the archive when a transition reads it.
pub struct Host<'a> {
    /// The game as `archiveSnapshot` describes it.
    pub source: Value,
    /// The game has moves (`localMoves.length > 0`).
    pub has_play: bool,
    /// The games already in history.
    pub games: &'a [Value],
    /// Milliseconds since the epoch (`Date.now()`).
    pub now: f64,
}

/// What an archive transition asks the page to write.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Save(Value),
    Remove(String),
}

pub struct GameArchive {
    /// `{ identity, hadPlay }`; the identity is saved as the `archive:<kind>` session.
    pub state: Value,
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // A clock-based id is still unique enough for one profile's history.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn read(host: &Host<'_>, kind: &str) -> Result<Value, String> {
    match kind {
        "hasPlay" => Ok(json!(host.has_play)),
        "source" => Ok(host.source.clone()),
        "games" => Ok(json!(host.games)),
        "now" => Ok(json!(host.now)),
        // `crypto.randomUUID()`.
        "id" => Ok(json!(new_id())),
        other => Err(format!("An archive has no host read {other}")),
    }
}

fn effects(done: &Value) -> Vec<Effect> {
    done["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|effect| match effect["type"].as_str()? {
            "save" => Some(Effect::Save(effect["game"].clone())),
            "remove" => Some(Effect::Remove(effect["id"].as_str()?.to_string())),
            _ => None,
        })
        .collect()
}

impl GameArchive {
    /// The archive of a game, keeping its saved identity (`archive:<kind>` session) when there is
    /// one; `resume` continues the saved game rather than starting apart from it.
    pub fn new(
        identity: Option<&Value>,
        resume: bool,
        host: &Host<'_>,
    ) -> Result<(GameArchive, Vec<Effect>), String> {
        let state = transition(
            "archiveState",
            &[identity.cloned().unwrap_or(Value::Null)],
            |kind| read(host, kind),
        )?;
        let mut archive = GameArchive {
            state: state
                .get("value")
                .cloned()
                .ok_or("An archive could not start.")?,
        };
        let effects = archive.apply("archiveInit", &[json!(resume)], host)?;
        Ok((archive, effects))
    }

    fn apply(
        &mut self,
        method: &str,
        input: &[Value],
        host: &Host<'_>,
    ) -> Result<Vec<Effect>, String> {
        let mut args = vec![self.state.clone()];
        args.extend_from_slice(input);
        let done = transition(method, &args, |kind| read(host, kind))?;
        if let Some(next) = done.get("state") {
            assign_written(&mut self.state, next, &done["written"]);
        }
        Ok(effects(&done))
    }

    /// The game changed: save it to history when the rules say so.
    pub fn save(&mut self, finished: bool, host: &Host<'_>) -> Result<Vec<Effect>, String> {
        self.apply("archiveSave", &[json!(finished)], host)
    }

    /// A new game begins: the next one gets its own place in history.
    pub fn reset(&mut self, host: &Host<'_>) -> Result<Vec<Effect>, String> {
        self.apply("archiveReset", &[], host)
    }

    /// The identity to keep as the `archive:<kind>` session.
    pub fn identity(&self) -> Value {
        self.state["identity"].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::computer::{ComputerGame, Host as GameHost};

    #[test]
    fn a_game_with_moves_is_saved_and_a_new_one_gets_its_own_identity() {
        let game_host = GameHost {
            now: 0.,
            allowed: true,
            ready: true,
        };
        let mut game = ComputerGame::new(None, &game_host).unwrap();
        let source = game.archive_snapshot(&game_host);
        let empty: Vec<Value> = Vec::new();
        let host = |source: Value, has_play: bool| Host {
            source,
            has_play,
            games: &empty,
            now: 1.7e12,
        };
        let (mut archive, effects) = GameArchive::new(None, true, &host(source, false)).unwrap();
        assert!(effects.is_empty(), "{effects:?}");
        let first = archive.identity();
        assert!(first["id"].as_str().is_some_and(|id| id.len() == 36));

        game.play("e2e4", &game_host).unwrap();
        let effects = archive
            .save(false, &host(game.archive_snapshot(&game_host), true))
            .unwrap();
        let Some(Effect::Save(saved)) = effects.first() else {
            panic!("a game with a move is saved: {effects:?}");
        };
        assert_eq!(saved["id"], first["id"]);

        archive
            .reset(&host(game.archive_snapshot(&game_host), true))
            .unwrap();
        assert_ne!(archive.identity()["id"], first["id"]);
    }
}
