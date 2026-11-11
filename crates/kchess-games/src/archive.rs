//! A played game's place in Game history: the archive rules (`kchess_domain::games::archive`)
//! decide when the game is saved, replaced or dropped; this answers what they ask with the
//! frontend's values.

use serde_json::{Value, json};

use kchess_domain::games::archive::{self, Identity, State};

pub use kchess_domain::games::archive::Effect;

/// What the frontend tells the archive.
pub struct Host<'a> {
    /// The game as its archive snapshot describes it.
    pub source: Value,
    /// The game has moves.
    pub has_play: bool,
    /// The games already in history.
    pub games: &'a [Value],
    /// Milliseconds since the epoch.
    pub now: f64,
}

impl archive::Host for Host<'_> {
    fn has_play(&mut self) -> bool {
        self.has_play
    }
    fn source(&mut self) -> Value {
        self.source.clone()
    }
    fn stored(&mut self, id: &str) -> Option<Value> {
        self.games
            .iter()
            .find(|game| game.get("id").and_then(Value::as_str) == Some(id))
            .cloned()
    }
    fn now(&mut self) -> f64 {
        self.now
    }
    fn new_id(&mut self) -> String {
        new_id()
    }
}

/// A random version-4 UUID.
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

pub struct GameArchive {
    state: State,
}

impl GameArchive {
    /// The archive of a game, keeping its saved identity (`archive:<kind>` session) when there is
    /// one; `resume` continues the saved game rather than starting apart from it.
    pub fn new(
        identity: Option<&Value>,
        resume: bool,
        host: &mut Host<'_>,
    ) -> Result<(GameArchive, Vec<Effect>), String> {
        let identity = identity
            .map(|identity| serde_json::from_value::<Identity>(identity.clone()))
            .transpose()
            .map_err(|error| error.to_string())?;
        let mut state = State::new(identity, host);
        let effects = state.init(host, resume);
        // A resumed game was saved before, so taking back its only move removes that entry.
        if resume && archive::Host::stored(host, &state.identity.id).is_some() {
            state.had_play = true;
        }
        Ok((GameArchive { state }, effects))
    }

    /// The game changed: save it to history when the rules say so.
    pub fn save(&mut self, finished: bool, host: &mut Host<'_>) -> Vec<Effect> {
        self.state.save(host, finished)
    }

    /// A new game begins: the next one gets its own place in history.
    pub fn reset(&mut self, host: &mut Host<'_>) -> Vec<Effect> {
        self.state.reset(host)
    }

    /// The identity to keep as the `archive:<kind>` session.
    pub fn identity(&self) -> Value {
        json!(self.state.identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::{ComputerGame, Host as GameHost};

    #[test]
    fn a_game_with_moves_is_saved_and_a_new_one_gets_its_own_identity() {
        let game_host = GameHost {
            now: 0.,
            allowed: true,
            ready: true,
        };
        let mut game = ComputerGame::new(None, &game_host).unwrap();
        let empty: Vec<Value> = Vec::new();
        let host = |source: Value, has_play: bool| Host {
            source,
            has_play,
            games: &empty,
            now: 1.7e12,
        };
        let (mut archive, effects) =
            GameArchive::new(None, true, &mut host(game.archive_snapshot(), false)).unwrap();
        assert!(effects.is_empty(), "{effects:?}");
        let first = archive.identity();
        assert!(first["id"].as_str().is_some_and(|id| id.len() == 36));

        game.play("e2e4", &game_host);
        let effects = archive.save(false, &mut host(game.archive_snapshot(), true));
        let Some(Effect::Save(saved)) = effects.first() else {
            panic!("a game with a move is saved: {effects:?}");
        };
        assert_eq!(saved["id"], first["id"]);

        archive.reset(&mut host(game.archive_snapshot(), true));
        assert_ne!(archive.identity()["id"], first["id"]);
    }

    #[test]
    fn ids_are_version_4_uuids() {
        let ids: Vec<String> = (0..64).map(|_| new_id()).collect();
        for id in &ids {
            let chars: Vec<char> = id.chars().collect();
            assert_eq!(chars.len(), 36, "{id}");
            assert_eq!(chars[14], '4', "{id}");
            assert!("89ab".contains(chars[19]), "{id}");
        }
        assert!(
            ids.iter()
                .any(|id| id.as_bytes()[15] != ids[0].as_bytes()[15])
        );
    }

    /// Restored after a restart, a game whose only move is taken back leaves no entry behind.
    #[test]
    fn a_resumed_game_taken_back_to_its_start_leaves_no_entry() {
        let identity = json!({ "id": "resumed-game", "startedAt": 1.0e12 });
        let saved = vec![json!({ "id": "resumed-game", "moves": ["e2e4"], "result": "*" })];
        let mut host = Host {
            source: json!({ "moves": [], "result": "*" }),
            has_play: false,
            games: &saved,
            now: 1.7e12,
        };
        let (mut archive, effects) = GameArchive::new(Some(&identity), true, &mut host).unwrap();
        assert!(effects.is_empty(), "{effects:?}");
        let effects = archive.save(false, &mut host);
        assert_eq!(effects, vec![Effect::Remove("resumed-game".into())]);
    }
}
