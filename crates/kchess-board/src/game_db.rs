//! The game databases on this computer: their list as the core reports it (`gameDbStatus`,
//! `gamedb:progress`), importing a PGN file into a new one, and the import's progress.

use std::path::PathBuf;

use futures::FutureExt as _;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{
    DatabaseId, DatabaseName, GameDbGameId, GameDbGameIds, PgnFilePath, PgnText,
};
use kchess_contracts::types::{
    DatabaseInfo, GameDbImported, GameDbPhase, GameDbProgress, GameDbQuery, GameDbSaved,
    GameDbStatus, LichessGames, PlayerGames, ProfileArea,
};

use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::theme::Palette;
use crate::widgets;
use kchess_ui::icon;

pub struct GameDb {
    pub status: GameDbStatus,
    progress: Option<GameDbProgress>,
    error: Option<String>,
    /// Status reads: an import or a failure shown since a read started makes it out of date when
    /// it lands, and neither its status nor its clearing counts.
    reads: crate::loading::Epoch,
    /// Cancel was asked of the import under way: its refusal is no failure to show.
    cancelling: bool,
    _events: Subscription,
}

/// The broadcast games' database, as the core names it.
const MASTER: &str = "master";
/// The engine games' database.
const TCEC: &str = "tcec";

/// What the databases tell the views that asked for something.
pub enum GameDbEvent {
    /// A player's games are in their database, `database`.
    PlayerInstalled { database: String, username: String },
    /// A month of strong Lichess games is in its floor's database, `database`.
    LichessInstalled { database: String },
    /// Games were copied into `database`.
    Copied { database: DatabaseInfo, added: u64 },
    /// The copies of games in a database were removed, `removed` of them.
    DuplicatesRemoved { removed: u64 },
}

/// Saves `pgn` (a game from the board) in the database `into`, or a new one named `name`: a new
/// game, or in the place of game `replacing` of `into`. Too small to show its progress, it runs
/// outside the import card; the list follows from the profile's change.
pub fn save_game(
    into: Option<String>,
    name: Option<String>,
    replacing: Option<u64>,
    pgn: String,
    cx: &App,
) -> futures::future::LocalBoxFuture<'static, Result<GameDbSaved, kchess_contracts::CoreError>> {
    let core = cx.global::<AppCore>();
    match (into, replacing) {
        (Some(database), Some(game)) => core
            .request(calls::GameDbReplaceGame {
                database: DatabaseId(database),
                game: GameDbGameId(game),
                pgn: PgnText(pgn),
            })
            .boxed_local(),
        (into, _) => core
            .request(calls::GameDbAddGames {
                pgn: PgnText(pgn),
                into: into.map(DatabaseId),
                name: name.map(DatabaseName),
            })
            .boxed_local(),
    }
}

/// The games an edit acts on: one game, or every game a search finds, whatever its page.
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen {
    Game(u64),
    /// Boxed: a search holds every filter, many times a game's id.
    Search(Box<GameDbQuery>),
}

impl Chosen {
    /// As the core's edits take them: the games listed, or the search.
    pub fn parts(&self) -> (Option<GameDbGameIds>, Option<GameDbQuery>) {
        match self {
            Chosen::Game(id) => (Some(GameDbGameIds(vec![*id])), None),
            Chosen::Search(query) => (None, Some(GameDbQuery::clone(query))),
        }
    }
}

/// The databases of strong Lichess games, as the core names them: `lichess-2400`.
const LICHESS: &str = "lichess-";

impl EventEmitter<GameDbEvent> for GameDb {}

/// How views reach the databases: `cx.global::<GameDbHandle>().0`.
#[derive(Clone)]
pub struct GameDbHandle(pub Entity<GameDb>);

impl Global for GameDbHandle {}

