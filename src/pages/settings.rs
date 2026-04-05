use gpui::{Context, Entity, IntoElement, MouseButton, Render, Window, div, prelude::*, px};
use kchess_board::ChessBoardView;

use crate::{
    engine::install,
    engine_state::{EngineInstallStatus, EngineSourceMode, EngineState},
    lichess_state::LichessState,
    search_input::SearchInput,
    storage::{
        self, BoardLooksSettings, LichessAccount, THEME_PRESET_CHESS_COM, THEME_PRESET_CUSTOM,
        THEME_PRESET_LICHESS,
    },
    theme::{AppearanceMode, ThemePalette, ThemeState},
    ui::primitives::{action_button, danger_button, section_card},
};

#[derive(Clone, Debug)]
struct SettingsFormState {
    last_light_value: String,
    last_dark_value: String,
    current_preset: String,
    light_error: Option<String>,
    dark_error: Option<String>,
    account_error: Option<String>,
    engine_path_error: Option<String>,
    persistence_error: Option<String>,
    appearance_persistence_error: Option<String>,
}

impl SettingsFormState {
    fn new(initial: &BoardLooksSettings) -> Self {
        Self {
            last_light_value: initial.light_square_hex.clone(),
            last_dark_value: initial.dark_square_hex.clone(),
            current_preset: initial.theme_preset.clone(),
            light_error: None,
            dark_error: None,
            account_error: None,
            engine_path_error: None,
            persistence_error: None,
            appearance_persistence_error: None,
        }
    }
}

pub struct SettingsPage {
    board_view: Entity<ChessBoardView>,
    lichess_state: Entity<LichessState>,
    engine_state: Entity<EngineState>,
    theme_state: Entity<ThemeState>,
    light_input: Entity<SearchInput>,
    dark_input: Entity<SearchInput>,
    account_input: Entity<SearchInput>,
    engine_path_input: Entity<SearchInput>,
    form: SettingsFormState,
}

impl SettingsPage {
    pub fn new(
        cx: &mut Context<Self>,
        board_view: Entity<ChessBoardView>,
        lichess_state: Entity<LichessState>,
        engine_state: Entity<EngineState>,
        theme_state: Entity<ThemeState>,
        initial: BoardLooksSettings,
    ) -> Self {
        let light_input = cx.new(SearchInput::new);
        let dark_input = cx.new(SearchInput::new);
        let account_input = cx.new(SearchInput::new);
        let engine_path_input = cx.new(SearchInput::new);

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
        engine_path_input.update(cx, |input, cx| {
            input.set_placeholder("custom stockfish binary path (optional)", cx);
        });

        light_input.update(cx, |input, cx| {
            input.set_theme_state(theme_state.clone(), cx);
        });
        dark_input.update(cx, |input, cx| {
            input.set_theme_state(theme_state.clone(), cx);
        });
        account_input.update(cx, |input, cx| {
            input.set_theme_state(theme_state.clone(), cx);
        });
        engine_path_input.update(cx, |input, cx| {
            input.set_theme_state(theme_state.clone(), cx);
        });
        let initial_custom_path = engine_state
            .read(cx)
            .custom_path()
            .unwrap_or_default()
            .to_string();
        engine_path_input.update(cx, |input, cx| {
            input.set_text(&initial_custom_path, cx);
        });

        Self {
            board_view,
            lichess_state,
            engine_state,
            theme_state,
            light_input,
            dark_input,
            account_input,
            engine_path_input,
            form: SettingsFormState::new(&initial),
        }
    }

