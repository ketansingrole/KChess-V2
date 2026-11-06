//! Settings → My accounts: the Lichess accounts on this
//! computer, connecting another, which one plays online, syncing, and disconnecting.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::SettingsPage;
use crate::accounts::{Accounts, AccountsHandle};
use crate::assets::icon;
use crate::theme::Palette;
use crate::widgets::{self, Look};

/// Ask before disconnecting one's own account or dropping a tracked one.
fn ask_remove(username: String, connected: bool, window: &mut Window, cx: &mut App) {
    let (title, label, text) = if connected {
        (
            "Disconnect your Lichess account?",
            "Disconnect",
            format!(
                "@{username} will be signed out and its synced games removed from this device. Your games on Lichess are not affected."
            ),
        )
    } else {
        (
            "Stop tracking this account?",
            "Stop tracking",
            format!("@{username} and its synced games will be removed from this device"),
        )
    };
    let handle = cx.global::<AccountsHandle>().0.downgrade();
    crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let (handle, username) = (handle.clone(), username.clone());
        dialog
            .title(title)
            .description(text.clone())
            .ok_text(label)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let username = username.clone();
                let _ = handle.update(cx, |accounts, cx| accounts.remove(username, window, cx));
                true
            })
    });
}

/// An account's initials, `@name` with its badges, and its muted lines (sync state, what it keeps).
fn identity(
    username: &str,
    badges: impl IntoIterator<Item = Option<Div>>,
    lines: impl IntoIterator<Item = String>,
    palette: &Palette,
) -> Div {
    let initials: String = username.chars().take(2).collect::<String>().to_uppercase();
    h_flex()
        .gap(px(12.))
        .min_w_0()
        .child(
            div()
                .size(px(36.))
                .flex_none()
                .rounded_full()
                .bg(palette.accented)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(initials),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap(px(2.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(format!("@{username}")),
                        )
                        .children(badges.into_iter().flatten()),
                )
                .children(lines.into_iter().map(|text| {
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(text)
                })),
        )
}

/// When an account last synced.
fn synced_line(account: &Value) -> String {
    account["lastSyncedAt"]
        .as_f64()
        .map(|at| format!("Last synced {}", crate::game::game_date(at)))
        .unwrap_or_else(|| "Not synced yet".into())
}

