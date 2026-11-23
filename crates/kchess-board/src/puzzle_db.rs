//! The downloaded puzzle database: its state as the core reports it
//! (`puzzleDbStatus`, `puzzledb:progress`), downloading, cancelling and deleting it, and the card
//! that shows it on Puzzles and in Offline downloads.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;

use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::theme::{Palette, radii};
use crate::widgets::{self, format_bytes};
use kchess_contracts::types::{ProfileArea, PuzzleDbPhase, PuzzleDbProgress, PuzzleDbStatus};
use kchess_ui::icon;

pub struct PuzzleDb {
    pub status: PuzzleDbStatus,
    progress: Option<PuzzleDbProgress>,
    error: Option<String>,
    /// Counts installs, deletes and failures shown: a status read that started before one of
    /// them is out of date when it lands, and neither its status nor its clearing counts.
    changes: u64,
    _events: Subscription,
}

/// How views reach the database: `cx.global::<PuzzleDbHandle>().0`.
#[derive(Clone)]
pub struct PuzzleDbHandle(pub Entity<PuzzleDb>);

impl Global for PuzzleDbHandle {}

pub fn init(cx: &mut App) {
    let events = cx.global::<AppCore>().events.clone();
    let db = cx.new(|cx: &mut Context<PuzzleDb>| PuzzleDb {
        status: PuzzleDbStatus::default(),
        progress: None,
        error: None,
        changes: 0,
        _events: cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
            if let ClientEvent::Event(CoreEvent::PuzzleDbProgress(progress)) = event {
                // A download that ended is no longer running, whatever an earlier status read
                // said, and a read still on its way is out of date: a window that had read it
                // busy kept showing it downloading, with a Cancel that did nothing, after the
                // window that started it closed or it failed.
                if progress.phase.is_terminal() {
                    this.status.busy = false;
                    this.changes += 1;
                }
                this.progress = Some(progress.clone());
                cx.notify();
            }
        }),
    });
    cx.set_global(PuzzleDbHandle(db.clone()));
    db.update(cx, |db, cx| db.refresh(cx));
    // Another window downloaded or deleted the database: its card and modes follow at once.
    crate::core::on_profile_change(cx, &[ProfileArea::Puzzles], move |cx| {
        db.update(cx, |db, cx| db.refresh(cx));
    });
}

impl PuzzleDb {
    pub fn installed(&self) -> bool {
        self.status.installed
    }

