//! A game's place in Game history as every frontend keeps it: the archive rules decide
//! (`archive.rs`), and the frontend carries out the writes they ask for, in order.

use serde_json::Value;

use crate::archive::{Effect, GameArchive, Host};

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
    archive: Option<GameArchive>,
    saved_identity: Value,
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
        let mut host = Host {
            source,
            has_play,
            games,
            now: crate::now_ms(),
        };
        let started = GameArchive::new(
            (!identity.is_null()).then_some(&identity),
            resume,
            &mut host,
        );
        let writes = match started {
            Ok((archive, effects)) => {
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
            source,
            has_play,
            games,
        )
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(&mut self, source: Value, has_play: bool, games: &[Value]) -> Vec<Write> {
        self.run(|archive, host| archive.reset(host), source, has_play, games)
    }

    fn run(
        &mut self,
        step: impl FnOnce(&mut GameArchive, &mut Host<'_>) -> Vec<Effect>,
        source: Value,
        has_play: bool,
        games: &[Value],
    ) -> Vec<Write> {
        let Some(archive) = self.archive.as_mut() else {
            return Vec::new();
        };
        let mut host = Host {
            source,
            has_play,
            games,
            now: crate::now_ms(),
        };
        let effects = step(archive, &mut host);
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
        if let Some(identity) = self.archive.as_ref().map(GameArchive::identity)
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
    use serde_json::json;

    /// A new place keeps its identity once; a game with a move is saved under it, and the
    /// identity is not written again until a new game gets its own.
    #[test]
    fn the_identity_is_written_only_when_it_changes() {
        let mut games = Vec::new();
        let mut game = crate::local::LocalGame::new(None, 0.).unwrap();
        let (mut place, writes) = HistoryPlace::new(
            "board",
            Value::Null,
            true,
            game.archive_snapshot(),
            false,
            &games,
        );
        let writes = writes.unwrap();
        let [Write::SaveSession { kind, identity }] = writes.as_slice() else {
            panic!("a new place keeps its identity: {writes:?}");
        };
        assert_eq!(kind, "archive:board");
        game.play("e2e4", 1.);
        let source = game.archive_snapshot();
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

    #[test]
    fn removed_games_leave_the_list() {
        let mut games = vec![json!({ "id": "a" }), json!({ "id": "b" })];
        apply_to(&mut games, &[Write::RemoveGame("a".into())]);
        assert_eq!(games, [json!({ "id": "b" })]);
    }
}
