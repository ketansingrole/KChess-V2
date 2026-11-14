//! The downloaded puzzle database: its state as the core reports it
//! (`puzzleDbStatus`, `puzzledb:progress`), downloading, cancelling and deleting it, and the card
//! that shows it on Puzzles and in Offline downloads.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;

use crate::assets::icon;
use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::theme::{Palette, radii};
use crate::widgets::{self, format_bytes};
use kchess_contracts::types::{ProfileArea, PuzzleDbPhase, PuzzleDbProgress, PuzzleDbStatus};

pub struct PuzzleDb {
    pub status: PuzzleDbStatus,
    progress: Option<PuzzleDbProgress>,
    error: Option<String>,
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
        _events: cx.subscribe(&events, |this, _, event: &ClientEvent, cx| {
            if let ClientEvent::Event(CoreEvent::PuzzleDbProgress(progress)) = event {
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

    /// Read the database's state again (a page was shown).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().request(calls::PuzzleDbStatus {});
        cx.spawn(async move |this, cx| {
            let status = status.await;
            let _ = this.update(cx, |this, cx| {
                match status {
                    Ok(status) => this.status = status,
                    Err(cause) => this.error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn install(&mut self, cx: &mut Context<Self>) {
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
                    Err(cause) => this.error = Some(cause.message),
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
                let _ = cancel.await;
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
                    Err(cause) => this.error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn busy(&self) -> bool {
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
        if let Some(at) = this.status.imported_at {
            use chrono::TimeZone as _;
            if let Some(day) = chrono::Local.timestamp_millis_opt(at).single() {
                line.push_str(&format!(" · downloaded {}", day.format("%-m/%-d/%Y")));
            }
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
                .rounded(radii::XL)
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
