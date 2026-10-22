//! A game's place in Game history, as a page keeps it (`useGameArchive`): the archive rules decide
//! (`archive.rs`) and the played games (`played.rs`) keep what they decide.

use gpui_kit::App;
use kchess_core::Level;
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

/// The games in history, as the rules read them.
fn games(cx: &App) -> Vec<Value> {
    cx.global::<PlayedHandle>().0.read(cx).games().to_vec()
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
        let games = games(cx);
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
        let games = games(cx);
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
                    Effect::Remove(id) => played.remove(&id, cx),
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