pub fn init(cx: &mut App) {
    let events = cx.global::<AppCore>().events.clone();
    let db = cx.new(|cx: &mut Context<GameDb>| GameDb {
        status: GameDbStatus::default(),
        progress: None,
        error: None,
        reads: crate::loading::Epoch::default(),
        cancelling: false,
        _events: cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
            if let ClientEvent::Event(CoreEvent::GameDbProgress(progress)) = event {
                // An import that ended is no longer running, whatever an earlier read said.
                if progress.phase.is_terminal() {
                    this.status.busy = false;
                    this.reads.invalidate();
                }
                this.progress = Some(progress.clone());
                // The event and the import's answer come apart, in either order: a read the
                // answer started is overtaken here, so one that starts now lists the database.
                if progress.phase == GameDbPhase::Done {
                    this.refresh(cx);
                }
                cx.notify();
            }
        }),
    });
    cx.set_global(GameDbHandle(db.clone()));
    db.update(cx, |db, cx| db.refresh(cx));
    // Another window or the command line imported, renamed or deleted one: the list follows.
    crate::core::on_profile_change(cx, &[ProfileArea::GameDb], move |cx| {
        db.update(cx, |db, cx| db.refresh(cx));
    });
}

impl GameDb {
    pub fn databases(&self) -> &[DatabaseInfo] {
        &self.status.databases
    }

