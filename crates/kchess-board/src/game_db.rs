//! The game databases on this computer: their list as the core reports it (`gameDbStatus`,
//! `gamedb:progress`), importing a PGN file into a new one, and the import's progress.

use std::path::PathBuf;

use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, PgnFilePath};
use kchess_contracts::types::{
    DatabaseInfo, GameDbPhase, GameDbProgress, GameDbStatus, ProfileArea,
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

    /// Read the list again: a read that works clears the failure an earlier one showed. A read
    /// that an import or a failure overtook changes nothing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
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
                        this.error = None;
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

    /// Imports the PGN file at `path` (`.pgn` or `.pgn.zst`) into a new database named after it.
    pub fn import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.changes += 1;
        self.error = None;
        self.cancelling = false;
        self.progress = Some(GameDbProgress {
            phase: GameDbPhase::Reading,
            database: String::new(),
            games: 0,
            skipped: 0,
            bytes: 0,
            total_bytes: None,
            message: None,
        });
        cx.notify();
        let import = cx.global::<AppCore>().request(calls::GameDbImport {
            path: PgnFilePath(path.to_string_lossy().into_owned()),
            into: None,
            name: None,
        });
        cx.spawn(async move |this, cx| {
            let result = import.await;
            this.update(cx, |this, cx| {
                this.progress = None;
                match result {
                    // The new database joins the list.
                    Ok(_) => this.refresh(cx),
                    // A failed or cancelled import leaves every database as it was.
                    Err(_) if this.cancelling => {}
                    Err(cause) => this.failed(&cause),
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
            cx.update(|_, cx| {
                let db = cx.global::<GameDbHandle>().0.clone();
                db.update(cx, |db, cx| db.import(path, cx));
            })
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
            (Some("Choose a .pgn or .pgn.zst file.".into()), false, 1)
        );
    }
}