    /// Read the database's state again (a page was shown): a read that works clears the
    /// failure an earlier one showed. A read that an install, a delete or a failure overtook
    /// changes nothing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().request(calls::PuzzleDbStatus {});
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
        self.error = Some(crate::loading::failure("puzzle-db", cause));
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        self.changes += 1;
        self.error = None;
        self.progress = Some(PuzzleDbProgress {
            phase: PuzzleDbPhase::Downloading,
            received: 0,
            kept: 0,
            total: None,
            message: None,
        });
        cx.notify();
        let install = cx.global::<AppCore>().request(calls::PuzzleDbInstall {});
        cx.spawn(async move |this, cx| {
            let result = install.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.status = status,
                    Err(cause) => this.failed(&cause),
                }
                let ended = this.progress.as_ref().is_none_or(|progress| {
                    matches!(
                        progress.phase,
                        PuzzleDbPhase::Done | PuzzleDbPhase::Cancelled
                    )
                });
                if ended || this.error.is_none() {
                    this.progress = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let cancel = cx.global::<AppCore>().request(calls::PuzzleDbCancel {});
        cx.background_executor()
            .spawn(async move {
                cancel.await.warn_on_err();
            })
            .detach();
    }

    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = cx.entity().downgrade();
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let db = db.clone();
            dialog
                .title("Delete the puzzle database?")
                .description(
                    "This removes the puzzles stored on this computer. Storm, Streak, Rush and offline puzzles stop working until you download it again. Your scores are kept.",
                )
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = db.update(cx, |db, cx| db.delete(cx));
                    true
                })
        });
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        self.changes += 1;
        self.error = None;
        let delete = cx.global::<AppCore>().request(calls::PuzzleDbDelete {});
        cx.spawn(async move |this, cx| {
            let result = delete.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => {
                        this.status = status;
                        this.progress = None;
                    }
                    Err(cause) => this.failed(&cause),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether a download runs: the card then shows its progress and Cancel.
    pub fn busy(&self) -> bool {
        self.status.busy
            || self
                .progress
                .as_ref()
                .is_some_and(|progress| !progress.phase.is_terminal())
    }
}

/// Download, see and delete the puzzles Downloaded mode plays from; `compact`
/// leaves out its title and frame, for a page that names it already.
pub fn card(compact: bool, cx: &App, palette: &Palette) -> impl IntoElement + use<> {
    let handle = cx.global::<PuzzleDbHandle>().0.clone();
    let this = handle.read(cx);
    let act = |run: fn(&mut PuzzleDb, &mut Window, &mut Context<PuzzleDb>)| {
        let handle = handle.downgrade();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = handle.update(cx, |db, cx| run(db, window, cx));
        }
    };
    let hint = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(palette.text_muted)
            .child(text)
    };
    let body: AnyElement = if this.busy() {
        let progress = this.progress.clone().unwrap_or(PuzzleDbProgress {
            phase: PuzzleDbPhase::Downloading,
            received: 0,
            kept: 0,
            total: None,
            message: None,
        });
        let received = progress.received as f64;
        let total = progress.total.map(|total| total as f64);
        let text = match progress.phase {
            PuzzleDbPhase::Importing => "Saving puzzles…".to_string(),
            PuzzleDbPhase::Failed => progress
                .message
                .unwrap_or_else(|| "The download failed".to_string()),
            PuzzleDbPhase::Downloading | PuzzleDbPhase::Done | PuzzleDbPhase::Cancelled => format!(
                "Downloading… {}{}",
                format_bytes(received),
                total
                    .map(|total| format!(" of {}", format_bytes(total)))
                    .unwrap_or_default()
            ),
        };
        let kept = progress.kept as f64;
        let share = total
            .filter(|total| *total > 0.)
            .map(|total| (received / total).min(1.));
        v_flex()
            .gap(px(8.))
            .child(
                div()
                    .h(px(8.))
                    .w_full()
                    .rounded_full()
                    .bg(palette.accented)
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(palette.primary)
                            .w(relative(share.unwrap_or(0.1) as f32)),
                    ),
            )
            .child(hint(if kept > 0. {
                format!(
                    "{text} · {} picked so far",
                    widgets::format_counted(kept, "puzzle", "puzzles")
                )
            } else {
                text
            }))
            .child(
                h_flex().child(
                    Button::new("db-cancel")
                        .outline()
                        .small()
                        .icon(icon("x"))
                        .label("Cancel")
                        .on_click(act(|db, _, cx| db.cancel(cx))),
                ),
            )
            .into_any_element()
    } else if this.status.installed {
        let mut line = format!(
            "{} · {} on disk",
            widgets::format_counted(this.status.count as f64, "puzzle", "puzzles"),
            format_bytes(this.status.bytes as f64)
        );
        if let Some(day) = this
            .status
            .imported_at
            .and_then(|at| crate::dates::local(at as f64))
        {
            line.push_str(&format!(" · downloaded {}", day.strftime("%-m/%-d/%Y")));
        }
        v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .child(
                        icon("circle-check")
                            .size(px(16.))
                            .text_color(palette.success),
                    )
                    .child(line),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("db-refresh")
                            .outline()
                            .small()
                            .icon(icon("refresh-cw"))
                            .label("Download a fresh sample (~300 MB)")
                            .on_click(act(|db, _, cx| db.install(cx))),
                    )
                    .child(
                        Button::new("db-delete")
                            .danger()
                            .small()
                            .icon(icon("trash-2"))
                            .label("Delete")
                            .on_click(act(|db, window, cx| db.ask_delete(window, cx))),
                    ),
            )
            .into_any_element()
    } else {
        v_flex()
            .gap(px(8.))
            .child(hint("About 300 MB download · keeps 100,000 puzzles".into()))
            .child(
                h_flex().child(
                    Button::new("db-install")
                        .primary()
                        .icon(icon("download"))
                        .label("Download puzzle database")
                        .on_click(act(|db, _, cx| db.install(cx))),
                ),
            )
            .into_any_element()
    };
    v_flex()
        .w_full()
        .max_w(px(440.))
        .gap(px(4.))
        .when(!compact, |card| {
            card.p(px(16.))
                .border_1()
                .border_color(palette.border)
                .rounded(radii::BOX)
                .bg(palette.bg)
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child("Puzzle database"),
                )
                .child(hint("For offline puzzles, Storm, Streak and Rush".into()))
        })
        .child(div().when(!compact, |body| body.mt(px(8.))).child(body))
        .children(this.error.clone().map(|error| {
            div()
                .text_size(px(12.))
                .text_color(palette.error)
                .child(error)
        }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::PuzzleDbHandle;
    use crate::testing::Started;

    /// A failed status read left its error under the page after a later read worked.
    #[gpui_kit::test]
    fn a_status_read_that_works_clears_an_earlier_failure(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Started::new(cx);
        let db = app.with_window(|_, cx| {
            super::init(cx);
            cx.global::<PuzzleDbHandle>().0.clone()
        });
        app.with_window(|_, cx| {
            db.update(cx, |db, cx| {
                db.error = Some("The puzzle database could not be read".into());
                db.refresh(cx);
            })
        });
        app.wait_for("the status read to clear the failure", |_, cx| {
            db.read(cx).error.is_none()
        });
    }

    /// Cancel stops a download under way: the install ends at once, with nothing installed.
    #[gpui_kit::test]
    fn cancel_stops_the_download_under_way(cx: &mut gpui_kit::TestAppContext) {
        let asked = Arc::new(AtomicBool::new(false));
        let server = {
            let asked = Arc::clone(&asked);
            // The database's first bytes never come, as from a stalled connection.
            kchess_testkit::Server::start(move |request, out| {
                if !request.target.starts_with("/database/") {
                    return kchess_testkit::respond(out, 404, "text/plain", b"");
                }
                asked.store(true, Ordering::SeqCst);
                kchess_testkit::ndjson_stream(out, Vec::new(), Duration::MAX, || true, |_| None)
            })
            .expect("a stand-in Lichess")
        };
        let mut app = Started::with_lichess(cx, Some(server.base().to_string()));
        let db = app.with_window(|_, cx| {
            super::init(cx);
            cx.global::<PuzzleDbHandle>().0.clone()
        });
        app.with_window(|_, cx| db.update(cx, |db, cx| db.install(cx)));
        app.wait_for("the download to start", |_, _| asked.load(Ordering::SeqCst));
        app.with_window(|_, cx| db.update(cx, |db, cx| db.cancel(cx)));
        app.wait_for("the install to end", |_, cx| db.read(cx).progress.is_none());
        assert!(app.with_window(|_, cx| db.read(cx).error.is_none()));
    }

    /// An install, a delete and a failure shown each move the count a status read compares on
    /// landing, so a read asked before any of them is known for out of date.
    #[gpui_kit::test]
    fn installs_deletes_and_failures_each_overtake_a_read(cx: &mut gpui_kit::TestAppContext) {
        let server = kchess_testkit::Server::start(|_, out| {
            kchess_testkit::respond(out, 404, "text/plain", b"")
        })
        .expect("a stand-in Lichess");
        let mut app = Started::with_lichess(cx, Some(server.base().to_string()));
        let db = app.with_window(|_, cx| {
            super::init(cx);
            cx.global::<PuzzleDbHandle>().0.clone()
        });
        let moved = |app: &mut Started, change: fn(&mut super::PuzzleDb, &mut gpui_kit::Context<super::PuzzleDb>)| {
            app.with_window(|_, cx| {
                db.update(cx, |db, cx| {
                    let before = db.changes;
                    change(db, cx);
                    db.changes > before
                })
            })
        };
        assert!(moved(&mut app, |db, _| db
            .failed(&kchess_contracts::CoreError::new("no"))));
        assert!(moved(&mut app, |db, cx| db.delete(cx)));
        assert!(moved(&mut app, |db, cx| db.install(cx)));
        // The install clears the failure shown as it starts: either answer ends it, a refusal
        // keeping its progress on the card.
        app.wait_for("the install's answer", |_, cx| {
            let db = db.read(cx);
            db.progress.is_none() || db.error.is_some()
        });
    }

    /// Deleting while a download runs is refused by the core: the refusal is logged and stays
    /// shown, whenever the status read asked before it lands.
    #[gpui_kit::test]
    fn deleting_during_a_download_shows_and_logs_the_refusal(cx: &mut gpui_kit::TestAppContext) {
        let asked = Arc::new(AtomicBool::new(false));
        let server = {
            let asked = Arc::clone(&asked);
            kchess_testkit::Server::start(move |request, out| {
                if !request.target.starts_with("/database/") {
                    return kchess_testkit::respond(out, 404, "text/plain", b"");
                }
                asked.store(true, Ordering::SeqCst);
                kchess_testkit::ndjson_stream(out, Vec::new(), Duration::MAX, || true, |_| None)
            })
            .expect("a stand-in Lichess")
        };
        let mut app = Started::with_lichess(cx, Some(server.base().to_string()));
        let db = app.with_window(|_, cx| {
            super::init(cx);
            cx.global::<PuzzleDbHandle>().0.clone()
        });
        app.with_window(|_, cx| db.update(cx, |db, cx| db.install(cx)));
        app.wait_for("the download to start", |_, _| asked.load(Ordering::SeqCst));
        let before = kchess_app::tests::logged_in("puzzle-db");
        app.with_window(|_, cx| db.update(cx, |db, cx| db.delete(cx)));
        app.wait_for("the refusal logged", |_, _| {
            kchess_app::tests::logged_in("puzzle-db") > before
        });
        // The database's first status read, asked before the delete, may land after it: it once
        // cleared the refusal shown (3 runs in 15).
        app.wait_for("the refusal shown", |_, cx| db.read(cx).error.is_some());
        for _ in 0..20 {
            app.cx().run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.with_window(|_, cx| db.read(cx).error.is_some()),
            "still shown"
        );
        app.with_window(|_, cx| db.update(cx, |db, cx| db.cancel(cx)));
    }
}