    /// What went wrong with the last import, download or read, as the import card shows it.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Read the list again: a read that works clears the failure an earlier one showed. A read
    /// that an import or a failure overtook changes nothing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.read_status(true, cx);
    }

    /// Read the list again; `clear`: a read that works clears the failure shown.
    fn read_status(&mut self, clear: bool, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().request(calls::GameDbStatus {});
        let asked = self.reads.current();
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |this, cx| {
                if !this.reads.is_current(asked) {
                    return;
                }
                match status {
                    Ok(status) => {
                        this.status = status;
                        if clear {
                            this.error = None;
                        }
                    }
                    Err(cause) => this.failed(&cause),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows a failure, which a status read already under way leaves on screen.
    fn failed(&mut self, cause: &kchess_contracts::CoreError) {
        self.reads.invalidate();
        self.error = Some(crate::loading::failure("game-db", cause));
    }

    /// Imports the PGN file at `path` (`.pgn`, `.pgn.zst`, or a `.zip` or `.7z` of them) into a new
    /// database named after it.
    pub fn import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let import = cx.global::<AppCore>().request(calls::GameDbImport {
            path: PgnFilePath(path.to_string_lossy().into_owned()),
            into: None,
            name: None,
        });
        self.run(GameDbPhase::Reading, "", import, |_, _| {}, cx);
    }

    /// Downloads the broadcast games their database lacks: every month at first, then the
    /// months published since.
    pub fn install_masters(&mut self, cx: &mut Context<Self>) {
        let install = cx
            .global::<AppCore>()
            .request(calls::GameDbMasterInstall { months: None });
        self.run(GameDbPhase::Downloading, MASTER, install, |_, _| {}, cx);
    }

    /// Downloads TCEC's latest engine games, replacing those of an older release.
    pub fn install_tcec(&mut self, cx: &mut Context<Self>) {
        let install = cx.global::<AppCore>().request(calls::GameDbTcecInstall {});
        self.run(GameDbPhase::Downloading, TCEC, install, |_, _| {}, cx);
    }

    /// Downloads the games `request` names into its player's database, replacing those an
    /// earlier download brought; `GameDbEvent::PlayerInstalled` follows once they are in, and the
    /// player is among the site's recent ones.
    pub fn install_player(&mut self, request: PlayerGames, cx: &mut Context<Self>) {
        let username = request.username.clone();
        let site = request.site;
        let install = cx
            .global::<AppCore>()
            .request(calls::GameDbPlayerInstall { request });
        self.run(
            GameDbPhase::Downloading,
            "",
            install,
            move |imported: GameDbImported, cx| {
                crate::player_search::remember(site, &username, cx);
                cx.emit(GameDbEvent::PlayerInstalled {
                    database: imported.database.id,
                    username,
                });
            },
            cx,
        );
    }

    /// Downloads the month of Lichess's rated games `request` names (the newest its floor's
    /// database lacks, when none), keeping its strong games there; `GameDbEvent::LichessInstalled`
    /// follows once they are in.
    pub fn install_lichess(&mut self, request: LichessGames, cx: &mut Context<Self>) {
        let database = format!("{LICHESS}{}", request.min_rating);
        let install = cx
            .global::<AppCore>()
            .request(calls::GameDbLichessInstall { request });
        self.run(
            GameDbPhase::Downloading,
            &database,
            install,
            move |imported: GameDbImported, cx| {
                cx.emit(GameDbEvent::LichessInstalled {
                    database: imported.database.id,
                });
            },
            cx,
        );
    }

    /// Removes the `chosen` games from `database`, its opening tree built again from those left.
    pub fn delete_games(&mut self, database: String, chosen: Chosen, cx: &mut Context<Self>) {
        let (games, search) = chosen.parts();
        let delete = cx.global::<AppCore>().request(calls::GameDbDeleteGames {
            database: DatabaseId(database.clone()),
            games,
            search,
        });
        self.run(GameDbPhase::Indexing, &database, delete, |_, _| {}, cx);
    }

    /// Removes the games of `database` that copy an earlier one, its indexes built again.
    pub fn remove_duplicates(&mut self, database: String, cx: &mut Context<Self>) {
        let remove = cx
            .global::<AppCore>()
            .request(calls::GameDbRemoveDuplicates {
                database: DatabaseId(database.clone()),
            });
        self.run(
            GameDbPhase::Indexing,
            &database,
            remove,
            |removed, cx| cx.emit(GameDbEvent::DuplicatesRemoved { removed }),
            cx,
        );
    }

    /// Copies the `chosen` games of `from` into the database `into`, or a new one named `name`.
    pub fn copy_games(
        &mut self,
        from: String,
        chosen: Chosen,
        into: Option<String>,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (games, search) = chosen.parts();
        let target = into.clone().unwrap_or_default();
        let copy = cx.global::<AppCore>().request(calls::GameDbCopyGames {
            database: DatabaseId(from),
            games,
            search,
            into: into.map(DatabaseId),
            name: name.map(DatabaseName),
        });
        self.run(
            GameDbPhase::Reading,
            &target,
            copy,
            |copied: GameDbImported, cx| {
                cx.emit(GameDbEvent::Copied {
                    database: copied.database,
                    added: copied.added,
                })
            },
            cx,
        );
    }

    /// Runs an import or download into `database` (empty while the core has not named it), its
    /// progress shown from `phase` until it ends; `then` has its answer once it worked.
    fn run<T: 'static>(
        &mut self,
        phase: GameDbPhase,
        database: &str,
        work: impl std::future::Future<Output = Result<T, kchess_contracts::CoreError>> + 'static,
        then: impl FnOnce(T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.reads.invalidate();
        self.error = None;
        self.cancelling = false;
        self.progress = Some(GameDbProgress {
            phase,
            database: database.to_string(),
            games: 0,
            skipped: 0,
            bytes: 0,
            total_bytes: None,
            message: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.progress = None;
                match result {
                    // The new or grown database is listed.
                    Ok(answer) => {
                        this.refresh(cx);
                        then(answer, cx);
                    }
                    // A failed or cancelled run leaves every database as it was, but for the
                    // months a download already added, which the list then shows.
                    Err(_) if this.cancelling => this.refresh(cx),
                    Err(cause) => {
                        this.failed(&cause);
                        this.read_status(false, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Deletes database `id`; the list follows the core's answer.
    pub fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        self.reads.invalidate();
        self.error = None;
        let delete = cx.global::<AppCore>().request(calls::GameDbDelete {
            database: DatabaseId(id),
        });
        cx.spawn(async move |this, cx| {
            let result = delete.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.status = status,
                    Err(cause) => this.failed(&cause),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.cancelling = true;
        let cancel = cx.global::<AppCore>().request(calls::GameDbCancel {});
        cx.background_executor()
            .spawn(async move {
                cancel.await.warn_on_err();
            })
            .detach();
    }

    /// Whether an import runs: its progress then shows, with Cancel.
    pub fn busy(&self) -> bool {
        self.status.busy
            || self
                .progress
                .as_ref()
                .is_some_and(|progress| !progress.phase.is_terminal())
    }
}

/// A database grows to about twice its download once its games are indexed: TCEC's 27 MB
/// download made a 57 MB database (measured 2026-11-25). Three times leaves room for indexing.
const GROWTH: u64 = 3;

/// What a download or import of `size` bytes takes once its games are in a database.
pub fn grown(size: Option<u64>) -> Option<u64> {
    size.map(|size| size.saturating_mul(GROWTH))
}

/// Run `start` when the disk has room for a database made from `size` bytes of games
/// (`crate::room::with_room`); otherwise only once the person says to `go` ahead anyway.
pub fn with_room(
    size: Option<u64>,
    go: &'static str,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    crate::room::with_room(grown(size), go, window, cx, start);
}

/// Import the file at `path`, once its size is read (off the main thread) and the disk has room
/// for it (`with_room`).
pub fn import_with_room(path: PathBuf, window: &mut Window, cx: &mut App) {
    let size = cx.background_spawn({
        let path = path.clone();
        async move { std::fs::metadata(&path).map(|meta| meta.len()).ok() }
    });
    window
        .spawn(cx, async move |cx| {
            let size = size.await;
            cx.update(|window, cx| {
                with_room(size, "Import anyway", window, cx, move |_, cx| {
                    let db = cx.global::<GameDbHandle>().0.clone();
                    db.update(cx, |db, cx| db.import(path.clone(), cx));
                })
            })
            .ok();
        })
        .detach();
}

/// Ask for a PGN file, then import it. Nothing happens when the dialog is cancelled.
pub fn choose_and_import(window: &mut Window, cx: &mut App) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            cx.update(|window, cx| import_with_room(path, window, cx))
                .ok();
        })
        .detach();
}

/// Import a PGN file, and the import's progress with Cancel while it runs; `primary` where
/// importing is the page's main action.
pub fn import_card(cx: &App, primary: bool, palette: &Palette) -> impl IntoElement + use<> {
    let handle = cx.global::<GameDbHandle>().0.clone();
    let this = handle.read(cx);
    let hint = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(palette.text_muted)
            .child(text)
    };
    let body: AnyElement = if this.busy() {
        let progress = this.progress.clone();
        let (share, text) = match &progress {
            Some(progress) if progress.phase == GameDbPhase::Downloading => (
                progress
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| (progress.bytes as f64 / total as f64).min(1.) as f32),
                match &progress.message {
                    // Strong games, counted as the month is read.
                    Some(month) if progress.database.starts_with(LICHESS) => format!(
                        "Downloading Lichess games · {month} · {}",
                        widgets::format_counted(progress.games as f64, "game", "games")
                    ),
                    Some(month) => format!("Downloading broadcast games · {month}"),
                    None if progress.database == MASTER => {
                        "Downloading broadcast games…".to_string()
                    }
                    None if progress.database == TCEC => "Downloading engine games…".to_string(),
                    // A player's games, counted as they arrive.
                    None if progress.games > 0 => format!(
                        "Downloading · {}",
                        widgets::format_counted(progress.games as f64, "game", "games")
                    ),
                    None => "Downloading games…".to_string(),
                },
            ),
            Some(progress) if progress.phase == GameDbPhase::Indexing => (
                None,
                format!(
                    "Building the opening tree · {}",
                    widgets::format_counted(progress.games as f64, "game", "games")
                ),
            ),
            Some(progress) => (
                progress
                    .total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| (progress.bytes as f64 / total as f64).min(1.) as f32),
                format!(
                    "Importing · {}",
                    widgets::format_counted(progress.games as f64, "game", "games")
                ),
            ),
            None => (None, "Importing…".to_string()),
        };
        let cancel = handle.downgrade();
        v_flex()
            .gap(px(8.))
            .child(widgets::progress_bar(share, palette))
            .child(hint(text))
            .child(
                h_flex().child(
                    Button::new("gamedb-cancel")
                        .outline()
                        .small()
                        .icon(icon("x"))
                        .label("Cancel")
                        .on_click(move |_, _, cx| {
                            cancel.update(cx, |db, cx| db.cancel(cx)).ok();
                        }),
                ),
            )
            .into_any_element()
    } else {
        h_flex()
            .child(
                Button::new("gamedb-import")
                    .when(primary, |button| button.primary())
                    .when(!primary, |button| button.outline())
                    .small()
                    .icon(icon("import"))
                    .label("Import PGN…")
                    .on_click(|_, window, cx| choose_and_import(window, cx)),
            )
            .into_any_element()
    };
    v_flex()
        .gap(px(4.))
        .child(body)
        .children(this.error.clone().map(|error| {
            div()
                .text_size(px(12.))
                .text_color(palette.error)
                .child(error)
        }))
}

#[cfg(test)]
mod room_tests {
    /// A database takes about three times its download once indexed: TCEC's 27 MB made 57 MB.
    #[test]
    fn a_database_is_reckoned_at_three_times_its_download() {
        assert_eq!(super::grown(Some(27)), Some(81));
        assert_eq!(super::grown(None), None);
        assert_eq!(super::grown(Some(u64::MAX)), Some(u64::MAX));
    }
}
