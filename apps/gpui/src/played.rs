//! The games played in this app, as Game history lists them (`useGameArchiveStore`): one list for
//! every page, changed at once and written to the profile in order by one writer, off the UI
//! thread so the app can wait for it while it quits. A write the core refuses shows what the
//! profile keeps again, and is tried again before the app quits.

use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui_kit::{App, AppContext as _, Context, Entity, Global, WeakEntity};
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::{Answer, AppCore, BoxedSave, Reads};

enum Write {
    Game(Value),
    Remove(String),
    /// A page's place in history (`archive:<kind>`), kept with the games it names.
    Session(String, Value),
    /// Done once every write before it was answered (with `true`, after those refused were tried
    /// again): with why one still failed, if any did.
    Flushed(oneshot::Sender<crate::core::Saved>, bool),
}

impl Write {
    /// What a later write replaces: a game (saved or removed), or a page's session.
    fn key(&self) -> String {
        match self {
            Self::Game(game) => format!("game:{}", game["id"]),
            Self::Remove(id) => format!("game:{}", json!(id)),
            Self::Session(key, _) => format!("session:{key}"),
            Self::Flushed(..) => String::new(),
        }
    }
}

pub struct PlayedGames {
    /// Newest saved first, as the core lists them.
    games: Vec<Value>,
    /// Why the last write failed, until one succeeds.
    pub error: Option<String>,
    writes: mpsc::UnboundedSender<Write>,
    reads: Reads,
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
        reads: Reads::default(),
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
            // Writes the core refused, the newest of each game and session, to try again.
            let mut refused: Vec<(Write, String)> = Vec::new();
            while let Some(write) = queue.next().await {
                let writes = match write {
                    Write::Flushed(done, again) => {
                        // Only before quitting: a failure's own reload must not retry in a loop.
                        let again: Vec<Write> = if again {
                            refused.drain(..).map(|(write, _)| write).collect()
                        } else {
                            Vec::new()
                        };
                        for write in again {
                            let result = send(&core, &write).await;
                            failing = settle(write, result, failing, &mut refused, &outcomes);
                        }
                        let _ = done.send(match refused.first() {
                            Some((_, cause)) => Err(format!("A game could not be saved: {cause}")),
                            None => Ok(()),
                        });
                        continue;
                    }
                    write => write,
                };
                let result = send(&core, &writes).await;
                failing = settle(writes, result, failing, &mut refused, &outcomes);
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

async fn send(core: &AppCore, write: &Write) -> Result<Value, kchess_core::CoreError> {
    match write {
        Write::Game(game) => core.call("saveArchivedGame", vec![game.clone()]).await,
        Write::Remove(id) => core.call("removeArchivedGame", vec![json!(id)]).await,
        Write::Session(key, value) => {
            core.call("saveSession", vec![json!(key), value.clone()])
                .await
        }
        Write::Flushed(..) => Ok(Value::Null),
    }
}

/// Note a write's result: a refused write is kept (replacing an older one for the same game or
/// session) and reported; the first success after a failure clears the page's error. Whether
/// writes are failing now.
fn settle(
    write: Write,
    result: Result<Value, kchess_core::CoreError>,
    failing: bool,
    refused: &mut Vec<(Write, String)>,
    outcomes: &mpsc::UnboundedSender<Option<String>>,
) -> bool {
    let key = write.key();
    refused.retain(|(kept, _)| kept.key() != key);
    match result {
        Err(cause) => {
            crate::log::write(
                Level::Warn,
                "history",
                &format!("saving a game failed: {}", cause.message),
            );
            let _ = outcomes.unbounded_send(Some(cause.message.clone()));
            refused.push((write, cause.message));
            true
        }
        Ok(_) if failing => {
            let _ = outcomes.unbounded_send(None);
            false
        }
        Ok(_) => false,
    }
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
        self.reads.edited();
        let _ = self.writes.unbounded_send(Write::Game(game));
        cx.notify();
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        let before = self.games.len();
        self.games.retain(|entry| entry["id"] != id);
        if self.games.len() != before {
            self.reads.edited();
            let _ = self.writes.unbounded_send(Write::Remove(id.to_string()));
            cx.notify();
        }
    }

    /// Keep a page's session after the game writes sent before it.
    pub fn save_session(&self, key: String, value: Value) {
        let _ = self.writes.unbounded_send(Write::Session(key, value));
    }

    /// Wait for every write sent so far, trying refused ones again: with why one still failed.
    pub fn flushed(&self) -> BoxedSave {
        self.written(true)
    }

    fn written(&self, again: bool) -> BoxedSave {
        let (done, wait) = oneshot::channel();
        let _ = self.writes.unbounded_send(Write::Flushed(done, again));
        Box::pin(async move {
            wait.await
                .unwrap_or_else(|_| Err("The game history is closed.".to_string()))
        })
    }

    /// Show the games the profile keeps (after a write failed).
    /// Show the profile's games: read once this window's writes are done, and never over a
    /// game saved or removed here after the read started (a deleted game would come back).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        let written = self.written(false);
        cx.spawn(async move |this, cx| {
            let _ = written.await;
            let Ok(library) = this.update(cx, |_, cx| {
                cx.global::<AppCore>().call("library", Vec::new())
            }) else {
                return;
            };
            if let Ok(library) = library.await
                && let Some(games) = library["games"].as_array()
            {
                let games = games.clone();
                let _ = this.update(cx, |this, cx| match this.reads.answer(read) {
                    Answer::Show => {
                        this.games = games;
                        cx.notify();
                    }
                    Answer::Again => this.reload(cx),
                    Answer::Superseded => {}
                });
            }
        })
        .detach();
    }
}