/// An account, what it keeps and downloaded, and what can be done with it.
fn account_row(
    account: &Value,
    accounts: &Accounts,
    several: bool,
    active: Option<&str>,
    palette: &Palette,
) -> AnyElement {
    let username = account["username"].as_str().unwrap_or_default().to_string();
    let connected = account["connected"] == json!(true);
    let plays = several && active == Some(username.as_str());
    let synced = synced_line(account);
    let (games, stored) = accounts.stored(&username);
    let usage = format!(
        "{} games · {} stored · {} downloaded",
        widgets::format_count(games),
        widgets::format_bytes(stored),
        widgets::format_bytes(accounts.downloaded(&username)),
    );
    let busy = accounts.busy;
    let act = {
        let handle_name = username.clone();
        move |run: fn(&mut Accounts, String, &mut Window, &mut Context<Accounts>)| {
            let name = handle_name.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let handle = cx.global::<AccountsHandle>().0.clone();
                handle.update(cx, |accounts, cx| run(accounts, name.clone(), window, cx));
            }
        }
    };
    let remove_name = username.clone();
    h_flex()
        .flex_wrap()
        .justify_between()
        .gap(px(12.))
        .py(px(14.))
        .border_t_1()
        .border_color(palette.border)
        .child(identity(
            &username,
            [
                Some(widgets::badge(
                    if connected { "Connected" } else { "Tracked" },
                    if connected {
                        Look::Success
                    } else {
                        Look::NeutralSoft
                    },
                    palette,
                )),
                plays.then(|| widgets::badge("Plays online", Look::Primary, palette)),
            ],
            [synced, usage],
            palette,
        ))
        .child(
            h_flex()
                .gap(px(8.))
                .when(several && connected && !plays, |row| {
                    row.child(
                        Button::new(SharedString::from(format!("account-activate-{username}")))
                            .ghost()
                            .small()
                            .icon(icon("gamepad-2"))
                            .label("Use for online play")
                            .disabled(busy)
                            .on_click(act(|accounts, name, _, cx| accounts.set_active(&name, cx))),
                    )
                })
                .child(
                    Button::new(SharedString::from(format!("account-sync-{username}")))
                        .outline()
                        .small()
                        .icon(icon("refresh-cw"))
                        .label("Sync")
                        .disabled(accounts.syncing())
                        .on_click(act(|accounts, name, window, cx| {
                            accounts.sync(Some(name), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("account-remove-{username}")))
                        .ghost()
                        .small()
                        .icon(if connected {
                            icon("unlink")
                        } else {
                            icon("user-minus")
                        })
                        .label(if connected {
                            "Disconnect"
                        } else {
                            "Stop tracking"
                        })
                        .text_color(palette.error)
                        .on_click(move |_, window, cx| {
                            ask_remove(remove_name.clone(), connected, window, cx)
                        }),
                ),
        )
        .into_any_element()
}

impl SettingsPage {
    pub(super) fn accounts(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let handle = cx.global::<AccountsHandle>().0.clone();
        let accounts = handle.read(cx);
        let connected = accounts.connected();
        let active = accounts.active(cx);
        let rows: Vec<Value> = accounts
            .all()
            .into_iter()
            .filter(|account| account["connected"] == json!(true))
            .collect();
        let several = connected.len() > 1;
        let target = handle.downgrade();
        widgets::card(palette)
            .gap(px(8.))
            .child(
                h_flex().justify_end().child(
                    Button::new("accounts-connect")
                        .when(connected.is_empty(), |button| button.primary())
                        .when(!connected.is_empty(), |button| button.outline())
                        .icon(icon("link"))
                        .label(if connected.is_empty() {
                            "Connect my account"
                        } else {
                            "Connect another account"
                        })
                        .loading(accounts.busy)
                        .disabled(accounts.busy)
                        .on_click(move |_, window, cx| {
                            let _ = target.update(cx, |accounts, cx| accounts.connect(window, cx));
                        }),
                ),
            )
            .when(!connected.is_empty(), |card| {
                card.child(widgets::hint(
                    "To add another account, sign in to it on lichess.org first",
                    palette,
                ))
            })
            .when(connected.is_empty(), |card| {
                card.child(widgets::empty_state(
                    icon("user-x"),
                    "No account connected",
                    None,
                    palette,
                ))
            })
            .children(
                rows.iter().map(|account| {
                    account_row(account, accounts, several, active.as_deref(), palette)
                }),
            )
    }

    /// Chess.com players whose games are synced: add one by name, sync, remove.
    pub(super) fn chesscom_accounts(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let accounts = cx.global::<AccountsHandle>().0.read(cx);
        let players = accounts.chesscom();
        let syncing = accounts.syncing();
        let typed = !self.chesscom_input.read(cx).value().trim().is_empty();
        let rows: Vec<Div> = players
            .iter()
            .map(|player| {
                let username = player["username"].as_str().unwrap_or_default().to_string();
                let (games, stored) = accounts.stored(&format!(
                    "{username}{}",
                    kchess_domain::chesscom::ACCOUNT_SUFFIX
                ));
                let usage = format!(
                    "{} games · {} stored",
                    widgets::format_count(games),
                    widgets::format_bytes(stored),
                );
                let (sync_name, remove_name) = (username.clone(), username.clone());
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .gap(px(12.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(identity(
                        &username,
                        [],
                        [synced_line(player), usage],
                        palette,
                    ))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new(SharedString::from(format!(
                                    "chesscom-sync-{username}"
                                )))
                                .outline()
                                .small()
                                .icon(icon("refresh-cw"))
                                .label("Sync")
                                .disabled(syncing)
                                .on_click(
                                    move |_, window, cx| {
                                        let name = sync_name.clone();
                                        crate::accounts::handle(cx).update(cx, |accounts, cx| {
                                            accounts.sync_chesscom(Some(name), window, cx)
                                        });
                                    },
                                ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "chesscom-remove-{username}"
                                )))
                                .ghost()
                                .small()
                                .icon(icon("user-minus"))
                                .label("Remove")
                                .text_color(palette.error)
                                .on_click(
                                    move |_, window, cx| {
                                        ask_remove_chesscom(remove_name.clone(), window, cx)
                                    },
                                ),
                            ),
                    )
            })
            .collect();
        widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading("Chess.com", palette))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        div().flex_1().min_w(px(160.)).max_w(px(352.)).child(
                            gpui_kit::component::input::Input::new(&self.chesscom_input)
                                .prefix(icon("user-plus").size(px(16.)))
                                .aria_label("Chess.com username"),
                        ),
                    )
                    .child(
                        Button::new("chesscom-add")
                            .when(players.is_empty(), |button| button.primary())
                            .when(!players.is_empty(), |button| button.outline())
                            .icon(icon("plus"))
                            .label("Sync games")
                            .tooltip("Downloads the player's public Chess.com games")
                            .loading(syncing && typed)
                            .disabled(!typed || syncing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_chesscom(window, cx)),
                            ),
                    ),
            )
            .children(rows)
    }

    /// Add the Chess.com player typed, and empty the field once they are added.
    pub(super) fn add_chesscom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.chesscom_input.read(cx).value().trim().to_string();
        if name.is_empty() || crate::accounts::handle(cx).read(cx).syncing() {
            return;
        }
        let input = self.chesscom_input.downgrade();
        crate::accounts::handle(cx).update(cx, |accounts, cx| {
            accounts.add_chesscom(name, window, cx, move |added, window, cx| {
                if added {
                    let _ = input.update(cx, |input, cx| input.set_value("", window, cx));
                }
            })
        });
    }
}

/// Ask before removing a Chess.com player and their games.
fn ask_remove_chesscom(username: String, window: &mut Window, cx: &mut App) {
    let handle = cx.global::<AccountsHandle>().0.downgrade();
    crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
        let (handle, username) = (handle.clone(), username.clone());
        dialog
            .title("Remove this Chess.com player?")
            .description(format!(
                "@{username} and their synced games will be removed from this device"
            ))
            .ok_text("Remove")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let username = username.clone();
                let _ = handle.update(cx, |accounts, cx| {
                    accounts.remove_chesscom(username, window, cx)
                });
                true
            })
    });
}
