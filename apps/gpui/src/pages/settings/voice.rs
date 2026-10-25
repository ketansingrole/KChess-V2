//! Settings → Voice input (`SettingsVoice.vue`): the microphone's privacy settings, a short level
//! test, and how spoken moves are taken; the voice history follows (`voice_history`).

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Rows, SettingsPage};
use crate::assets::icon;
use crate::theme::Palette;
use crate::voice::capture::{self, Capture, CaptureError};
use kchess_contracts::types::Settings;

/// The system's microphone privacy settings, where there are any to open.
pub fn privacy_settings_url() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
    } else if cfg!(target_os = "windows") {
        Some("ms-settings:privacy-microphone")
    } else {
        None
    }
}

/// A running level check: the microphone stays open until it is stopped.
pub(super) struct MicTest {
    _capture: Capture,
    device: String,
    level: f32,
    _pump: Task<()>,
}

impl SettingsPage {
    /// Open the microphone and show its loudness, or close it again.
    pub(super) fn toggle_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.is_some() {
            return self.stop_mic_test(cx);
        }
        self.mic_error = None;
        match capture::start() {
            Ok((capture, mut chunks)) => {
                let pump = cx.spawn(async move |this, cx| {
                    while let Some(chunk) = chunks.next().await {
                        let alive = this
                            .update(cx, |this, cx| {
                                if let Some(test) = &mut this.mic_test {
                                    test.level = capture::level(&chunk);
                                    cx.notify();
                                }
                            })
                            .is_ok();
                        if !alive {
                            return;
                        }
                    }
                });
                self.mic_test = Some(MicTest {
                    device: capture.device.clone(),
                    _capture: capture,
                    level: 0.,
                    _pump: pump,
                });
            }
            Err(cause) => {
                self.mic_denied = cause == CaptureError::Denied;
                self.mic_error = Some(cause.message().into());
            }
        }
        cx.notify();
    }

    /// Release the microphone (leaving the section or the page).
    pub fn stop_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn voice(
        &self,
        settings: &Settings,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let (frontend, core) = (&settings.frontend, &settings.core);
        let test = self.mic_test.as_ref();
        let status: Option<SharedString> = match (&self.mic_error, test) {
            (Some(error), _) => Some(error.clone()),
            (None, Some(test)) if !test.device.is_empty() => {
                Some(format!("Speak now · {}", test.device).into())
            }
            (None, Some(_)) => Some("Speak now".into()),
            (None, None) => None,
        };
        let meter = test.map(|test| {
            div()
                .w(px(192.))
                .h(px(6.))
                .mt(px(6.))
                .rounded_full()
                .bg(palette.accented)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(palette.primary)
                        .w(relative(test.level.max(0.02))),
                )
        });
        let mut rows = Rows::new(self.narrow());
        if let Some(url) = privacy_settings_url() {
            rows = rows.row(
                "Microphone access",
                self.mic_denied.then(|| {
                    "Access was turned off. Allow KChess in your system privacy settings.".into()
                }),
                h_flex().child(
                    Button::new("voice-privacy")
                        .ghost()
                        .icon(icon("settings-2"))
                        .label("Privacy settings")
                        .on_click(move |_, _, cx| cx.open_url(url)),
                ),
                palette,
            );
        }
        rows.row_with(
            v_flex()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child("Test your microphone"),
                )
                .children(status.map(|status| {
                    div()
                        .text_size(px(12.))
                        .text_color(if self.mic_error.is_some() {
                            palette.error
                        } else {
                            palette.text_muted
                        })
                        .child(status)
                }))
                .children(meter),
            h_flex().child(
                Button::new("voice-mic-test")
                    .outline()
                    .icon(if test.is_some() {
                        icon("square")
                    } else {
                        icon("audio-lines")
                    })
                    .label(if test.is_some() { "Stop" } else { "Start test" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mic_test(cx))),
            ),
        )
        .switch(
            "voice-confirm",
            "Confirm spoken moves",
            Some("Say “confirm” before playing the move."),
            frontend.voice_confirm_moves,
            true,
            |s, on| s.frontend.voice_confirm_moves = on,
            palette,
        )
        .switch(
            "voice-push-to-talk",
            "Hold to speak",
            Some("Hold Space or the on-screen button."),
            frontend.voice_push_to_talk,
            true,
            |s, on| s.frontend.voice_push_to_talk = on,
            palette,
        )
        .switch(
            "voice-history",
            "Keep voice history",
            Some("Saves recognized text on this device; no audio."),
            core.voice_history,
            true,
            |s, on| s.core.voice_history = on,
            palette,
        )
        .card(palette)
        .when_some(self.voice_history.clone(), |column, history| {
            v_flex().gap(px(16.)).child(column).child(history)
        })
    }
}
