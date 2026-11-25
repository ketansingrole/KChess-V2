//! The endgame tablebases as Settings → Storage lists them: what is on disk, or the download's
//! progress, with Download (Resume after a stop), Cancel and Delete. Once on disk they answer the
//! analysis board's tablebase for up to five pieces without a connection.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::types::{TablebasePhase, TablebaseProgress};

use super::{Section, SettingsPage};
use crate::assets::icon;
use crate::core::AppCore;
use crate::log::ResultExt as _;
use crate::widgets::{self, DownloadRow, format_bytes};

impl SettingsPage {
    /// What the profile holds of the tablebases (`tablebaseStatus`).
    pub(super) fn refresh_tablebases(&mut self, cx: &mut Context<Self>) {
        let status = cx.global::<AppCore>().request(calls::TablebaseStatus {});
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |this, cx| {
                match status {
                    Ok(status) => this.tablebases = Some(status),
                    Err(cause) => {
                        this.tablebase_error =
                            Some(kchess_app::loading::failure("tablebases", &cause).into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A download's progress, from this window or another.
    pub(super) fn tablebase_progress(
        &mut self,
        progress: &TablebaseProgress,
        cx: &mut Context<Self>,
    ) {
        if progress.phase.is_terminal() {
            self.tablebase_progress = None;
            if self.section == Section::Storage {
                self.refresh_tablebases(cx);
                self.refresh_free_space(cx);
            }
        } else {
            self.tablebase_progress = Some(progress.clone());
        }
        cx.notify();
    }

    fn download_tablebases_with_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let takes = self
            .tablebases
            .as_ref()
            .map(|status| status.total_bytes.saturating_sub(status.bytes));
        let this = cx.entity().downgrade();
        crate::room::with_room(takes, "Download anyway", window, cx, move |_, cx| {
            this.update(cx, |this, cx| this.download_tablebases(cx))
                .ok();
        });
    }

    fn download_tablebases(&mut self, cx: &mut Context<Self>) {
        let Some(status) = &self.tablebases else {
            return;
        };
        self.tablebase_error = None;
        self.tablebase_progress = Some(TablebaseProgress {
            phase: TablebasePhase::Downloading,
            received: status.bytes,
            total: status.total_bytes,
            message: None,
        });
        cx.notify();
        let install = cx.global::<AppCore>().request(calls::TablebaseInstall {});
        cx.spawn(async move |this, cx| {
            let installed = install.await;
            this.update(cx, |this, cx| {
                this.tablebase_progress = None;
                match installed {
                    Ok(status) => this.tablebases = Some(status),
                    Err(cause) => {
                        this.tablebase_error =
                            Some(kchess_app::loading::failure("tablebases", &cause).into());
                        this.refresh_tablebases(cx);
                    }
                }
                this.refresh_free_space(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cancel_tablebases(&mut self, cx: &mut Context<Self>) {
        let cancel = cx.global::<AppCore>().request(calls::TablebaseCancel {});
        cx.background_spawn(async move {
            cancel.await.warn_on_err();
        })
        .detach();
    }

    fn ask_delete_tablebases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let this = this.clone();
            dialog
                .title("Delete the endgame tablebases?")
                .description("Endgames then need a connection to look up.")
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |this, cx| this.delete_tablebases(cx)).ok();
                    true
                })
        });
    }

    fn delete_tablebases(&mut self, cx: &mut Context<Self>) {
        self.tablebase_error = None;
        let delete = cx.global::<AppCore>().request(calls::TablebaseDelete {});
        cx.spawn(async move |this, cx| {
            let deleted = delete.await;
            this.update(cx, |this, cx| {
                match deleted {
                    Ok(status) => this.tablebases = Some(status),
                    Err(cause) => {
                        this.tablebase_error =
                            Some(kchess_app::loading::failure("tablebases", &cause).into())
                    }
                }
                this.refresh_free_space(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The bytes the tablebases take on disk.
    pub(super) fn tablebase_bytes(&self) -> u64 {
        self.tablebases.as_ref().map_or(0, |status| status.bytes)
    }

    /// The tablebases' row: what is on disk, or how far the download has come.
    pub(super) fn tablebase_row(&self, cx: &mut Context<Self>) -> DownloadRow {
        let row = |detail: String| {
            DownloadRow::new(
                "storage-tablebases",
                icon("chess-king"),
                "Endgame tablebases",
                detail,
            )
            .error("storage-tablebases-error", self.tablebase_error.clone())
        };
        let Some(status) = &self.tablebases else {
            return row("Checking…".into());
        };
        let busy = self.tablebase_progress.is_some() || status.busy;
        if busy {
            let (received, total) = self
                .tablebase_progress
                .as_ref()
                .map_or((status.bytes, status.total_bytes), |progress| {
                    (progress.received, progress.total)
                });
            let share = (total > 0).then(|| (received as f64 / total as f64).min(1.) as f32);
            return row(format!(
                "Downloading… {} of {}",
                format_bytes(received as f64),
                format_bytes(total as f64)
            ))
            .progress(share)
            .action(
                Button::new("storage-tablebases-cancel")
                    .ghost()
                    .small()
                    .icon(icon("x"))
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_tablebases(cx))),
            );
        }
        let pieces = format!("Up to {} pieces", status.pieces);
        let delete = || {
            Button::new("storage-tablebases-delete")
                .ghost()
                .small()
                .icon(icon("trash-2"))
                .accessibility_label("Delete the endgame tablebases")
                .on_click(cx.listener(|this, _, window, cx| this.ask_delete_tablebases(window, cx)))
        };
        let download =
            |label: &'static str| {
                Button::new("storage-tablebases-download")
                    .outline()
                    .small()
                    .icon(icon("download"))
                    .label(label)
                    .disabled(!crate::network::online(cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.download_tablebases_with_room(window, cx)
                    }))
            };
        if status.installed {
            return row(format!("{pieces} · {}", format_bytes(status.bytes as f64)))
                .action(delete());
        }
        if status.files > 0 {
            return row(format!(
                "{pieces} · {} of {}",
                format_bytes(status.bytes as f64),
                format_bytes(status.total_bytes as f64)
            ))
            .action(download("Resume"))
            .action(delete());
        }
        row(format!(
            "{pieces} · {}",
            format_bytes(status.total_bytes as f64)
        ))
        .action(download("Download"))
    }
}
