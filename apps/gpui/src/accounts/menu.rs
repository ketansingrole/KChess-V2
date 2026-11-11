//! The sidebar's account menu, in its footer beside Settings.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
use crate::theme::Palette;

/// What the menu's button shows: the sidebar is drawn again when one of these changes.
#[derive(PartialEq)]
pub struct MenuInputs {
    online: bool,
    busy: bool,
    active: Option<String>,
    connected: Vec<String>,
}

impl MenuInputs {
    pub fn of(cx: &App) -> MenuInputs {
        let accounts = super::handle(cx).read(cx);
        MenuInputs {
            online: crate::network::online(cx),
            busy: accounts.busy,
            active: accounts.active(cx),
            connected: accounts.connected(),
        }
    }
}

/// The sidebar's account menu: which account plays (with several), its profile, managing and
/// connecting accounts, What's new, reporting a problem, and logging out. With no account it is
/// named Connect Lichess and opens on connecting one.
pub fn menu_button(expanded: bool, cx: &App) -> AnyElement {
    let handle = crate::accounts::handle(cx);
    let accounts = handle.read(cx);
    let active = accounts.active(cx);
    let connected = accounts.connected();
    let can_connect = crate::network::online(cx) && !accounts.busy;
    let (glyph, label) = match &active {
        Some(active) => (icon("user"), format!("@{active}")),
        None => (icon("log-in"), "Connect Lichess".to_string()),
    };
    let target = handle.downgrade();
    // A navigation row: the label between the icon and a dimmed up-down chevron; elevated while
    // its menu is open.
    let palette = cx.global::<Palette>();
    let dimmed = palette.dimmed;
    Button::new("account-menu")
        .custom(crate::shell::ghost_sidebar(cx))
        .w_full()
        .px(px(6.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .when(!expanded, |row| row.justify_center())
                .child(glyph.size(px(18.)).flex_none())
                .when(expanded, |row| {
                    row.child(div().flex_1().min_w_0().truncate().child(label.clone()))
                        .child(
                            icon("chevrons-up-down")
                                .size(px(18.))
                                .flex_none()
                                .text_color(dimmed),
                        )
                }),
        )
        .accessibility_label(match &active {
            Some(_) => format!("{label} account menu"),
            None => label.clone(),
        })
        // On the rail the icon stands alone: name the account as the footer's other buttons do.
        .when(!expanded, |button| button.tooltip(label.clone()))
        // Above the button: open, from its left edge across the sidebar; on the rail, centred on
        // it. At least 192px wide, 8px clear.
        .dropdown_menu_with_anchor(
            if expanded {
                Anchor::BottomLeft
            } else {
                Anchor::BottomCenter
            },
            move |menu, _, _| {
                let menu = menu.min_w(if expanded {
                    crate::shell::SIDEBAR_WIDTH - px(33.)
                } else {
                    px(192.)
                });
                let act = |run: fn(
                    &mut crate::accounts::Accounts,
                    &mut Window,
                    &mut Context<crate::accounts::Accounts>,
                )| {
                    let target = target.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = target.update(cx, |accounts, cx| run(accounts, window, cx));
                    }
                };
                let mut menu = menu;
                // Several accounts: which one plays, then the account's pages, then signing out.
                // One account: the trigger already names it, so the menu opens on its pages.
                let several = connected.len() > 1;
                if let Some(active) = &active {
                    if several {
                        menu = menu.label("Play as");
                        for name in &connected {
                            let (name, target) = (name.clone(), target.clone());
                            menu = menu.item(
                                PopupMenuItem::new(format!("@{name}"))
                                    .checked(&name == active)
                                    .on_click(move |_, _, cx| {
                                        let _ = target.update(cx, |accounts, cx| {
                                            accounts.set_active(&name, cx)
                                        });
                                    }),
                            );
                        }
                        menu = menu.separator();
                    }
                    let profile = active.clone();
                    // The Settings button sits beside this one; the menu goes to its accounts.
                    menu = menu
                        .item(PopupMenuItem::new("Profile").icon(icon("user")).on_click(
                            move |_, window, cx| {
                                crate::pages::players::open_player(&profile, window, cx)
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Manage accounts")
                                .icon(icon("users"))
                                .on_click(|_, window, cx| {
                                    crate::shell::open_settings(
                                        crate::pages::settings::Section::Accounts,
                                        window,
                                        cx,
                                    )
                                }),
                        );
                }
                menu = menu
                    .item(
                        PopupMenuItem::new(if active.is_none() {
                            "Connect Lichess account"
                        } else {
                            "Connect another account"
                        })
                        .icon(icon("user-plus"))
                        .disabled(!can_connect)
                        .on_click(act(|accounts, window, cx| accounts.connect(window, cx))),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("What's new")
                            .icon(icon("rocket"))
                            .on_click(|_, window, cx| crate::whats_new::open(window, cx)),
                    )
                    .item(
                        PopupMenuItem::new("Report a problem")
                            .icon(icon("message-circle"))
                            .on_click(|_, _, cx| {
                                cx.open_url(crate::pages::settings::updates::ISSUES)
                            }),
                    );
                if let Some(active) = &active {
                    menu = menu.separator().item(
                        PopupMenuItem::new(if several {
                            format!("Log out @{active}")
                        } else {
                            "Log out".into()
                        })
                        .icon(icon("log-out"))
                        .on_click(act(|accounts, window, cx| {
                            accounts.logout(false, window, cx)
                        })),
                    );
                    if several {
                        menu = menu.item(
                            PopupMenuItem::new("Log out all accounts")
                                .icon(icon("log-out"))
                                .on_click(act(|accounts, window, cx| {
                                    accounts.logout(true, window, cx)
                                })),
                        );
                    }
                }
                menu
            },
        )
        .into_any_element()
}
