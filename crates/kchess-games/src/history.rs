//! A game's place in Game history as every frontend keeps it: the archive rules
//! (`kchess_domain::games::archive`) decide when the game is saved, replaced or dropped, and the
//! frontend carries out the writes they ask for, in order.

use serde_json::{Value, json};

use kchess_domain::games::archive::{Effect, Host, Identity, State};

/// A write to the profile's library a Game history place asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// Save (or replace) a played game.
    SaveGame(Value),
    /// Remove the played game with this id.
    RemoveGame(String),
    /// Keep the place's identity as the session `archive:<kind>`.
    SaveSession { kind: String, identity: Value },
}

/// The place in Game history of a `kind` game (`computer`, `board`, `clock`).
pub struct HistoryPlace {
    kind: &'static str,
    archive: Option<State>,
    saved_identity: Value,
}

/// What the archive is told of the game now.
fn host<'a>(source: &'a Value, has_play: bool, games: &'a [Value]) -> Host<'a> {
    Host {
        has_play,
        source,
        games,
        now: crate::now_ms(),
        new_id: new_id(),
    }
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

/// The session a `kind` game's identity is kept as.
pub fn session_kind(kind: &str) -> String {
    format!("archive:{kind}")
}

impl HistoryPlace {
    /// The place kept by `identity` (the `archive:<kind>` session, null when there is none),
    /// among the played `games`; `resume` continues the saved game's entry instead of starting a
    /// new one. A place the rules could not start keeps nothing, and says why.
    pub fn new(
        kind: &'static str,
        identity: Value,
        resume: bool,
        source: Value,
        has_play: bool,
        games: &[Value],
    ) -> (HistoryPlace, Result<Vec<Write>, String>) {
        let mut place = HistoryPlace {
            kind,
            archive: None,
            saved_identity: identity.clone(),
        };
        let kept = (!identity.is_null())
            .then(|| serde_json::from_value::<Identity>(identity))
            .transpose();
        let writes = match kept {
            Ok(kept) => {
                let host = host(&source, has_play, games);
                let mut archive = State::new(kept, &host);
                let effects = archive.init(resume, &host);
                place.archive = Some(archive);
                Ok(place.writes(effects))
            }
            Err(cause) => Err(format!("Game history could not start ({kind}): {cause}")),
        };
        (place, writes)
    }

    /// The game changed; `finished` saves it as it is now.
    pub fn save(
        &mut self,
        finished: bool,
        source: Value,
        has_play: bool,
        games: &[Value],
    ) -> Vec<Write> {
        self.run(
            |archive, host| archive.save(finished, host),
            &source,
            has_play,
            games,
        )
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(&mut self, source: Value, has_play: bool, games: &[Value]) -> Vec<Write> {
        self.run(
            |archive, host| archive.reset(host),
            &source,
            has_play,
            games,
        )
    }

    fn run(
        &mut self,
        step: impl FnOnce(&mut State, &Host<'_>) -> Vec<Effect>,
        source: &Value,
        has_play: bool,
        games: &[Value],
    ) -> Vec<Write> {
        let Some(archive) = self.archive.as_mut() else {
            return Vec::new();
        };
        let effects = step(archive, &host(source, has_play, games));
        self.writes(effects)
    }

    /// The archive's effects as writes, then the identity when it changed.
    fn writes(&mut self, effects: Vec<Effect>) -> Vec<Write> {
        let mut writes: Vec<Write> = effects
            .into_iter()
            .map(|effect| match effect {
                Effect::Save(game) => Write::SaveGame(game),
                Effect::Remove(id) => Write::RemoveGame(id),
            })
            .collect();
        if let Some(identity) = self.archive.as_ref().map(|archive| json!(archive.identity))
            && identity != self.saved_identity
        {
            self.saved_identity = identity.clone();
            writes.push(Write::SaveSession {
                kind: session_kind(self.kind),
                identity,
            });
        }
        writes
    }
}

/// Keep `games` (the played games a frontend holds) as the writes leave them.
pub fn apply_to(games: &mut Vec<Value>, writes: &[Write]) {
    for write in writes {
        match write {
            Write::SaveGame(game) => {
                games.retain(|kept| kept["id"] != game["id"]);
                games.push(game.clone());
            }
            Write::RemoveGame(id) => games.retain(|kept| kept["id"] != id.as_str()),
            Write::SaveSession { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let source = json!({ "moves": [], "result": "*" });
        let (mut place, writes) =
            HistoryPlace::new("computer", identity, true, source.clone(), false, &saved);
        assert_eq!(writes, Ok(Vec::new()));
        let writes = place.save(false, source, false, &saved);
        assert_eq!(writes, vec![Write::RemoveGame("resumed-game".into())]);
    }

    /// A new place keeps its identity once; a game with a move is saved under it, and the
    /// identity is not written again until a new game gets its own.
    #[test]
    fn the_identity_is_written_only_when_it_changes() {
        let mut games = Vec::new();
        let mut game = crate::local::LocalGame::resume(None, 0.).unwrap();
        let (mut place, writes) = HistoryPlace::new(
            "board",
            Value::Null,
            true,
            json!(game.archive_snapshot()),
            false,
            &games,
        );
        let writes = writes.unwrap();
        let [Write::SaveSession { kind, identity }] = writes.as_slice() else {
            panic!("a new place keeps its identity: {writes:?}");
        };
        assert_eq!(kind, "archive:board");
        game.play("e2e4", 1.);
        let source = json!(game.archive_snapshot());
        let writes = place.save(false, source.clone(), true, &games);
        assert!(
            matches!(writes.first(), Some(Write::SaveGame(game)) if game["id"] == identity["id"]),
            "{writes:?}"
        );
        apply_to(&mut games, &writes);
        assert_eq!(games.len(), 1);
        let writes = place.save(false, source.clone(), true, &games);
        assert!(
            writes
                .iter()
                .all(|write| !matches!(write, Write::SaveSession { .. })),
            "{writes:?}"
        );
        let writes = place.reset(source, true, &games);
        assert!(
            writes.iter().any(|write| matches!(
                write,
                Write::SaveSession { identity: next, .. } if next["id"] != identity["id"]
            )),
            "{writes:?}"
        );
    }

    /// Each history kind's identity is a session the library keeps.
    #[test]
    fn every_place_is_kept_as_a_known_session() {
        use kchess_domain::misc::constants::SESSION_KINDS;
        for kind in [
            crate::computer::HISTORY,
            crate::local::HISTORY,
            crate::local::CLOCK_HISTORY,
        ] {
            assert!(
                SESSION_KINDS.contains(&session_kind(kind).as_str()),
                "{kind}"
            );
        }
    }

    #[test]
    fn removed_games_leave_the_list() {
        let mut games = vec![json!({ "id": "a" }), json!({ "id": "b" })];
        apply_to(&mut games, &[Write::RemoveGame("a".into())]);
        assert_eq!(games, [json!({ "id": "b" })]);
    }
}
