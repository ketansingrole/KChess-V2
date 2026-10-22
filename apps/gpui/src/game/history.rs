//! A game's place in Game history, as a page keeps it (`useGameArchive`): the archive rules decide
//! (`archive.rs`); what they decide is written to the profile in order by one writer, which the
//! app waits for before it quits.

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::App;
use kchess_core::Level;
use serde_json::{Value, json};

use super::archive::{Effect, GameArchive, Host};
use crate::core::{AppCore, BoxedSave};

enum Write {
    Game(Value),
    Remove(String),
    Identity(Value),
    Flushed(oneshot::Sender<()>),
}

pub struct HistoryPlace {
    kind: &'static str,
    archive: Option<GameArchive>,
    /// The games in history, as the rules read them.
    games: Vec<Value>,
    saved_identity: Value,
    writes: mpsc::UnboundedSender<Write>,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_millis() as f64)
}

impl HistoryPlace {
    /// The place of a `kind` game (`computer`, `board`, `clock`) in the library just read;
    /// `resume` continues the saved game's entry instead of starting a new one.
    pub fn new(
        kind: &'static str,
        library: &Value,
        resume: bool,
        source: Value,
        has_play: bool,
        cx: &mut App,
    ) -> HistoryPlace {
        let core = cx.global::<AppCore>().clone();
        let (writes, mut queue) = mpsc::unbounded::<Write>();
        let session = format!("archive:{kind}");
        cx.background_executor()
            .spawn(async move {
                while let Some(write) = queue.next().await {
                    let result = match write {
                        Write::Game(game) => core.call("saveArchivedGame", vec![game]).await,
                        Write::Remove(id) => core.call("removeArchivedGame", vec![json!(id)]).await,
                        Write::Identity(identity) => {
                            core.call("saveSession", vec![json!(session), identity])
                                .await
                        }
                        Write::Flushed(done) => {
                            let _ = done.send(());
                            continue;
                        }
                    };
                    if let Err(cause) = result {
                        crate::log::write(
                            Level::Warn,
                            "history",
                            &format!("saving a game failed: {}", cause.message),
                        );
                    }
                }
            })
            .detach();
        let games = library["games"].as_array().cloned().unwrap_or_default();
        let identity = library["sessions"][format!("archive:{kind}")].clone();
        let mut place = HistoryPlace {
            kind,
            archive: None,
            games,
            saved_identity: identity.clone(),
            writes,
        };
        let host = Host {
            source,
            has_play,
            games: &place.games,
            now: now_ms(),
        };
        match GameArchive::new((!identity.is_null()).then_some(&identity), resume, &host) {
            Ok((archive, effects)) => {
                place.archive = Some(archive);
                place.apply(effects);
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
    pub fn save(&mut self, finished: bool, source: Value, has_play: bool) {
        self.run(
            |archive, host| archive.save(finished, host),
            source,
            has_play,
        );
    }

    /// A new game begins: the one before keeps its place and this one gets its own.
    pub fn reset(&mut self, source: Value, has_play: bool) {
        self.run(|archive, host| archive.reset(host), source, has_play);
    }

    fn run(
        &mut self,
        step: impl FnOnce(&mut GameArchive, &Host<'_>) -> Result<Vec<Effect>, String>,
        source: Value,
        has_play: bool,
    ) {
        let Some(mut archive) = self.archive.take() else {
            return;
        };
        let host = Host {
            source,
            has_play,
            games: &self.games,
            now: now_ms(),
        };
        let effects = step(&mut archive, &host);
        self.archive = Some(archive);
        match effects {
            Ok(effects) => self.apply(effects),
            Err(cause) => crate::log::write(Level::Warn, "history", &cause),
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Save(game) => {
                    let id = game["id"].clone();
                    self.games.retain(|entry| entry["id"] != id);
                    self.games.insert(0, game.clone());
                    let _ = self.writes.unbounded_send(Write::Game(game));
                }
                Effect::Remove(id) => {
                    self.games.retain(|entry| entry["id"] != id.as_str());
                    let _ = self.writes.unbounded_send(Write::Remove(id));
                }
            }
        }
        if let Some(identity) = self.archive.as_ref().map(GameArchive::identity)
            && identity != self.saved_identity
        {
            self.saved_identity = identity.clone();
            let _ = self.writes.unbounded_send(Write::Identity(identity));
        }
    }

    /// Wait for every write sent so far (before the app quits).
    pub fn flushed(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            let _ = wait.await;
        })
    }
}
