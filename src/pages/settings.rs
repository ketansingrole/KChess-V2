use gpui::{
    Context, Entity, IntoElement, MouseButton, Render, Window, div, prelude::*, px, rgb, rgba,
};
use kchess_board::ChessBoardView;

use crate::{
    lichess_state::LichessState,
    search_input::SearchInput,
    storage::{
        self, BoardLooksSettings, LichessAccount, THEME_PRESET_CHESS_COM, THEME_PRESET_CUSTOM,
        THEME_PRESET_LICHESS,
    },
};

pub struct SettingsPage {
    board_view: Entity<ChessBoardView>,
    lichess_state: Entity<LichessState>,
    light_input: Entity<SearchInput>,
    dark_input: Entity<SearchInput>,
    account_input: Entity<SearchInput>,
    last_light_value: String,
    last_dark_value: String,
    current_preset: String,
    light_error: Option<String>,
    dark_error: Option<String>,
    account_error: Option<String>,
    persistence_error: Option<String>,
}

impl SettingsPage {
    pub fn new(
        cx: &mut Context<Self>,
        board_view: Entity<ChessBoardView>,
        lichess_state: Entity<LichessState>,
        initial: BoardLooksSettings,
    ) -> Self {
        let light_input = cx.new(SearchInput::new);
        let dark_input = cx.new(SearchInput::new);
        let account_input = cx.new(SearchInput::new);

        light_input.update(cx, |input, cx| {
            input.set_placeholder("#f0d9b5", cx);
            input.set_text(&initial.light_square_hex, cx);
        });

        dark_input.update(cx, |input, cx| {
            input.set_placeholder("#b58863", cx);
            input.set_text(&initial.dark_square_hex, cx);
        });

        account_input.update(cx, |input, cx| {
            input.set_placeholder("lichess username", cx);
        });

        Self {
            board_view,
            lichess_state,
            light_input,
            dark_input,
            account_input,
            last_light_value: initial.light_square_hex,
            last_dark_value: initial.dark_square_hex,
            current_preset: initial.theme_preset,
            light_error: None,
            dark_error: None,
            account_error: None,
            persistence_error: None,
        }
    }

    fn process_input_changes(&mut self, cx: &mut Context<Self>) {
        let light_value = self.light_input.read(cx).text();
        let dark_value = self.dark_input.read(cx).text();

        if light_value == self.last_light_value && dark_value == self.last_dark_value {
            return;
        }

        self.last_light_value = light_value.clone();
        self.last_dark_value = dark_value.clone();

        let normalized_light = storage::normalize_hex(&light_value);
        let normalized_dark = storage::normalize_hex(&dark_value);

        self.light_error = normalized_light
            .is_none()
            .then_some("Use #RRGGBB.".to_string());
        self.dark_error = normalized_dark
            .is_none()
            .then_some("Use #RRGGBB.".to_string());

        let (Some(light_hex), Some(dark_hex)) = (normalized_light, normalized_dark) else {
            return;
        };

        let Some(light_square) = storage::parse_hex_color(&light_hex) else {
            return;
        };
        let Some(dark_square) = storage::parse_hex_color(&dark_hex) else {
            return;
        };

        let preset = storage::resolve_theme_preset(&light_hex, &dark_hex);

        self.board_view.update(cx, |board, cx| {
            let mut theme = board.theme();
            theme.light_square = light_square;
            theme.dark_square = dark_square;
            board.set_theme(theme);
            cx.notify();
        });

        let settings = BoardLooksSettings::new(light_hex, dark_hex, preset.clone());
        match storage::save_board_looks_settings(&settings) {
            Ok(()) => {
                self.current_preset = preset;
                self.persistence_error = None;
            }
            Err(err) => {
                self.persistence_error = Some(err.to_string());
            }
        }
    }

    fn apply_preset(&mut self, preset: &str, cx: &mut Context<Self>) {
        let Some((light, dark)) = storage::preset_colors(preset) else {
            return;
        };

        self.light_input.update(cx, |input, cx| {
            input.set_text(light, cx);
        });
        self.dark_input.update(cx, |input, cx| {
            input.set_text(dark, cx);
        });
        self.light_error = None;
        self.dark_error = None;
        self.persistence_error = None;
    }

    fn add_lichess_account(&mut self, cx: &mut Context<Self>) {
        let username = self.account_input.read(cx).text();
        match self
            .lichess_state
            .update(cx, |state, cx| state.add_account(&username, cx))
        {
            Ok(_) => {
                self.account_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.account_error = None;
            }
            Err(err) => self.account_error = Some(err),
        }
    }

    fn remove_lichess_account(&mut self, username: &str, cx: &mut Context<Self>) {
        match self
            .lichess_state
            .update(cx, |state, cx| state.remove_account(username, cx))
        {
            Ok(()) => {
                self.account_error = None;
            }
            Err(err) => self.account_error = Some(err),
        }
    }
}

