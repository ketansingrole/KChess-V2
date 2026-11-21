//! The compact voice bar: the Voice toggle, the level meter, one line
//! saying the most urgent thing (an error, what is being heard, the page's answer, what was heard),
//! the page's choices, and Hold to speak with push to talk.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ErrorKind, State};
use crate::theme::{Palette, radii};
use kchess_ui::icon;

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
    /// The line sits in the bar; else the page shows `voice_line` where it has the room.
    pub inline: bool,
    /// The bar sits at its row's end (beside the move box): the Voice button goes last, so what
    /// appears beside it once voice is on grows away from it rather than pushing it from under
    /// the pointer that turned it on.
    pub end: bool,
}

/// What the bar has to say while it is `bar.owner`'s, and whether that is an error.
fn status(bar: &Bar, cx: &App) -> Option<(SharedString, bool)> {
    let voice = super::handle(cx).read(cx);
    if voice.owner() != Some(bar.owner) || !bar.enabled {
        return None;
    }
    let push = crate::settings::frontend(cx).voice_push_to_talk;
    let line: SharedString = match voice.state {
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
    Some((line, voice.state == State::Error))
}

/// The bar's line on a row of its own, wrapping rather than cut off, with what recovers from
/// an error after it.
pub fn voice_line(bar: &Bar, cx: &App) -> Option<AnyElement> {
    let palette = *cx.global::<Palette>();
    let actions = recovery(bar.id, cx);
    status(bar, cx).map(|(line, error)| {
        h_flex()
            .gap(px(8.))
            .flex_wrap()
            .child(
                div()
                    .id((bar.id, 4usize))
                    .test_support()
                    .role(Role::Status)
                    .aria_label(line.clone())
                    .flex_1()
                    .min_w(px(160.))
                    .text_size(px(12.))
                    .text_color(if error {
                        palette.error
                    } else {
                        palette.text_muted
                    })
                    .child(line),
            )
            .children(actions)
            .into_any_element()
    })
}

/// Privacy settings (while the system blocks the microphone) and Try again, while voice
/// failed for the page.
fn recovery(id: &'static str, cx: &App) -> Vec<AnyElement> {
    let voice = super::handle(cx).read(cx);
    if voice.state != State::Error {
        return Vec::new();
    }
    let denied = matches!(voice.error, Some((_, ErrorKind::Denied)));
    let privacy = crate::microphone::privacy_settings(cx).filter(|_| denied);
    let mut actions = Vec::new();
    if let Some(url) = privacy {
        actions.push(
            Button::new((id, 1usize))
                .small()
                .outline()
                .icon(icon("settings-2"))
                .label("Privacy settings")
                .on_click(move |_, _, cx| cx.open_url(url))
                .into_any_element(),
        );
    }
    actions.push(
        Button::new((id, 2usize))
            .small()
            .outline()
            .icon(icon("rotate-ccw"))
            .label("Try again")
            .on_click(|_, _, cx| super::handle(cx).update(cx, |voice, cx| voice.retry(cx)))
            .into_any_element(),
    );
    actions
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
    let push = crate::settings::frontend(cx).voice_push_to_talk;
    let line = status(&bar, cx).filter(|_| bar.inline);
    let enabled = bar.enabled;
    let end = bar.end;
    let mut toggle = Some(
        Button::new((bar.id, 0usize))
            .small()
            .when(enabled, |button| button.primary())
            .when(!enabled, |button| button.ghost())
            // The speech model loading: a spinner turns, and the button still turns voice off.
            .map(|button| match state {
                State::Loading => button.icon(crate::widgets::spinner(px(14.))),
                State::Error => button.icon(icon("mic-off")),
                _ => button.icon(icon("mic")),
            })
            // Named "Voice" on and off alike: dropped to its icon once on, the button shrank
            // 68 px away from the pointer that turned it on, and the next click missed it.
            .label("Voice")
            .tooltip(if enabled {
                "Turn off voice input"
            } else {
                "Voice input: speak your moves"
            })
            .accessibility_label("Voice input")
            .on_click(move |_, window, cx| toggle(!enabled, window, cx)),
    );
    h_flex()
        .id(bar.id)
        .gap(px(8.))
        .min_w_0()
        .text_size(px(12.))
        .when(!end, |row| row.children(toggle.take()))
        .when(mine, |row| {
            row.when(state == State::Listening, |row| {
                row.child(
                    div()
                        .w(px(36.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(palette.accented)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(palette.primary)
                                .w(relative(voice.level.max(0.04))),
                        ),
                )
            })
            .children(line.map(|(line, error)| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if error {
                        palette.error
                    } else {
                        palette.text_muted
                    })
                    .child(line)
            }))
            .children(bar.extra)
            // Beside the move box, the line and what recovers from an error go under the
            // board instead (`voice_line`): in the line's 60% they pushed the bar past its end.
            .when(bar.inline, |row| row.children(recovery(bar.id, cx)))
            .when(
                push && bar.active && matches!(state, State::Paused | State::Listening),
                |row| {
                    row.child(
                        div()
                            .id((bar.id, 3usize))
                            .px(px(12.))
                            .py(px(4.))
                            .rounded(radii::SM)
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
                },
            )
        })
        .children(toggle)
        .into_any_element()
}
