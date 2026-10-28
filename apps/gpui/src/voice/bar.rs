//! The compact voice bar: the Voice toggle, the level meter, one line
//! saying the most urgent thing (an error, what is being heard, the page's answer, what was heard),
//! the page's choices, and Hold to speak with push to talk.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ErrorKind, State};
use crate::assets::icon;
use crate::theme::Palette;

/// What a page puts in its bar.
pub struct Bar {
    pub id: &'static str,
    pub owner: u64,
    pub enabled: bool,
    /// The page takes phrases now (its turn, a question asked).
    pub active: bool,
    /// What to say, shown while listening.
    pub hint: SharedString,
    /// The page's answer to the last phrase.
    pub feedback: Option<SharedString>,
    /// What was heard last, kept until the next phrase.
    pub heard: Option<SharedString>,
    /// Choices the page offers (numbered moves), and Cancel.
    pub extra: Vec<AnyElement>,
}

/// The bar; `toggle` turns voice on or off for the page.
pub fn voice_bar(
    bar: Bar,
    toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let palette = *cx.global::<Palette>();
    let voice = super::handle(cx).read(cx);
    let mine = voice.owner() == Some(bar.owner) && bar.enabled;
    let state = if mine { voice.state } else { State::Off };
    let push = cx
        .global::<crate::settings::SettingsHandle>()
        .0
        .read(cx)
        .settings
        .frontend
        .voice_push_to_talk;
    let line: SharedString = match state {
        State::Error => voice.error.as_ref().map_or_else(
            || "Voice input unavailable".into(),
            |(text, _)| text.clone().into(),
        ),
        _ if !voice.partial.is_empty() => format!("Hearing “{}”", voice.partial).into(),
        _ if bar.feedback.is_some() => bar.feedback.clone().unwrap_or_default(),
        _ if bar.heard.is_some() => {
            format!("Heard “{}”", bar.heard.clone().unwrap_or_default()).into()
        }
        State::Loading => voice.preparing.clone().into(),
        _ if !bar.active => "Ready for your next turn or run".into(),
        _ if push => "Hold Space to speak".into(),
        _ => format!("Listening · {}", bar.hint).into(),
    };
    let denied = matches!(voice.error, Some((_, ErrorKind::Denied)));
    let enabled = bar.enabled;
    h_flex()
        .id(bar.id)
        .gap(px(8.))
        .min_w_0()
        .text_size(px(12.))
        .child(
            Button::new((bar.id, 0usize))
                .small()
                .when(enabled, |button| button.primary())
                .when(!enabled, |button| button.ghost())
                .icon(match state {
                    State::Loading => icon("loader-circle"),
                    State::Error => icon("mic-off"),
                    _ => icon("mic"),
                })
                .when(!enabled, |button| button.label("Voice"))
                .tooltip(if enabled {
                    "Turn off voice input"
                } else {
                    "Voice input: speak your moves"
                })
                .accessibility_label("Voice input")
                .on_click(move |_, window, cx| toggle(!enabled, window, cx)),
        )
        .when(mine, |row| {
            row.when(state == State::Listening, |row| {
                row.child(
                    div()
                        .w(px(36.))
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(palette.accented)
                        .child(
                            div()
                                .h_full()
                                .rounded(px(3.))
                                .bg(palette.primary)
                                .w(relative(voice.level.max(0.04))),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if state == State::Error {
                        palette.error
                    } else {
                        palette.text_muted
                    })
                    .child(line),
            )
            .children(bar.extra)
            .when(state == State::Error && denied, |row| {
                row.child(
                    Button::new((bar.id, 1usize))
                        .xsmall()
                        .outline()
                        .icon(icon("settings-2"))
                        .label("Privacy settings")
                        .on_click(|_, _, cx| {
                            if cfg!(target_os = "macos") {
                                cx.open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone");
                            } else if cfg!(target_os = "windows") {
                                cx.open_url("ms-settings:privacy-microphone");
                            }
                        }),
                )
            })
            .when(state == State::Error, |row| {
                row.child(
                    Button::new((bar.id, 2usize))
                        .xsmall()
                        .outline()
                        .icon(icon("rotate-ccw"))
                        .label("Try again")
                        .on_click(|_, _, cx| {
                            super::handle(cx).update(cx, |voice, cx| voice.retry(cx))
                        }),
                )
            })
            .when(push && bar.active && matches!(state, State::Paused | State::Listening), |row| {
                row.child(
                    div()
                        .id((bar.id, 3usize))
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette.border)
                        .when(state == State::Listening, |hold| hold.bg(palette.accented))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            super::handle(cx).update(cx, |voice, cx| voice.talk(true, cx))
                        })
                        .on_mouse_up(MouseButton::Left, |_, _, cx| {
                            super::handle(cx).update(cx, |voice, cx| voice.talk(false, cx))
                        })
                        .on_mouse_up_out(MouseButton::Left, |_, _, cx| {
                            super::handle(cx).update(cx, |voice, cx| voice.talk(false, cx))
                        })
                        .child("Hold to speak"),
                )
            })
        })
        .into_any_element()
}
