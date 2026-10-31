//! The downloaded puzzle database: its state as the core reports it
//! (`puzzleDbStatus`, `puzzledb:progress`), downloading, cancelling and deleting it, and the card
//! that shows it on Puzzles and in Offline downloads.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::{AppCore, CoreEvent};
use crate::theme::{Palette, radii};
use crate::widgets::{self, format_bytes};

pub struct PuzzleDb {
    /// `PuzzleDbStatus`: installed, count, bytes, importedAt, busy.
    pub status: Value,
    progress: Option<Value>,
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
        status: Value::Null,
        progress: None,
        error: None,
        _events: cx.subscribe(&events, |this, _, event: &CoreEvent, cx| {
            if event.name == "puzzledb:progress" {
                this.progress = Some(event.payload.clone());
                cx.notify();
            }
        }),
    });
    cx.set_global(PuzzleDbHandle(db.clone()));
    db.update(cx, |db, cx| db.refresh(cx));
}

impl PuzzleDb {
    pub fn installed(&self) -> bool {
        self.status["installed"] == json!(true)
    }

    /// Read the database's state again (a page was shown).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().call("puzzleDbStatus", Vec::new());
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
        self.progress = Some(json!({ "phase": "downloading", "received": 0, "kept": 0 }));
        cx.notify();
        let install = cx.global::<AppCore>().call("puzzleDbInstall", Vec::new());
        cx.spawn(async move |this, cx| {
            let result = install.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(status) => this.status = status,
                    Err(cause) => this.error = Some(cause.message),
                }
                let phase = this
                    .progress
                    .as_ref()
                    .and_then(|progress| progress["phase"].as_str().map(str::to_string));
                if matches!(phase.as_deref(), Some("done" | "cancelled") | None)
                    || this.error.is_none()
                {
                    this.progress = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let cancel = cx.global::<AppCore>().call("puzzleDbCancel", Vec::new());
        cx.background_executor()
            .spawn(async move {
                let _ = cancel.await;
            })
            .detach();
    }

    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
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
        let delete = cx.global::<AppCore>().call("puzzleDbDelete", Vec::new());
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
        self.status["busy"] == json!(true)
            || self.progress.as_ref().is_some_and(|progress| {
                matches!(
                    progress["phase"].as_str(),
                    Some("downloading" | "importing")
                )
            })
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
        let progress = this.progress.clone().unwrap_or(Value::Null);
        let received = progress["received"].as_f64().unwrap_or_default();
        let total = progress["total"].as_f64();
        let text = match progress["phase"].as_str() {
            Some("importing") => "Saving puzzles…".to_string(),
            Some("failed") => progress["message"]
                .as_str()
                .unwrap_or("The download failed")
                .to_string(),
            _ => format!(
                "Downloading… {}{}",
                format_bytes(received),
                total
                    .map(|total| format!(" of {}", format_bytes(total)))
                    .unwrap_or_default()
            ),
        };
        let kept = progress["kept"].as_f64().unwrap_or_default();
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
                    "{text} · {} puzzles picked so far",
                    widgets::format_count(kept)
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
    } else if this.status["installed"] == json!(true) {
        let mut line = format!(
            "{} puzzles · {} on disk",
            widgets::format_count(this.status["count"].as_f64().unwrap_or_default()),
            format_bytes(this.status["bytes"].as_f64().unwrap_or_default())
        );
        if let Some(at) = this.status["importedAt"].as_f64() {
            use chrono::TimeZone as _;
            if let Some(day) = chrono::Local.timestamp_millis_opt(at as i64).single() {
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
