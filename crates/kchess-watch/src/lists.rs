//! The lists beside the watched board: TV's channels and the followed players playing now.

use super::*;

impl WatchPage {
    pub(super) fn channel_list(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let disabled = Self::playing_own_game(cx);
        let current = match &self.target {
            Some(Target::Channel(channel)) => Some(channel.clone()),
            _ => None,
        };
        let header = h_flex()
            .justify_between()
            .px(px(8.))
            .child(widgets::heading("Channels", palette))
            .child(
                Button::new("watch-channels-refresh")
                    .ghost()
                    .small()
                    .icon(icon("refresh-cw"))
                    .accessibility_label("Refresh channels")
                    .on_click(cx.listener(|this, _, _, cx| this.load_channels(cx))),
            );
        let failed = self
            .channels_error
            .clone()
            .filter(|_| self.channels.is_empty());
        list_card(palette)
            .child(header)
            .children(failed.map(|error| {
                div()
                    .px(px(8.))
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                v_flex()
                    .id("watch-channel-list")
                    .gap(px(2.))
                    .overflow_y_scroll()
                    .children(self.channels.iter().enumerate().map(|(index, channel)| {
                        let key = channel.key.clone();
                        let chosen = current.as_deref() == Some(key.as_str());
                        let sub = channel.player.as_ref().map_or("No game".into(), |player| {
                            player_label(player.title.as_deref(), &player.name, player.rating)
                        });
                        h_flex()
                            .id(("watch-channel", index))
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(channel.label.clone())
                            .aria_selected(chosen)
                            .gap(px(12.))
                            .p(px(8.))
                            .rounded(radii::MD)
                            .when(chosen, |row| {
                                row.bg(crate::theme::tint(palette.primary, 14.))
                            })
                            .when(disabled, |row| row.opacity(0.6))
                            .when(!disabled, |row| {
                                let key = key.clone();
                                row.cursor_pointer()
                                    .pressable(palette)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.watch_channel(key.clone(), cx)
                                    }))
                            })
                            .child(channel_icon(&key).size(px(20.)).flex_none().text_color(
                                if chosen {
                                    palette.primary
                                } else {
                                    palette.text_muted
                                },
                            ))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(channel.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .truncate()
                                            .child(sub),
                                    ),
                            )
                    })),
            )
    }

    pub(super) fn friends_list(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let disabled = Self::playing_own_game(cx);
        let friends = crate::accounts::handle(cx).read(cx).tracked();
        let online = friends
            .iter()
            .filter(|friend| {
                self.presence
                    .get(&friend.username.to_lowercase())
                    .is_some_and(|p| p.online)
            })
            .count();
        let playing: Vec<(String, Option<String>)> = friends
            .iter()
            .filter_map(|friend| {
                let name = friend.username.as_str();
                let presence = self.presence.get(&name.to_lowercase())?;
                presence
                    .playing
                    .then(|| (name.to_string(), presence.playing_id.clone()))
            })
            .collect();
        let header = h_flex()
            .justify_between()
            .items_start()
            .px(px(8.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::heading("Followed players playing now", palette))
                    .child(widgets::hint(
                        if friends.is_empty() {
                            "Follow players on the Following page".to_string()
                        } else {
                            format!("{online} of {} online", friends.len())
                        },
                        palette,
                    )),
            )
            .child(
                Button::new("watch-friends-refresh")
                    .ghost()
                    .small()
                    .size(px(24.))
                    .icon(icon("refresh-cw"))
                    .accessibility_label("Refresh followed players")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_presence(cx))),
            );
        list_card(palette)
            .child(header)
            .children(self.presence_error.clone().map(|error| {
                div()
                    .px(px(8.))
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(format!("Could not refresh followed players: {error}"))
            }))
            // Said only of players followed, and not while the refresh failed: unknown, not none.
            .when(
                playing.is_empty() && !friends.is_empty() && self.presence_error.is_none(),
                |card| {
                    card.child(
                        div()
                            .id("watch-friends-none")
                            .test_support()
                            .px(px(8.))
                            .child(widgets::hint(
                                "No followed player is playing right now",
                                palette,
                            )),
                    )
                },
            )
            .children(
                playing
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, game))| {
                        h_flex()
                            .gap(px(8.))
                            .p(px(8.))
                            .child(icon("swords").size(px(16.)).text_color(palette.text_muted))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        kchess_board::player_search::player_link(
                                            ("watch-friend", index),
                                            name,
                                            palette.primary,
                                        )
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::MEDIUM),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child("Playing now"),
                                    ),
                            )
                            .child(
                                Button::new(("watch-friend-game", index))
                                    .small()
                                    .primary()
                                    .label("Watch")
                                    .disabled(game.is_none() || disabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(id) = game.clone() {
                                            this.watch(Target::Game(id), cx);
                                        }
                                    })),
                            )
                    }),
            )
    }
}

/// The list beside the watched board (channels, followed players).
pub(super) fn list_card(palette: &Palette) -> Div {
    widgets::card(palette)
        .px(px(12.))
        .py(px(16.))
        .gap(px(8.))
        .w(px(240.))
        .flex_none()
}
