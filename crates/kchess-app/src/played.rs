//! The games played in this app, as Game history lists them: one list for
//! every page, changed at once and written to the profile in order by one writer, off the UI
//! thread so the app can wait for it while it quits. A write the core refuses shows what the
//! profile keeps again, and is tried again before the app quits.

use crate::log::ResultExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use kchess_contracts::Level;
use kchess_contracts::calls;
use kchess_contracts::check::{ArchivedGame, LibraryId, SessionBase, SessionObject};
use kchess_contracts::types::{Library, PlayedGame, ProfileArea, SessionDocument, SessionKind};

use crate::core::{Answer, AppCore, Reads};
use crate::quit::{BoxedSave, Saved};
use crate::writer::{Store, Writer};

enum Write {
    Game(PlayedGame),
    Remove(String),
    /// A page's session, or its place in history, kept with the games it names.
    Session(SessionKind, SessionDocument),
    /// The page read the profile's session of this kind (inside, none when there was none): its
    /// saves from now on are made over it, and refused once it changed elsewhere. `None` when
    /// the read failed: the player's own new game is saved over whatever the profile has.
    Read(SessionKind, Option<Option<SessionDocument>>),
}

impl Write {
    /// What a later write replaces: a game (saved or removed), or a page's session.
    fn key(&self) -> String {
        match self {
            Self::Game(game) => format!("game:{}", game.id),
            Self::Remove(id) => format!("game:{id}"),
            Self::Session(kind, _) | Self::Read(kind, _) => format!("session:{}", kind.as_str()),
        }
    }
}

pub struct PlayedGames {
    /// Newest saved first, as the core lists them.
    games: Vec<PlayedGame>,
    /// Counts the changes to `games`, so a page can keep what it works out of them until the next.
    revision: u64,
    /// Why the last write failed, until one succeeds.
    pub error: Option<String>,
    writes: Writer<Write>,
    reads: Reads,
}

/// How views reach the played games: `cx.global::<PlayedHandle>().0`.
#[derive(Clone)]
pub struct PlayedHandle(pub Entity<PlayedGames>);

impl Global for PlayedHandle {}

/// Start from the games of the library read before the first page opens, and start the writer.
pub fn init(library: &Library, cx: &mut App) {
    let games = library.games.clone();
    // A failure, and the first success after one (`None`), for the page to show.
    let (outcomes, mut failed) = mpsc::unbounded::<Option<String>>();
    let saving = Saving {
        core: cx.global::<AppCore>().clone(),
        refused: Vec::new(),
        failing: false,
        outcomes,
        read: std::collections::BTreeMap::new(),
    };
    let writes = Writer::start(saving, "The game history is closed", cx);
    let entity = cx.new(|_| PlayedGames {
        games,
        revision: 0,
        error: None,
        writes,
        reads: Reads::default(),
    });
    cx.spawn({
        let entity = entity.downgrade();
        async move |cx| {
            while let Some(outcome) = failed.next().await {
                entity
                    .update(cx, |this, cx| {
                        if outcome.is_some() {
                            this.reload(cx);
                        }
                        this.error = outcome;
                        cx.notify();
                    })
                    .ok();
            }
        }
    })
    .detach();
    crate::quit::on_quit(cx, {
        let entity = entity.clone();
        move |cx| Some(entity.read(cx).flushed())
    });
    // Another window saved or removed a game: show the profile's.
    crate::core::on_profile_change(cx, &[ProfileArea::PlayedGames], {
        let entity = entity.downgrade();
        move |cx| {
            entity.update(cx, |games, cx| games.reload(cx)).ok();
        }
    });
    cx.set_global(PlayedHandle(entity));
}

/// The games' writes (`Writer`): a refused one is kept, the newest of each game and session, and
/// tried again before the app quits; a failure, and the first success after one, reach the page.
struct Saving {
    core: AppCore,
    /// Writes the core refused, the newest of each game and session, with why.
    refused: Vec<(Write, String)>,
    /// Whether the last write failed.
    failing: bool,
    outcomes: mpsc::UnboundedSender<Option<String>>,
    /// Each session kind a page read, as the profile holds it as far as this app knows: as read,
    /// then as last saved. A save that failed leaves it, so the next is made over what the
    /// profile still has; one refused as stale stays refused until the page reads again.
    read: std::collections::BTreeMap<&'static str, Option<SessionDocument>>,
}

