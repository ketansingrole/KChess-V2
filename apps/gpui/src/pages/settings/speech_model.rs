//! The speech model offline voice input listens with, as Settings → Storage lists it: its
//! size, or its download's progress, with Download and Delete.

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::*;
use kchess_contracts::voice::{Phase, Progress, Status};

use super::{Section, SettingsPage};
use crate::assets::icon;
use crate::widgets::{self, DownloadRow};

/// How often the row looks again while the model is being prepared elsewhere.
const BUSY_POLL: std::time::Duration = std::time::Duration::from_secs(2);
/// What the model takes: a 50 MB download unpacked to at most 200 MB.
const MODEL_TAKES: u64 = 250 * 1024 * 1024;

impl SettingsPage {
    /// Whether the speech model is installed (`voiceModelStatus`); checked again while busy.
    pub(super) fn refresh_voice_model(&mut self, cx: &mut Context<Self>) {
        let status = crate::voice::model_status(cx);
        cx.spawn(async move |this, cx| {
            let status = status.await;
            let busy = status.as_ref().is_some_and(|status| status.busy);
            let _ = this.update(cx, |this, cx| {
                if !this.voice_downloading {
                    match status {
                        Some(status) => this.voice_model = Some(status),
                        None => {
                            this.voice_model_error = Some("Could not check the voice model".into())
                        }
                    }
                }
                cx.notify();
            });
            if busy {
                cx.background_executor().timer(BUSY_POLL).await;
                let _ = this.update(cx, |this, cx| {
                    if this.section == Section::Storage {
                        this.refresh_voice_model(cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Download and prepare the model, once the disk has room for it.
    fn download_voice_model_with_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        crate::room::with_room(
            Some(MODEL_TAKES),
            "Download anyway",
            window,
            cx,
            move |_, cx| {
                this.update(cx, |this, cx| this.download_voice_model(cx))
                    .ok();
            },
        );
    }

    /// Download and prepare the model now.
    fn download_voice_model(&mut self, cx: &mut Context<Self>) {
        if self.voice_downloading || self.voice_model.as_ref().is_some_and(|status| status.busy) {
            return;
        }
        self.voice_downloading = true;
        self.voice_model_error = None;
        let (sender, mut updates) = futures::channel::mpsc::unbounded::<Progress>();
        let listener: crate::voice::Listener = std::sync::Arc::new(move |progress: &Progress| {
            let _ = sender.unbounded_send(progress.clone());
        });
        let download = crate::voice::download_model(listener, cx);
        cx.spawn(async move |this, cx| {
            let watch = {
                let this = this.clone();
                let mut cx = cx.clone();
                async move {
                    while let Some(progress) = updates.next().await {
                        let _ = this.update(&mut cx, |this, cx| {
                            this.voice_model = Some(Status {
                                installed: false,
                                bytes: 0,
                                busy: true,
                                progress: Some(progress),
                            });
                            cx.notify();
                        });
                    }
                }
            };
            let (outcome, ()) = futures::join!(download, watch);
            let _ = this.update(cx, |this, cx| {
                this.voice_downloading = false;
                let failed = match outcome {
                    Some(Ok(())) => None,
                    Some(Err(cause)) => Some(cause),
                    None => Some("Could not download the voice model".to_string()),
                };
                if let Some(cause) = failed {
                    crate::log::write(
                        kchess_contracts::Level::Warn,
                        "speech-model",
                        &format!("Could not download the voice model: {cause}"),
                    );
                    this.voice_model_error = Some(cause.into());
                }
                this.refresh_voice_model(cx);
                this.refresh_free_space(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn ask_delete_voice_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        widgets::confirm_danger(
            window,
            cx,
            "Delete the speech model?",
            "Voice input stops working until you download it again (about 50 MB).",
            "Delete",
            move |_, cx| {
                this.update(cx, |this, cx| this.delete_voice_model(cx)).ok();
            },
        );
    }

    fn delete_voice_model(&mut self, cx: &mut Context<Self>) {
        self.voice_model_error = None;
        let delete = crate::voice::delete_model(cx);
        cx.spawn(async move |this, cx| {
            let deleted = delete.await;
            this.update(cx, |this, cx| {
                if let Err(cause) = deleted {
                    this.voice_model_error =
                        Some(kchess_app::loading::failure("speech-model", &cause).into());
                }
                this.refresh_voice_model(cx);
                this.refresh_free_space(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The speech model's row: its size, or how far its download has come.
    pub(super) fn voice_model_row(&self, cx: &mut Context<Self>) -> DownloadRow {
        let status = self.voice_model.as_ref();
        let busy = self.voice_downloading || status.is_some_and(|status| status.busy);
        let row = |detail: String| {
            DownloadRow::new("storage-voice", icon("mic"), "Voice input", detail)
                .error("storage-voice-error", self.voice_model_error.clone())
        };
        if busy {
            let progress = status.and_then(|status| status.progress.as_ref());
            let share = progress.and_then(|progress| {
                let total = progress.total.filter(|total| *total > 0)?;
                Some((progress.received as f64 / total as f64).min(1.) as f32)
            });
            return row(progress_text(progress)).progress(share);
        }
        match status {
            None => row("Checking the speech model…".into()),
            Some(status) if status.installed => row(format!(
                "English speech model · {}",
                widgets::format_bytes(status.bytes as f64)
            ))
            .action(
                Button::new("storage-voice-delete")
                    .ghost()
                    .small()
                    .icon(icon("trash-2"))
                    .accessibility_label("Delete the speech model")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.ask_delete_voice_model(window, cx)),
                    ),
            ),
            Some(_) => row("English speech model · about 50 MB".into()).action(
                Button::new("storage-voice-download")
                    .outline()
                    .small()
                    .icon(icon("download"))
                    .label("Download")
                    .disabled(!crate::network::online(cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.download_voice_model_with_room(window, cx)
                    })),
            ),
        }
    }
}

/// How far the model's download has come, in words.
fn progress_text(progress: Option<&Progress>) -> String {
    match progress {
        None => "Checking the speech model…".to_string(),
        Some(progress) => match progress.phase {
            Phase::Checking => "Checking the speech model…".to_string(),
            Phase::Preparing => "Preparing offline speech recognition…".to_string(),
            Phase::Downloading => format!(
                "Downloading… {}{}",
                widgets::format_bytes(progress.received as f64),
                progress
                    .total
                    .map(|total| format!(" of {}", widgets::format_bytes(total as f64)))
                    .unwrap_or_default()
            ),
        },
    }
}
