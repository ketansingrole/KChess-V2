//! A game's place in Game history, as a page keeps it (`useGameArchive`): the archive rules decide
//! (`archive.rs`) and the played games (`played.rs`) keep what they decide.

use gpui_kit::App;
use kchess_contracts::Level;
use serde_json::Value;

use super::archive::{Effect, GameArchive, Host};
use crate::core::BoxedSave;
use crate::played::PlayedHandle;

pub struct HistoryPlace {
    kind: &'static str,
    archive: Option<GameArchive>,
    saved_identity: Value,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_millis() as f64)
}

/// The games in history as the rules read them: only the entry of the game with this identity.
/// The rules look a game up by its identity's id alone (before any new identity), and a copy of
/// every played game, which they then copy again for each read, made half of opening Over the
/// board over a full history.
fn games(identity: &Value, cx: &App) -> Vec<Value> {
    entries_of(cx.global::<PlayedHandle>().0.read(cx).games(), identity)
}

fn entries_of(games: &[Value], identity: &Value) -> Vec<Value> {
    let id = &identity["id"];
    if id.is_null() {
        return Vec::new();
    }
    games
        .iter()
        .filter(|game| game["id"] == *id)
        .cloned()
        .collect()
}

impl HistoryPlace {
    /// The place of a `kind` game (`computer`, `board`, `clock`), from the library's sessions;
    /// `resume` continues the saved game's entry instead of starting a new one.
    pub fn new(
        kind: &'static str,
        library: &Value,
        resume: bool,
        source: Value,
        has_play: bool,
        cx: &mut App,
    ) -> HistoryPlace {
        let identity = library["sessions"][format!("archive:{kind}")].clone();
        let mut place = HistoryPlace {
            kind,
            archive: None,
            saved_identity: identity.clone(),
        };
        let games = games(&identity, cx);
        let host = Host {
            source,
            has_play,
            games: &games,
            now: now_ms(),
        };
        match GameArchive::new((!identity.is_null()).then_some(&identity), resume, &host) {
            Ok((archive, effects)) => {
                place.archive = Some(archive);
                place.apply(effects, cx);
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "history",
                &format!("Game history could not start ({}): {cause}", place.kind),
            ),
        }
        place
    }

    /// The game changed (`watch(source, () => save())`); `finished` saves it as it is now.
    pub fn save(&mut self, finished: bool, source: Value, has_play: bool, cx: &mut App) {
        self.run(
            |archive, host| archive.save(finished, host),
            source,
            has_play,
            cx,
        );
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(&mut self, source: Value, has_play: bool, cx: &mut App) {
        self.run(|archive, host| archive.reset(host), source, has_play, cx);
    }

    fn run(
        &mut self,
        step: impl FnOnce(&mut GameArchive, &Host<'_>) -> Result<Vec<Effect>, String>,
        source: Value,
        has_play: bool,
        cx: &mut App,
    ) {
        let Some(mut archive) = self.archive.take() else {
            return;
        };
        let games = games(&archive.identity(), cx);
        let host = Host {
            source,
            has_play,
            games: &games,
            now: now_ms(),
        };
        let effects = step(&mut archive, &host);
        self.archive = Some(archive);
        match effects {
            Ok(effects) => self.apply(effects, cx),
            Err(cause) => crate::log::write(Level::Warn, "history", &cause),
        }
    }

    fn apply(&mut self, effects: Vec<Effect>, cx: &mut App) {
        let played = cx.global::<PlayedHandle>().0.clone();
        played.update(cx, |played, cx| {
            for effect in effects {
                match effect {
                    Effect::Save(game) => played.save(game, cx),
                    Effect::Remove(id) => {
                        played.remove(&id, cx);
                    }
                }
            }
            if let Some(identity) = self.archive.as_ref().map(GameArchive::identity)
                && identity != self.saved_identity
            {
                self.saved_identity = identity.clone();
                played.save_session(format!("archive:{}", self.kind), identity);
            }
        });
    }

    /// Wait for every write sent so far (before the app quits).
    pub fn flushed(&self, cx: &App) -> BoxedSave {
        cx.global::<PlayedHandle>().0.read(cx).flushed()
    }
}

#[cfg(test)]
mod tests {
    use super::entries_of;
    use serde_json::json;

    /// The rules read only the entry of the game they keep, not a copy of every played game.
    #[test]
    fn the_rules_read_only_their_games_entry() {
        let games = [
            json!({ "id": "a", "moves": [] }),
            json!({ "id": "b", "moves": ["e2e4"] }),
            json!({ "moves": [] }),
        ];
        assert_eq!(
            entries_of(&games, &json!({ "id": "b", "startedAt": 1 })),
            [games[1].clone()]
        );
        assert!(entries_of(&games, &json!({ "id": "c" })).is_empty());
        assert!(entries_of(&games, &serde_json::Value::Null).is_empty());
    }
}
