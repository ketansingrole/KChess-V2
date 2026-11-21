//! Writing to a player: the message dialog.

use super::*;

/// The message dialog: a private message from the active account.
pub struct MessageDialog {
    account: String,
    username: String,
    pub(super) text: Entity<TextareaState>,
    sending: Loading,
    /// The account was connected before messaging was asked for, and must connect again.
    reconnect: bool,
    _subscriptions: Vec<Subscription>,
}

impl MessageDialog {
    pub(super) fn new(
        account: String,
        username: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MessageDialog {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .placeholder("Write a message")
        });
        let subscriptions = vec![cx.observe(&text, |_, _, cx| cx.notify())];
        MessageDialog {
            account,
            username,
            text,
            sending: Loading::default(),
            reconnect: false,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).value().trim().to_string();
        if text.is_empty() || self.sending.busy() {
            return;
        }
        let turn = self.sending.start();
        // Each try says for itself whether the account must reconnect (as creating a
        // tournament does): an earlier banner stayed over a later, different failure.
        self.reconnect = false;
        cx.notify();
        let send = cx.global::<AppCore>().request(calls::SendMessage {
            account: Username(self.account.clone()),
            username: Username(self.username.clone()),
            text: MessageText(text),
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = send.await;
            this.update_in(cx, |this, window, cx| {
                if !this.sending.settle(turn, "players", &answer) {
                    return;
                }
                match answer {
                    Ok(Reply::NeedsReconnect) => this.reconnect = true,
                    Ok(Reply::Done(_)) => {
                        window.close_dialog(cx);
                        window.push_notification(format!("Message sent to {}", this.username), cx);
                    }
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for MessageDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let empty = self.text.read(cx).value().trim().is_empty();
        v_flex()
            .gap(px(12.))
            .child(widgets::hint(
                format!("Sent from @{} as a Lichess private message", self.account),
                &palette,
            ))
            .when(self.reconnect, |dialog| {
                dialog.child(
                    h_flex()
                        .gap(px(8.))
                        .p(px(12.))
                        .rounded(radii::MD)
                        .bg(crate::theme::tint(palette.warning, 10.))
                        .text_size(px(13.))
                        .child(icon("key-round").size(px(16.)).text_color(palette.warning))
                        .child(div().flex_1().child(format!(
                            "Lichess needs a new permission. Connect @{} again to allow sending messages.",
                            self.account
                        )))
                        .child(
                            Button::new("message-reconnect")
                                .small()
                                .outline()
                                .icon(icon("link"))
                                .label("Reconnect")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                    crate::accounts::handle(cx)
                                        .update(cx, |accounts, cx| accounts.connect(window, cx));
                                }),
                        ),
                )
            })
            .child(Textarea::new(&self.text).aria_label("Message"))
            .children(self.sending.error().cloned().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("message-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("message-send")
                            .primary()
                            .icon(icon("send"))
                            .label("Send")
                            .loading(self.sending.busy())
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    /// A reconnect banner from an earlier try stayed while the message was sent again.
    #[gpui_kit::test]
    fn sending_again_clears_the_reconnect_banner(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let reconnect = app.with_window(|window, cx| {
            let dialog =
                cx.new(|cx| super::MessageDialog::new("Alice".into(), "Bob".into(), window, cx));
            dialog.update(cx, |dialog, cx| {
                dialog.reconnect = true;
                dialog
                    .text
                    .update(cx, |text, cx| text.set_value("Good game!", window, cx));
                dialog.send(window, cx);
                dialog.reconnect
            })
        });
        assert!(!reconnect);
    }
}
