//! The games played in this app, as Game history lists them (`useGameArchiveStore`): one list for
//! every page, changed at once and written to the profile in order by one writer, off the UI
//! thread so the app can wait for it while it quits. A write the core refuses shows what the
//! profile keeps again.

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, WeakEntity};
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::{AppCore, BoxedSave};

enum Write {
    Game(Value),
    Remove(String),
    /// A page's place in history (`archive:<kind>`), kept with the games it names.
    Session(String, Value),
    Flushed(oneshot::Sender<()>),
}

pub struct PlayedGames {
    /// Newest saved first, as the core lists them.
    games: Vec<Value>,
    /// Why the last write failed, until one succeeds.
    pub error: Option<String>,
    writes: mpsc::UnboundedSender<Write>,
}

/// How views reach the played games: `cx.global::<PlayedHandle>().0`.
#[derive(Clone)]
pub struct PlayedHandle(pub Entity<PlayedGames>);

impl Global for PlayedHandle {}

/// Read the games before the first page opens, and start the writer.
pub fn init(cx: &mut App) {
    let games = cx
        .global::<AppCore>()
        .bridge
        .call_now("library", Vec::new())
        .ok()
        .and_then(|library| library["games"].as_array().cloned())
        .unwrap_or_default();
    let (writes, queue) = mpsc::unbounded();
    let entity = cx.new(|_| PlayedGames {
        games,
        error: None,
        writes,
    });
    write_in_order(entity.downgrade(), queue, cx);
    crate::core::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.read(cx).flushed())
    });
    // Another window saved or removed a game: show the profile's.
    crate::core::on_profile_change(cx, &["playedGames"], {
        let entity = entity.downgrade();
        move |cx| {
            let _ = entity.update(cx, |games, cx| games.reload(cx));
        }
    });
    cx.set_global(PlayedHandle(entity));
}

fn write_in_order(
    entity: WeakEntity<PlayedGames>,
    mut queue: mpsc::UnboundedReceiver<Write>,
    cx: &mut App,
) {
    let core = cx.global::<AppCore>().clone();
    // A failure, and the first success after one (`None`), for the page to show.
    let (outcomes, mut failed) = mpsc::unbounded::<Option<String>>();
    cx.background_executor()
        .spawn(async move {
            let mut failing = false;
            while let Some(write) = queue.next().await {
                let result = match write {
                    Write::Game(game) => core.call("saveArchivedGame", vec![game]).await,
                    Write::Remove(id) => core.call("removeArchivedGame", vec![json!(id)]).await,
                    Write::Session(key, value) => {
                        core.call("saveSession", vec![json!(key), value]).await
                    }
                    Write::Flushed(done) => {
                        let _ = done.send(());
                        continue;
                    }
                };
                match result {
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "history",
                            &format!("saving a game failed: {}", cause.message),
                        );
                        failing = true;
                        let _ = outcomes.unbounded_send(Some(cause.message));
                    }
                    Ok(_) if failing => {
                        failing = false;
                        let _ = outcomes.unbounded_send(None);
                    }
                    Ok(_) => {}
                }
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        while let Some(outcome) = failed.next().await {
            let _ = entity.update(cx, |this, cx| {
                if outcome.is_some() {
                    this.reload(cx);
                }
                this.error = outcome;
                cx.notify();
            });
        }
    })
    .detach();
}

impl PlayedGames {
    pub fn games(&self) -> &[Value] {
        &self.games
    }

    /// Save a game: it shows at once, ahead of the rest.
    pub fn save(&mut self, game: Value, cx: &mut Context<Self>) {
        let id = game["id"].clone();
        self.games.retain(|entry| entry["id"] != id);
        self.games.insert(0, game.clone());
        let _ = self.writes.unbounded_send(Write::Game(game));
        cx.notify();
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        let before = self.games.len();
        self.games.retain(|entry| entry["id"] != id);
        if self.games.len() != before {
            let _ = self.writes.unbounded_send(Write::Remove(id.to_string()));
            cx.notify();
        }
    }

    /// Keep a page's session after the game writes sent before it.
    pub fn save_session(&self, key: String, value: Value) {
        let _ = self.writes.unbounded_send(Write::Session(key, value));
    }

    /// Wait for every write sent so far.
    pub fn flushed(&self) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done));
        Box::pin(async move {
            let _ = wait.await;
        })
    }

    /// Show the games the profile keeps (after a write failed).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let library = cx.global::<AppCore>().call("library", Vec::new());
        cx.spawn(async move |this, cx| {
            if let Ok(library) = library.await
                && let Some(games) = library["games"].as_array()
            {
                let games = games.clone();
                let _ = this.update(cx, |this, cx| {
                    this.games = games;
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
