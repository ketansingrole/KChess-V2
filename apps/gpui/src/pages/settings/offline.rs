//! Settings → Offline downloads: what KChess keeps on this computer to
//! work without a connection — the chess engine, the puzzle database, and the games and studies
//! saved in the profile, and the speech model for offline voice input.

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::voice::{Phase, Progress, Status};

use super::{Section, SettingsPage, engine::engine_name};
use crate::assets::icon;
use crate::pages::Page;
use crate::theme::Palette;
use crate::widgets::{self, Look};

/// How often the card looks again while the model is being prepared elsewhere.
const BUSY_POLL: std::time::Duration = std::time::Duration::from_secs(2);

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
                    if this.section == Section::Offline {
                        this.refresh_voice_model(cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Download and prepare the model now (`ensureVoiceModel`).
    fn download_voice_model(&mut self, cx: &mut Context<Self>) {
        if self.voice_downloading || self.voice_model.as_ref().is_some_and(|status| status.busy) {
            return;
        }
        self.voice_downloading = true;
        self.voice_model_error = None;
        let (sender, mut updates) = futures::channel::mpsc::unbounded::<Progress>();
        let listener: kchess_core::voice::Listener =
            std::sync::Arc::new(move |progress: &Progress| {
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
                        kchess_core::Level::Warn,
                        "offline-downloads",
                        &format!("Could not download the voice model: {cause}"),
                    );
                    this.voice_model_error = Some(cause.into());
                }
                this.refresh_voice_model(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn voice_model_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let status = self.voice_model.as_ref();
        let busy = self.voice_downloading || status.is_some_and(|status| status.busy);
        let badge = match status {
            Some(status) if status.installed => Some(widgets::badge(
                format!("Installed · {}", widgets::format_bytes(status.bytes as f64)),
                Look::Success,
                palette,
            )),
            Some(status) if !status.busy => Some(widgets::badge(
                "Download needed",
                Look::NeutralSoft,
                palette,
            )),
            _ => None,
        };
        let progress = status.and_then(|status| status.progress.as_ref());
        let share = progress
            .and_then(|progress| {
                progress
                    .total
                    .filter(|total| *total > 0)
                    .map(|total| (progress.received, total))
            })
            .map(|(received, total)| (received as f64 / total as f64).min(1.));
        let progress_text = match progress {
            None => "Checking the voice model…".to_string(),
            Some(progress) => match progress.phase {
                Phase::Checking => "Checking the voice model…".to_string(),
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
        };
        let online = crate::network::online(cx);
        widgets::card(palette)
            .gap(px(8.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .child(widgets::heading("Offline voice input", palette))
                    .children(badge),
            )
            .when(
                status.is_some_and(|status| !status.installed && !busy),
                |card| card.child(widgets::hint("English speech model · up to 50 MB", palette)),
            )
            .when(busy, |card| {
                card.child(
                    div()
                        .mt(px(6.))
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
                .child(widgets::hint(progress_text, palette))
            })
            .when(
                status.is_none() && self.voice_model_error.is_none(),
                |card| card.child(widgets::hint("Checking installed content…", palette)),
            )
            .children(self.voice_model_error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .when(
                status.is_some_and(|status| !status.installed) && !busy,
                |card| {
                    card.child(
                        h_flex().mt(px(6.)).child(
                            Button::new("offline-voice-download")
                                .primary()
                                .icon(icon("download"))
                                .label("Download voice model")
                                .disabled(!online)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.download_voice_model(cx)),
                                ),
                        ),
                    )
                },
            )
            .when(status.is_some_and(|status| status.installed), |card| {
                card.child(
                    h_flex().mt(px(6.)).child(
                        Button::new("offline-voice-settings")
                            .outline()
                            .icon(icon("mic"))
                            .label("Voice input settings")
                            .on_click(|_, window, cx| {
                                crate::shell::open_settings(Section::Voice, window, cx)
                            }),
                    ),
                )
            })
    }

    pub(super) fn offline(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let status = self.engine.as_ref();
        let ready = status.is_some_and(|status| status.ready);
        let name = engine_name(status);
        let managed = status
            .map(|status| &status.managed)
            .filter(|managed| managed.installed);
        let engine = widgets::card(palette)
            .gap(px(8.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .child(widgets::heading("Chess engine", palette))
                    .child(widgets::badge(
                        "Included with KChess",
                        Look::Success,
                        palette,
                    )),
            )
            .when(ready && !name.is_empty(), |card| {
                card.child(widgets::hint(format!("Selected engine: {name}."), palette))
            })
            .children(managed.map(|managed| {
                let version = managed
                    .version
                    .as_ref()
                    .map(|version| format!(" · {version}"))
                    .unwrap_or_default();
                widgets::hint(
                    format!("Optional native engine installed{version}"),
                    palette,
                )
            }))
            .when(status.is_some() && !ready, |card| {
                card.child(widgets::hint(
                    "Selected engine unavailable. Choose another in Chess engine settings.",
                    palette,
                ))
            })
            .child(
                h_flex().mt(px(6.)).child(
                    Button::new("offline-engine")
                        .outline()
                        .icon(icon("cpu"))
                        .label("Chess engine settings")
                        .on_click(|_, window, cx| {
                            crate::shell::open_settings(Section::Engine, window, cx)
                        }),
                ),
            );
        let puzzles = widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("Downloaded puzzles", palette))
            .child(crate::puzzle_db::card(true, cx, palette));
        let saved = widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("Saved games and studies", palette))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("offline-studies")
                            .outline()
                            .label("Your studies")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::Studies, window, cx)
                            }),
                    )
                    .child(
                        Button::new("offline-history")
                            .outline()
                            .label("Game history")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::History, window, cx)
                            }),
                    ),
            );
        v_flex()
            .gap(px(16.))
            .child(engine)
            .child(puzzles)
            .child(self.voice_model_card(cx, palette))
            .child(saved)
    }
}