    fn process_input_changes(&mut self, cx: &mut Context<Self>) {
        let light_value = self.light_input.read(cx).text();
        let dark_value = self.dark_input.read(cx).text();

        if light_value == self.form.last_light_value && dark_value == self.form.last_dark_value {
            return;
        }

        self.form.last_light_value = light_value.clone();
        self.form.last_dark_value = dark_value.clone();

        let normalized_light = storage::normalize_hex(&light_value);
        let normalized_dark = storage::normalize_hex(&dark_value);

        self.form.light_error = normalized_light
            .is_none()
            .then_some("Use #RRGGBB.".to_string());
        self.form.dark_error = normalized_dark
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
                self.form.current_preset = preset;
                self.form.persistence_error = None;
            }
            Err(err) => {
                self.form.persistence_error = Some(err.to_string());
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
        self.form.light_error = None;
        self.form.dark_error = None;
        self.form.persistence_error = None;
    }

    fn set_appearance_mode(
        &mut self,
        mode: AppearanceMode,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let window_appearance = window.appearance();
        self.theme_state.update(cx, |theme, cx| {
            theme.set_mode(mode, window_appearance, cx);
        });

        match storage::save_app_appearance_mode(mode) {
            Ok(()) => {
                self.form.appearance_persistence_error = None;
            }
            Err(err) => {
                self.form.appearance_persistence_error = Some(err.to_string());
            }
        }
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
                self.form.account_error = None;
            }
            Err(err) => self.form.account_error = Some(err),
        }
    }

    fn remove_lichess_account(&mut self, username: &str, cx: &mut Context<Self>) {
        match self
            .lichess_state
            .update(cx, |state, cx| state.remove_account(username, cx))
        {
            Ok(()) => {
                self.form.account_error = None;
            }
            Err(err) => self.form.account_error = Some(err),
        }
    }

    fn install_or_update_engine(&mut self, cx: &mut Context<Self>) {
        self.engine_state
            .update(cx, |state, cx| state.install_or_update(cx));
    }

    fn save_custom_engine_path(&mut self, cx: &mut Context<Self>) {
        let value = self.engine_path_input.read(cx).text();
        self.engine_state.update(cx, |state, cx| {
            state.set_custom_path(Some(value), cx);
        });
        self.form.engine_path_error = None;
    }

    fn clear_custom_engine_path(&mut self, cx: &mut Context<Self>) {
        self.engine_state.update(cx, |state, cx| {
            state.clear_custom_path(cx);
        });
        self.engine_path_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.form.engine_path_error = None;
    }

    fn open_stockfish_download_page(&self, cx: &mut Context<Self>) {
        cx.open_url(install::STOCKFISH_DOWNLOAD_PAGE);
    }
}

fn color_swatch(value: &str, is_invalid: bool, palette: ThemePalette) -> impl IntoElement {
    let color = storage::parse_hex_color(value).unwrap_or(palette.surface_alt);

    div()
        .w(px(28.0))
        .h(px(28.0))
        .rounded_md()
        .border_1()
        .border_color(if is_invalid {
            palette.status_error_text
        } else {
            palette.border_muted
        })
        .bg(color)
}

fn input_shell(input: Entity<SearchInput>, palette: ThemePalette) -> impl IntoElement {
    let input_for_focus = input.clone();
    div()
        .h(px(32.0))
        .flex_1()
        .min_w(px(0.0))
        .px_3()
        .rounded_md()
        .bg(palette.input_bg)
        .border_1()
        .border_color(palette.input_border)
        .focus(|style| style.border_color(palette.input_focus_border))
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
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(color_swatch(value, is_invalid, palette))
                .child(input_shell(input, palette)),
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
    palette: ThemePalette,
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
        .border_color(palette.input_border)
        .bg(palette.input_bg)
        .cursor_pointer()
        .hover(|this| this.bg(palette.surface_hover))
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app)
        })
        .child(
            div()
                .text_sm()
                .text_color(palette.text_primary)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.text_muted)
                .whitespace_normal()
                .child(description),
        )
}

