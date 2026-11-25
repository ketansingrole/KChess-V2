//! The game databases on this computer: their list as the core reports it (`gameDbStatus`,
//! `gamedb:progress`), importing a PGN file into a new one, and the import's progress.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, PgnFilePath};
use kchess_contracts::types::{
    DatabaseInfo, GameDbImported, GameDbPhase, GameDbProgress, GameDbStatus, LichessGames,
    PlayerGames, ProfileArea,
};

use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::theme::Palette;
use crate::widgets;
use kchess_ui::icon;

pub struct GameDb {
    pub status: GameDbStatus,
    progress: Option<GameDbProgress>,
    error: Option<String>,
    /// Counts imports and failures shown: a status read that started before one of them is out
    /// of date when it lands, and neither its status nor its clearing counts.
    changes: u64,
    /// Cancel was asked of the import under way: its refusal is no failure to show.
    cancelling: bool,
    _events: Subscription,
}

/// The master games' database, as the core names it.
const MASTER: &str = "master";
/// The engine games' database.
const TCEC: &str = "tcec";

/// What the databases tell the views that asked for something.
pub enum GameDbEvent {
    /// A player's games are in their database, `database`.
    PlayerInstalled { database: String, username: String },
    /// A month of strong Lichess games is in its floor's database, `database`.
    LichessInstalled { database: String },
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
        changes: 0,
        cancelling: false,
        _events: cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
            if let ClientEvent::Event(CoreEvent::GameDbProgress(progress)) = event {
                // An import that ended is no longer running, whatever an earlier read said.
                if progress.phase.is_terminal() {
                    this.status.busy = false;
                    this.changes += 1;
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
        let asked = self.changes;
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |this, cx| {
                if this.changes != asked {
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
        self.changes += 1;
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

    /// Downloads the master games the master database lacks: every month at first, then the
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
        self.changes += 1;
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
        self.changes += 1;
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
/// What a download must leave free, so the computer and its other programs keep working.
const KEEP_FREE: u64 = 2 * 1024 * 1024 * 1024;

/// The bytes free on the databases' disk when they are too few for a download or import of
/// `size` bytes (unknown: none), its database and `KEEP_FREE` besides; `None` when there is room
/// or the system did not say.
pub fn short_of_room(free: Option<u64>, size: Option<u64>) -> Option<u64> {
    let free = free?;
    let need = size
        .unwrap_or(0)
        .saturating_mul(GROWTH)
        .saturating_add(KEEP_FREE);
    (free < need).then_some(free)
}

/// What the warning says: the room left, and what the games could take when their size is known.
fn room_left(free: u64, size: Option<u64>) -> String {
    let free = widgets::format_bytes(free as f64);
    match size {
        Some(size) => format!(
            "{free} is free. These games could take about {} once they are in.",
            widgets::format_bytes(size.saturating_mul(GROWTH) as f64)
        ),
        None => format!("{free} is free."),
    }
}

/// Run `start` when the disk has room for `size` bytes more (`short_of_room`); otherwise only
/// once the person says to `go` ahead anyway ("Download anyway").
pub fn with_room(
    size: Option<u64>,
    go: &'static str,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    let free = cx.global::<GameDbHandle>().0.read(cx).status.free_bytes;
    let Some(free) = short_of_room(free, size) else {
        start(window, cx);
        return;
    };
    let description = room_left(free, size);
    let start = std::rc::Rc::new(start);
    widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let start = start.clone();
        dialog
            .title("Your disk is almost full")
            .description(description.clone())
            .ok_text(go)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                start(window, cx);
                true
            })
    });
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

/// Import a PGN file, and the import's progress with Cancel while it runs.
pub fn import_card(cx: &App, palette: &Palette) -> impl IntoElement + use<> {
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
                    Some(month) => format!("Downloading master games · {month}"),
                    None if progress.database == MASTER => "Downloading master games…".to_string(),
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
                    .outline()
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
    use super::{GROWTH, KEEP_FREE, room_left, short_of_room};

    /// The warning names the room left, and what the games could take when that is known.
    #[test]
    fn the_warning_says_what_is_free_and_what_the_games_take() {
        let gb = 1024 * 1024 * 1024;
        assert_eq!(
            room_left(gb, Some(gb)),
            "1.0 GB is free. These games could take about 3.0 GB once they are in."
        );
        assert_eq!(room_left(gb, None), "1.0 GB is free.");
    }

    /// A download needs its database's room and `KEEP_FREE` besides; a disk the system says
    /// nothing about is never called full.
    #[test]
    fn a_download_is_short_of_room_below_its_database_and_a_margin() {
        let size = 1_000_000;
        let need = size * GROWTH + KEEP_FREE;
        assert_eq!(short_of_room(Some(need - 1), Some(size)), Some(need - 1));
        assert_eq!(short_of_room(Some(need), Some(size)), None);
        assert_eq!(
            short_of_room(Some(KEEP_FREE - 1), None),
            Some(KEEP_FREE - 1)
        );
        assert_eq!(short_of_room(Some(KEEP_FREE), None), None);
        assert_eq!(short_of_room(None, Some(u64::MAX)), None);
    }
}

#[cfg(test)]
mod tests {
    use super::GameDbHandle;
    use crate::core::{AppCore, ClientEvent, CoreEvent};
    use crate::testing::Started;
    use kchess_contracts::types::{GameDbPhase, GameDbProgress, GameDbStatus};

    /// An import lists its database; a refused one shows why, which the list read after it
    /// leaves on screen.
    #[gpui_kit::test]
    fn an_import_lists_its_database_and_a_refusal_stays_shown(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Started::new(cx);
        let db = app.with_window(|_, cx| {
            super::init(cx);
            cx.global::<GameDbHandle>().0.clone()
        });
        let files = tempfile::tempdir().expect("a folder");
        let path = files.path().join("club.pgn");
        std::fs::write(
            &path,
            "[White \"A\"]\n[Black \"B\"]\n[Result \"1-0\"]\n\n1. e4 1-0\n",
        )
        .expect("the file");
        app.with_window(|_, cx| db.update(cx, |db, cx| db.import(path, cx)));
        app.wait_for("the database listed", |_, cx| {
            let db = db.read(cx);
            db.databases().len() == 1 && !db.busy()
        });
        assert_eq!(
            app.with_window(|_, cx| db.read(cx).databases()[0].name.clone()),
            "club"
        );

        // The import's answer landed first and read the list; its end came after the read began
        // (Windows and Linux CI, PR 415). The list still shows the database.
        app.with_window(|_, cx| {
            db.update(cx, |db, cx| {
                db.status = GameDbStatus::default();
                db.refresh(cx);
            });
            let done = GameDbProgress {
                phase: GameDbPhase::Done,
                database: "club".into(),
                games: 1,
                skipped: 0,
                bytes: 0,
                total_bytes: None,
                message: None,
            };
            let events = cx.global::<AppCore>().events.clone();
            events.update(cx, |_, cx| {
                cx.emit(ClientEvent::Event(CoreEvent::GameDbProgress(done)))
            });
        });
        app.wait_for("the database listed after its late end", |_, cx| {
            db.read(cx).databases().len() == 1
        });

        let refused = files.path().join("notes.txt");
        app.with_window(|_, cx| db.update(cx, |db, cx| db.import(refused, cx)));
        app.wait_for("the refusal", |_, cx| db.read(cx).error.is_some());
        for _ in 0..20 {
            app.cx().run_until_parked();
        }
        let db = app.with_window(|_, cx| {
            let db = db.read(cx);
            (db.error.clone(), db.busy(), db.databases().len())
        });
        assert_eq!(
            db,
            (
                Some("Choose a .pgn, .pgn.zst, .pgn.bz2, .zip or .7z file.".into()),
                false,
                1
            )
        );
    }
}