fn color_swatch(value: &str, is_invalid: bool) -> impl IntoElement {
    let color = storage::parse_hex_color(value).unwrap_or_else(|| rgb(0xe2e8f0));

    div()
        .w(px(28.0))
        .h(px(28.0))
        .rounded_md()
        .border_1()
        .border_color(if is_invalid {
            rgba(0xdc2626cc)
        } else {
            rgba(0x0f172a22)
        })
        .bg(color)
}

fn input_shell(input: Entity<SearchInput>) -> impl IntoElement {
    let input_for_focus = input.clone();
    div()
        .h(px(32.0))
        .flex_1()
        .px_3()
        .rounded_md()
        .bg(rgba(0xffffffff))
        .border_1()
        .border_color(rgba(0xcbd5e1ff))
        .focus(|style| style.border_color(rgb(0x3f7fe5)))
        .cursor_text()
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            let handle = input_for_focus.read(app).keyboard_focus_handle();
            window.focus(&handle);
        })
        .flex()
        .items_center()
        .child(input)
}

fn field_row(
    label: &'static str,
    value: &str,
    input: Entity<SearchInput>,
    is_invalid: bool,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().text_color(rgb(0x334155)).child(label))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(color_swatch(value, is_invalid))
                .child(input_shell(input)),
        )
}

fn preset_label(preset: &str) -> &'static str {
    match preset {
        THEME_PRESET_LICHESS => "Lichess",
        THEME_PRESET_CHESS_COM => "Chess.com",
        THEME_PRESET_CUSTOM => "Custom",
        _ => "Custom",
    }
}

fn preset_button(
    title: &'static str,
    description: &'static str,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgba(0xcbd5e1ff))
        .bg(rgba(0xffffffff))
        .cursor_pointer()
        .hover(|this| this.bg(rgba(0xf8fafcff)))
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app)
        })
        .child(div().text_sm().text_color(rgb(0x0f172a)).child(title))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x64748b))
                .whitespace_normal()
                .child(description),
        )
}

fn action_button(
    label: &'static str,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(if disabled {
            rgba(0xcbd5e166)
        } else {
            rgba(0x3f7fe544)
        })
        .bg(if disabled {
            rgba(0xf8fafcff)
        } else {
            rgba(0xe8f0feff)
        })
        .text_sm()
        .text_color(if disabled {
            rgb(0x94a3b8)
        } else {
            rgb(0x1d4ed8)
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xdbeafeff)))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .child(label)
}

fn danger_button(
    label: &'static str,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(if disabled {
            rgba(0xfecaca88)
        } else {
            rgba(0xf8717188)
        })
        .bg(if disabled {
            rgba(0xfffbebff)
        } else {
            rgba(0xfef2f2ff)
        })
        .text_sm()
        .text_color(if disabled {
            rgb(0xfca5a5)
        } else {
            rgb(0xb91c1c)
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xfee2e2ff)))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .child(label)
}

fn section_card(
    title: &'static str,
    description: &'static str,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .w_full()
        .max_w(px(720.0))
        .flex()
        .flex_col()
        .gap_4()
        .px_4()
        .py_4()
        .rounded_xl()
        .bg(rgba(0xffffffed))
        .border_1()
        .border_color(rgba(0xd6dae1ff))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_base().text_color(rgb(0x0f172a)).child(title))
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(rgb(0x64748b))
                        .whitespace_normal()
                        .child(description),
                ),
        )
        .child(body)
}