fn account_row(
    account: &LichessAccount,
    removing: bool,
    palette: ThemePalette,
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
        .bg(palette.surface_alt)
        .border_1()
        .border_color(palette.border_muted)
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
                        .text_color(palette.text_primary)
                        .child(format!("@{}", account.username)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child(sync_label),
                ),
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
                        .bg(palette.surface_hover)
                        .text_xs()
                        .text_color(palette.text_secondary)
                        .child(auth_label),
                )
                .child(danger_button("Remove", removing, palette, on_remove)),
        )
}

fn appearance_mode_segment(
    title: &'static str,
    selected: bool,
    palette: ThemePalette,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .flex_1()
        .h(px(28.0))
        .px_3()
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .cursor_pointer()
        .border_1()
        .border_color(if selected {
            palette.accent_border
        } else {
            palette.border_muted
        })
        .bg(if selected {
            palette.accent_bg
        } else {
            palette.input_bg
        })
        .text_color(if selected {
            palette.accent_text
        } else {
            palette.text_secondary
        })
        .hover(|this| {
            this.bg(if selected {
                palette.accent_bg_hover
            } else {
                palette.surface_hover
            })
        })
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app);
        })
        .child(title)
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_input_changes(cx);

        let current_light = self.light_input.read(cx).text();
        let current_dark = self.dark_input.read(cx).text();
        let account_input_value = self.account_input.read(cx).text();
        let engine_path_value = self.engine_path_input.read(cx).text();
        let preset_name = preset_label(&self.form.current_preset);
        let (palette, appearance_mode) = {
            let theme = self.theme_state.read(cx);
            (theme.palette(), theme.mode())
        };
        let (
            engine_status,
            engine_source_mode,
            engine_status_message,
            engine_last_error,
            engine_release_tag,
            engine_managed_path,
            engine_custom_warning,
            engine_installing,
        ) = {
            let engine_state = self.engine_state.read(cx);
            (
                engine_state.install_status(),
                engine_state.source_mode(),
                engine_state.status_message().map(ToOwned::to_owned),
                engine_state.last_error().map(ToOwned::to_owned),
                engine_state.installed_release_tag().map(ToOwned::to_owned),
                engine_state.managed_path().to_string_lossy().to_string(),
                engine_state.custom_path_warning(),
                engine_state.is_installing(),
            )
        };
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
        let view_for_engine_install = cx.entity();
        let view_for_engine_save = cx.entity();
        let view_for_engine_clear = cx.entity();
        let view_for_engine_download_page = cx.entity();
        let view_for_mode_system = cx.entity();
        let view_for_mode_light = cx.entity();
        let view_for_mode_dark = cx.entity();
        let total_accounts = tracked_accounts.len() + own_accounts.len();
        let narrow_layout = f32::from(window.bounds().size.width) < 1120.0;
        let engine_status_label = match engine_status {
            EngineInstallStatus::NotInstalled => "Not installed",
            EngineInstallStatus::Installing => "Installing...",
            EngineInstallStatus::Ready => "Ready",
            EngineInstallStatus::Error => "Error",
        };
        let engine_source_label = match engine_source_mode {
            EngineSourceMode::Managed => "Managed install",
            EngineSourceMode::Custom => "Custom path",
            EngineSourceMode::Unavailable => "Unavailable",
        };

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
                                    .child(div().text_color(palette.text_primary).text_xl().child("General"))
                                    .child(
                                        div()
                                            .text_color(palette.text_muted)
                                            .text_sm()
                                            .whitespace_normal()
                                            .child("Manage global app behavior and appearance."),
                                    ),
                            )
                            .child(section_card(
                                "Appearance",
                                "Pick how KChess colors are rendered across the app shell and pages.",
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(34.0))
                                            .p_1()
                                            .rounded_lg()
                                            .border_1()
                                            .border_color(palette.input_border)
                                            .bg(palette.surface_alt)
                                            .flex()
                                            .gap_1()
                                            .child(appearance_mode_segment(
                                                "System",
                                                appearance_mode == AppearanceMode::System,
                                                palette,
                                                move |window, app| {
                                                    view_for_mode_system.update(app, |view, cx| {
                                                        view.set_appearance_mode(
                                                            AppearanceMode::System,
                                                            window,
                                                            cx,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(appearance_mode_segment(
                                                "Light",
                                                appearance_mode == AppearanceMode::Light,
                                                palette,
                                                move |window, app| {
                                                    view_for_mode_light.update(app, |view, cx| {
                                                        view.set_appearance_mode(
                                                            AppearanceMode::Light,
                                                            window,
                                                            cx,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(appearance_mode_segment(
                                                "Dark",
                                                appearance_mode == AppearanceMode::Dark,
                                                palette,
                                                move |window, app| {
                                                    view_for_mode_dark.update(app, |view, cx| {
                                                        view.set_appearance_mode(
                                                            AppearanceMode::Dark,
                                                            window,
                                                            cx,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .when(self.form.appearance_persistence_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    self.form
                                                        .appearance_persistence_error
                                                        .clone()
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                    }),
                                palette,
                            ))
                            .child(section_card(
                                "Chess Engine",
                                "Install Stockfish into KChess app data for computer play, or provide an optional custom binary path.",
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
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_full()
                                                            .bg(palette.surface_hover)
                                                            .text_xs()
                                                            .text_color(palette.text_secondary)
                                                            .child(engine_status_label),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_full()
                                                            .bg(palette.surface_alt)
                                                            .text_xs()
                                                            .text_color(palette.text_muted)
                                                            .child(engine_source_label),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(action_button(
                                                        "Install/Update Stockfish",
                                                        engine_installing,
                                                        palette,
                                                        move |_, app| {
                                                            view_for_engine_install.update(
                                                                app,
                                                                |view, cx| {
                                                                    view.install_or_update_engine(cx);
                                                                },
                                                            );
                                                        },
                                                    ))
                                                    .child(action_button(
                                                        "Open official page",
                                                        false,
                                                        palette,
                                                        move |_, app| {
                                                            view_for_engine_download_page.update(
                                                                app,
                                                                |view, cx| {
                                                                    view.open_stockfish_download_page(cx);
                                                                },
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(palette.text_muted)
                                            .child(format!("Managed path: {engine_managed_path}")),
                                    )
                                    .when(engine_release_tag.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.text_secondary)
                                                .child(format!(
                                                    "Installed release: {}",
                                                    engine_release_tag.clone().unwrap_or_default()
                                                )),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(palette.text_secondary)
                                                    .child("Custom engine path (optional)"),
                                            )
                                            .child(input_shell(
                                                self.engine_path_input.clone(),
                                                palette,
                                            ))
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(action_button(
                                                        "Save custom path",
                                                        false,
                                                        palette,
                                                        move |_, app| {
                                                            view_for_engine_save.update(
                                                                app,
                                                                |view, cx| {
                                                                    view.save_custom_engine_path(cx);
                                                                },
                                                            );
                                                        },
                                                    ))
                                                    .child(action_button(
                                                        "Clear custom path",
                                                        engine_path_value.trim().is_empty(),
                                                        palette,
                                                        move |_, app| {
                                                            view_for_engine_clear.update(
                                                                app,
                                                                |view, cx| {
                                                                    view.clear_custom_engine_path(cx);
                                                                },
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .when(engine_status_message.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_info_text)
                                                .child(
                                                    engine_status_message
                                                        .clone()
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                    })
                                    .when(engine_custom_warning.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    engine_custom_warning
                                                        .clone()
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                    })
                                    .when(engine_last_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(engine_last_error.clone().unwrap_or_default()),
                                        )
                                    })
                                    .when(self.form.engine_path_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    self.form
                                                        .engine_path_error
                                                        .clone()
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                    }),
                                palette,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(palette.text_primary)
                                            .text_xl()
                                            .child("Board Looks"),
                                    )
                                    .child(
                                        div()
                                            .text_color(palette.text_muted)
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
                                        self.form.light_error.is_some(),
                                        palette,
                                    ))
                                    .when(self.form.light_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    self.form.light_error.clone().unwrap_or_default(),
                                                ),
                                        )
                                    })
                                    .child(field_row(
                                        "Black squares",
                                        &current_dark,
                                        self.dark_input.clone(),
                                        self.form.dark_error.is_some(),
                                        palette,
                                    ))
                                    .when(self.form.dark_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    self.form.dark_error.clone().unwrap_or_default(),
                                                ),
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
                                                    .text_color(palette.text_secondary)
                                                    .child("Preset theme"),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .bg(palette.surface_hover)
                                                    .text_xs()
                                                    .text_color(palette.text_secondary)
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
                                                palette,
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
                                                palette,
                                                move |_, app| {
                                                    view_for_chess_com.update(app, |view, cx| {
                                                        view.apply_preset(THEME_PRESET_CHESS_COM, cx);
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .when(self.form.persistence_error.is_some(), |this| {
                                        this.child(
                                            div().text_xs().text_color(palette.status_error_text).child(
                                                self.form
                                                    .persistence_error
                                                    .clone()
                                                    .unwrap_or_else(|| "Failed to save settings".to_string()),
                                            ),
                                        )
                                    }),
                                palette,
                            ),
                        )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(palette.text_primary)
                                            .text_xl()
                                            .child("Lichess"),
                                    )
                                    .child(
                                        div()
                                            .text_color(palette.text_muted)
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
                                                .bg(palette.status_info_bg)
                                                .text_sm()
                                                .text_color(palette.status_info_text)
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
                                                .bg(palette.status_error_bg)
                                                .text_sm()
                                                .text_color(palette.status_error_text)
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
                                                            .text_color(palette.text_secondary)
                                                            .child("Connected accounts"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(palette.text_muted)
                                                            .whitespace_normal()
                                                            .child("Connect only the account you control."),
                                                    ),
                                            )
                                            .child(action_button(
                                                "Connect with Lichess",
                                                connecting_account,
                                                palette,
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
                                                        .border_color(palette.border_muted)
                                                        .bg(palette.surface_alt)
                                                        .px_3()
                                                        .py_3()
                                                        .text_sm()
                                                        .text_color(palette.text_muted)
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

                                                account_row(account, removing, palette, move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.remove_lichess_account(&username, cx);
                                                        cx.notify();
                                                    });
                                                })
                                            })),
                                    ),
                                palette,
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
                                                            .text_color(palette.text_secondary)
                                                            .child("Public Lichess username"),
                                                    )
                                                    .child(input_shell(self.account_input.clone(), palette)),
                                            )
                                            .child(action_button(
                                                "Track profile",
                                                account_input_value.trim().is_empty(),
                                                palette,
                                                move |_, app| {
                                                    view_for_add_account.update(app, |view, cx| {
                                                        view.add_lichess_account(cx);
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .when(self.form.account_error.is_some(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.status_error_text)
                                                .child(
                                                    self.form.account_error.clone().unwrap_or_default(),
                                                ),
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
                                                            .text_color(palette.text_secondary)
                                                            .child("Tracked public profiles"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_full()
                                                            .bg(palette.surface_hover)
                                                            .text_xs()
                                                            .text_color(palette.text_secondary)
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
                                                palette,
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
                                                        .border_color(palette.border_muted)
                                                        .bg(palette.surface_alt)
                                                        .px_3()
                                                        .py_3()
                                                        .text_sm()
                                                        .text_color(palette.text_muted)
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

                                                account_row(account, removing, palette, move |_, app| {
                                                    view.update(app, |view, cx| {
                                                        view.remove_lichess_account(&username, cx);
                                                        cx.notify();
                                                    });
                                                })
                                            })),
                                    ),
                                palette,
                            ),
                            )
                            .child(div().h(px(28.0))),
                    ),
            ),
        )
    }
}
