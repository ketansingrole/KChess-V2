//! The game chat: docked at the bottom right like a messenger window, so it
//! never pushes the game around; the opponent's room, and spectators' when asked for.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use super::OnlinePage;
use crate::assets::icon;
use crate::online::{OnlineHandle, Phase};
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Look};

const OPEN_KEY: &str = "kchess:chat-open";
const SPECTATORS_KEY: &str = "kchess:chat-spectators";
/// Lichess's own quick messages.
const PRESETS: [&str; 7] = [
    "Hello",
    "Good luck",
    "Have fun!",
    "You too!",
    "Good game",
    "Well played",
    "Thank you",
];

impl OnlinePage {
    pub(super) fn chat_dock(
        &mut self,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<AnyElement> {
        let settings = cx
            .global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .clone();
        if !settings.core.online_chat || settings.frontend.zen_mode {
            return None;
        }
        let online = cx.global::<OnlineHandle>().0.read(cx);
        let open: bool = crate::prefs::get(cx, OPEN_KEY, false);
        let spectators: bool = crate::prefs::get(cx, SPECTATORS_KEY, false);
        let me = online.account().to_lowercase();
        let lines: Vec<serde_json::Value> = online.model()["chat"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|line| spectators || line["room"] == json!("player"))
            .cloned()
            .collect();
        if lines.len() < self.chat_read {
            self.chat_read = 0;
        }
        if open {
            self.chat_read = lines.len();
        }
        let unread = lines[self.chat_read.min(lines.len())..]
            .iter()
            .filter(|line| {
                line["user"]
                    .as_str()
                    .is_none_or(|user| user.to_lowercase() != me)
            })
            .count();
        let presets = matches!(online.phase, Phase::Playing | Phase::Finished);
        let typed = !self.chat_input.read(cx).value().trim().is_empty();
        let head = h_flex()
            .id("chat-toggle")
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label("Chat")
            .aria_expanded(open)
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| {
                crate::prefs::set(cx, OPEN_KEY, !open);
                cx.notify();
            }))
            .child(icon("message-circle").size(px(16.)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Chat"))
            .when(unread > 0, |head| {
                head.child(widgets::badge(unread.to_string(), Look::Primary, palette))
            })
            .child(div().flex_1())
            .when(open, |head| {
                head.child(
                    Button::new("chat-spectators")
                        .xsmall()
                        .when(spectators, |button| button.primary())
                        .when(!spectators, |button| button.ghost())
                        .icon(icon("users"))
                        .tooltip("Also show messages from people watching")
                        .accessibility_label("Spectator messages")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            crate::prefs::set(cx, SPECTATORS_KEY, !spectators);
                            cx.notify();
                        })),
                )
            })
            .child(
                if open {
                    icon("chevron-down")
                } else {
                    icon("chevron-up")
                }
                .size(px(16.)),
            );
        let body =
            open.then(|| {
                v_flex()
                    .gap(px(8.))
                    .px(px(12.))
                    .pb(px(12.))
                    .child(
                        v_flex()
                            .id("chat-log")
                            .h(px(200.))
                            .overflow_y_scroll()
                            .gap(px(4.))
                            .text_size(px(13.))
                            .when(lines.is_empty(), |log| {
                                log.child(widgets::hint("No messages.", palette))
                            })
                            .children(lines.iter().map(|line| {
                                let spectator = line["room"] != json!("player");
                                h_flex()
                                    .gap(px(6.))
                                    .when(spectator, |row| row.text_color(palette.text_muted))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(
                                        line["user"].as_str().unwrap_or_default().to_string(),
                                    ))
                                    .child(line["text"].as_str().unwrap_or_default().to_string())
                            })),
                    )
                    .when(presets, |body| {
                        body.child(h_flex().flex_wrap().gap(px(4.)).children(
                            PRESETS.iter().enumerate().map(|(index, preset)| {
                                Button::new(("chat-preset", index))
                                    .xsmall()
                                    .ghost()
                                    .label(*preset)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.send_chat(Some(preset), window, cx)
                                    }))
                            }),
                        ))
                    })
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.chat_input)
                                        .small()
                                        .aria_label("Chat message"),
                                ),
                            )
                            .child(
                                Button::new("chat-send")
                                    .small()
                                    .primary()
                                    .icon(icon("send"))
                                    .accessibility_label("Send")
                                    .disabled(!typed)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_chat(None, window, cx)
                                    })),
                            ),
                    )
            });
        Some(
            v_flex()
                .id("chat-dock")
                .occlude()
                .absolute()
                .right(px(16.))
                .bottom_0()
                .w(px(320.))
                .rounded_t(radii::XL)
                .border_1()
                .border_color(palette.border)
                .bg(palette.elevated)
                .shadow_lg()
                .child(head)
                .children(body)
                .into_any_element(),
        )
    }
}