fn account_row(
    account: &LichessAccount,
    removing: bool,
    on_remove: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let sync_label = account
        .last_synced_at
        .map(|_| "Synced")
        .unwrap_or("Not synced yet");
    let auth_label = if account.auth_kind == storage::LICHESS_AUTH_KIND_OAUTH {
        "Connected"
    } else {
        "Tracked"
    };

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3()
        .py_3()
        .rounded_lg()
        .bg(rgba(0xf8fafcff))
        .border_1()
        .border_color(rgba(0xe2e8f0ff))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x0f172a))
                        .child(format!("@{}", account.username)),
                )
                .child(div().text_xs().text_color(rgb(0x64748b)).child(sync_label)),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(rgba(0xe2e8f0aa))
                        .text_xs()
                        .text_color(rgb(0x475569))
                        .child(auth_label),
                )
                .child(danger_button("Remove", removing, on_remove)),
        )
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_input_changes(cx);

        let current_light = self.light_input.read(cx).text();
        let current_dark = self.dark_input.read(cx).text();
        let account_input_value = self.account_input.read(cx).text();
        let preset_name = preset_label(&self.current_preset);
        let (
            tracked_accounts,
            own_accounts,
            connecting_account,
            syncing_accounts,
            sync_status,
            sync_error,
        ) = {
            let lichess_state = self.lichess_state.read(cx);
            (
                lichess_state.tracked_accounts(),
                lichess_state.own_accounts(),
                lichess_state.connecting(),
                lichess_state.syncing(),
                lichess_state.status_message().map(ToOwned::to_owned),
                lichess_state.error_message().map(ToOwned::to_owned),
            )
        };

        let view_for_lichess = cx.entity();
        let view_for_chess_com = cx.entity();
        let view_for_add_account = cx.entity();
        let view_for_sync = cx.entity();
        let view_for_connect = cx.entity();
        let total_accounts = tracked_accounts.len() + own_accounts.len();
        let narrow_layout = f32::from(window.bounds().size.width) < 1120.0;

        div().size_full().min_h(px(0.0)).child(
            div().size_full().min_h(px(0.0)).flex().flex_col().child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .id("settings-scroll")
                    .overflow_y_scroll()
                    .scrollbar_width(px(10.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .p_6()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(760.0))
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(rgb(0x1f2937))
                                            .text_xl()
                                            .child("Board Looks"),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(0x64748b))
                                            .text_sm()
                                            .whitespace_normal()
                                            .child(
                                                "Enter board colors directly as hex values.\nThe swatch updates as soon as the value is valid.",
                                            ),
                                    ),
                            )
                            .child(
                            section_card(
                                "Board theme",
                                "Choose a preset or fine-tune the square colors manually.",
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(field_row(
                                        "White squares",
                                        &current_light,
                                        self.light_input.clone(),
                                        self.light_error.is_some(),
                                    ))
                                    .when(self.light_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0xdc2626))
                                                .child(self.light_error.clone().unwrap_or_default()),
                                        )
                                    })
                                    .child(field_row(
                                        "Black squares",
                                        &current_dark,
                                        self.dark_input.clone(),
                                        self.dark_error.is_some(),
                                    ))
                                    .when(self.dark_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0xdc2626))
                                                .child(self.dark_error.clone().unwrap_or_default()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .gap_3()
                                            .when(narrow_layout, |this| {
                                                this.flex_col().items_start()
                                            })
                                            .when(!narrow_layout, |this| {
                                                this.items_center().justify_between()
                                            })
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgb(0x334155))
                                                    .child("Preset theme"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .bg(rgba(0xe2e8f055))
                                                    .text_xs()
                                                    .text_color(rgb(0x475569))
                                                    .child(format!("Current: {preset_name}")),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .grid()
                                            .grid_cols(if narrow_layout { 1 } else { 2 })
                                            .gap_3()
                                            .child(preset_button(
                                                "Lichess",
                                                "Classic Lichess board colors",
                                                move |_, app| {
                                                    view_for_lichess.update(app, |view, cx| {
                                                        view.apply_preset(THEME_PRESET_LICHESS, cx);
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(preset_button(
                                                "Chess.com",
                                                "Apply Chess.com board colors",
                                                move |_, app| {
                                                    view_for_chess_com.update(app, |view, cx| {
                                                        view.apply_preset(THEME_PRESET_CHESS_COM, cx);
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .when(self.persistence_error.is_some(), |this| {
                                        this.child(
                                            div().text_xs().text_color(rgb(0xb91c1c)).child(
                                                self.persistence_error
                                                    .clone()
                                                    .unwrap_or_else(|| "Failed to save settings".to_string()),
                                            ),
                                        )
                                    }),
                            ),
                        )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(rgb(0x1f2937))
                                            .text_xl()
                                            .child("Lichess"),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(0x64748b))
                                            .text_sm()
                                            .whitespace_normal()
                                            .child(
                                                "Keep public profile tracking separate from your own connected Lichess login,\nso the purpose of each account is obvious.",
                                            ),
                                    ),
                            )
                            .when(sync_status.is_some(), |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .max_w(px(720.0))
                                        .child(
                                            div()
                                                .px_3()
                                                .py_2()
                                                .rounded_md()
                                                .bg(rgba(0xe0f2feaa))
                                                .text_sm()
                                                .text_color(rgb(0x0369a1))
                                                .child(sync_status.clone().unwrap_or_default()),
                                        ),
                                )
                            })
                            .when(sync_error.is_some(), |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .max_w(px(720.0))
                                        .child(
                                            div()
                                                .px_3()
                                                .py_2()
                                                .rounded_md()
                                                .bg(rgba(0xfee2e2aa))
                                                .text_sm()
                                                .text_color(rgb(0xb91c1c))
                                                .child(sync_error.clone().unwrap_or_default()),
                                        ),
                                )
                            })
                            .child(
                            section_card(
                                "Connect your own account",
                                "Use browser login for the Lichess account that you own.\nThis path can later support online play from inside KChess.",
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        div()
                                            .flex()
                                            .gap_3()
                                            .when(narrow_layout, |this| {
                                                this.flex_col().items_start()
                                            })
                                            .when(!narrow_layout, |this| {
                                                this.items_center().justify_between()
                                            })
                                            .child(
                                                div()
                                                    .when(!narrow_layout, |this| {
                                                        this.flex_1().min_w(px(0.0))
                                                    })
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(0x334155))
                                                            .child("Connected accounts"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(0x64748b))
                                                            .whitespace_normal()
                                                            .child("Connect only the account you control."),
                                                    ),
                                            )
                                            .child(action_button(
                                                "Connect with Lichess",
                                                connecting_account,
                                                move |_, app| {
                                                    view_for_connect.update(app, |view, cx| {
                                                        view.lichess_state.update(cx, |state, cx| {
                                                            state.start_connect_flow(cx)
                                                        });
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .when(own_accounts.is_empty(), |this| {
                                                this.child(
                                                    div()
                                                        .rounded_lg()
                                                        .border_1()
                                                        .border_color(rgba(0xe2e8f0ff))
                                                        .bg(rgba(0xf8fafcff))
                                                        .px_3()
                                                        .py_3()
                                                        .text_sm()
                                                        .text_color(rgb(0x64748b))
                                                        .child("No connected account yet."),
                                                )
                                            })
                                            .children(own_accounts.iter().map(|account| {
                                                let username = account.username.clone();
                                                let view = cx.entity();
                                                let removing = self
                                                    .lichess_state
                                                    .read(cx)
                                                    .removing(&username);

                                                account_row(account, removing, move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.remove_lichess_account(&username, cx);
                                                        cx.notify();
                                                    });
                                                })
                                            })),
                                    ),
                            ),
                        )
                            .child(
                            section_card(
                                "Track any public profile",
                                "Download and keep syncing games for any Lichess username,\neven if it is not your account. This is only for public game history.",
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        div()
                                            .flex()
                                            .gap_3()
                                            .when(narrow_layout, |this| {
                                                this.flex_col().items_start()
                                            })
                                            .when(!narrow_layout, |this| this.items_end())
                                            .child(
                                                div()
                                                    .when(!narrow_layout, |this| this.flex_1())
                                                    .flex()
                                                    .flex_col()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(0x334155))
                                                            .child("Public Lichess username"),
                                                    )
                                                    .child(input_shell(self.account_input.clone())),
                                            )
                                            .child(action_button(
                                                "Track profile",
                                                account_input_value.trim().is_empty(),
                                                move |_, app| {
                                                    view_for_add_account.update(app, |view, cx| {
                                                        view.add_lichess_account(cx);
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .when(self.account_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0xb91c1c))
                                                .child(self.account_error.clone().unwrap_or_default()),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .gap_3()
                                            .when(narrow_layout, |this| {
                                                this.flex_col().items_start()
                                            })
                                            .when(!narrow_layout, |this| {
                                                this.items_center().justify_between()
                                            })
                                            .child(
                                                div()
                                                    .when(!narrow_layout, |this| {
                                                        this.flex_1().min_w(px(0.0))
                                                    })
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(0x334155))
                                                            .child("Tracked public profiles"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_full()
                                                            .bg(rgba(0xe2e8f055))
                                                            .text_xs()
                                                            .text_color(rgb(0x475569))
                                                            .child(format!(
                                                                "{} tracked · {} total accounts",
                                                                tracked_accounts.len(),
                                                                total_accounts
                                                            )),
                                                    ),
                                            )
                                            .child(action_button(
                                                "Sync all",
                                                syncing_accounts,
                                                move |_, app| {
                                                    view_for_sync.update(app, |view, cx| {
                                                        view.lichess_state.update(cx, |state, cx| {
                                                            state.sync_all(cx)
                                                        });
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .when(tracked_accounts.is_empty(), |this| {
                                                this.child(
                                                    div()
                                                        .rounded_lg()
                                                        .border_1()
                                                        .border_color(rgba(0xe2e8f0ff))
                                                        .bg(rgba(0xf8fafcff))
                                                        .px_3()
                                                        .py_3()
                                                        .text_sm()
                                                        .text_color(rgb(0x64748b))
                                                        .child("No public profiles tracked yet."),
                                                )
                                            })
                                            .children(tracked_accounts.iter().map(|account| {
                                                let username = account.username.clone();
                                                let view = cx.entity();
                                                let removing = self
                                                    .lichess_state
                                                    .read(cx)
                                                    .removing(&username);

                                                account_row(account, removing, move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.remove_lichess_account(&username, cx);
                                                        cx.notify();
                                                    });
                                                })
                                            })),
                                    ),
                            ),
                            )
                            .child(div().h(px(28.0))),
                    ),
            ),
        )
    }
}