impl Saving {
    /// Send `write`, and note how it went: a refused write is kept (replacing an older one for
    /// the same game or session) and reported; the first success after a failure clears the
    /// page's error.
    async fn send(&mut self, write: Write) {
        let based_on = match &write {
            Write::Read(kind, read) => {
                match read {
                    Some(session) => self.read.insert(kind.as_str(), session.clone()),
                    None => self.read.remove(kind.as_str()),
                };
                return;
            }
            Write::Session(kind, _) => self
                .read
                .get(kind.as_str())
                .map(|session| SessionBase::of(session.as_ref())),
            _ => None,
        };
        let result = send(&self.core, &write, based_on).await;
        if let (Ok(()), Write::Session(kind, session)) = (&result, &write)
            && let Some(read) = self.read.get_mut(kind.as_str())
        {
            *read = Some(session.clone());
        }
        let key = write.key();
        self.refused.retain(|(kept, _)| kept.key() != key);
        match result {
            Err(cause) => {
                crate::log::write(
                    Level::Warn,
                    "history",
                    &format!("saving a game failed: {}", cause.message),
                );
                let _ = self.outcomes.unbounded_send(Some(cause.message.clone()));
                self.refused.push((write, cause.message));
                self.failing = true;
            }
            Ok(()) if self.failing => {
                let _ = self.outcomes.unbounded_send(None);
                self.failing = false;
            }
            Ok(()) => {}
        }
    }
}

impl Store for Saving {
    type Write = Write;

    async fn write(&mut self, write: Write) {
        self.send(write).await;
    }

    async fn flush(&mut self, retry: bool) -> Saved {
        if retry {
            let again: Vec<Write> = self.refused.drain(..).map(|(write, _)| write).collect();
            for write in again {
                self.send(write).await;
            }
        }
        match self.refused.first() {
            Some((_, cause)) => Err(format!("A game could not be saved: {cause}")),
            None => Ok(()),
        }
    }
}

/// Sends a write; what the core answers is not read, only whether it went through.
async fn send(
    core: &AppCore,
    write: &Write,
    based_on: Option<SessionBase>,
) -> Result<(), kchess_contracts::CoreError> {
    match write {
        Write::Game(game) => core
            .request(calls::SaveArchivedGame {
                game: ArchivedGame::of(game),
            })
            .await
            .map(drop),
        Write::Remove(id) => {
            core.request(calls::RemoveArchivedGame {
                id: LibraryId(id.clone()),
            })
            .await
        }
        Write::Session(kind, session) => {
            core.request(calls::SaveSession {
                kind: *kind,
                session: SessionObject::of(session),
                based_on,
            })
            .await
        }
        Write::Read(..) => Ok(()),
    }
}

impl PlayedGames {
    pub fn games(&self) -> &[PlayedGame] {
        &self.games
    }

    /// Changes with every change to the games.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    /// Save a game: it shows at once, ahead of the rest.
    pub fn save(&mut self, game: PlayedGame, cx: &mut Context<Self>) {
        self.games.retain(|entry| entry.id != game.id);
        self.games.insert(0, game.clone());
        self.changed();
        self.reads.edited();
        self.writes.send(Write::Game(game));
        cx.notify();
    }

    /// Remove a game: the game and where it was listed, for `restore`, if it was listed.
    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) -> Option<(usize, PlayedGame)> {
        let index = self.games.iter().position(|entry| entry.id == id)?;
        let game = self.games.remove(index);
        self.changed();
        self.reads.edited();
        self.writes.send(Write::Remove(id.to_string()));
        cx.notify();
        Some((index, game))
    }

    /// Put a removed game back where it was listed (Undo).
    pub fn restore(&mut self, index: usize, game: PlayedGame, cx: &mut Context<Self>) {
        self.games.retain(|entry| entry.id != game.id);
        self.games.insert(index.min(self.games.len()), game.clone());
        self.changed();
        self.reads.edited();
        self.writes.send(Write::Game(game));
        cx.notify();
    }

    /// Keep a page's session after the game writes sent before it.
    pub fn save_session(&self, kind: SessionKind, session: SessionDocument) {
        self.writes.send(Write::Session(kind, session));
    }

    /// A page read the profile's session of `kind` (`None` when the read failed): its saves are
    /// made over it from now on.
    pub fn read_session(&self, kind: SessionKind, read: Option<Option<SessionDocument>>) {
        self.writes.send(Write::Read(kind, read));
    }

    /// Wait for every write sent so far, trying refused ones again: with why one still failed.
    pub fn flushed(&self) -> BoxedSave {
        self.writes.flushed()
    }

    /// Show the profile's games: read once this window's writes are done, and never over a
    /// game saved or removed here after the read started (a deleted game would come back).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read = self.reads.start();
        let written = self.writes.written();
        cx.spawn(async move |this, cx| {
            let _ = written.await;
            let Ok(library) = this.update(cx, |_, cx| {
                cx.global::<AppCore>().request(calls::PlayedGames {})
            }) else {
                return;
            };
            if let Some(games) = library.await.warn_on_err() {
                this.update(cx, |this, cx| match this.reads.answer(read) {
                    Answer::Show => {
                        this.games = games;
                        this.changed();
                        cx.notify();
                    }
                    Answer::Again => this.reload(cx),
                    Answer::Superseded => {}
                })
                .ok();
            }
        })
        .detach();
    }
}
