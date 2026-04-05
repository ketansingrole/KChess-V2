use std::rc::Rc;

use gpui::{Context, MouseButton, Render, Window, deferred, div, prelude::*, px, svg};

use crate::app::command_palette::match_commands;
use crate::app::navigation::TAB_OPTIONS;
use crate::computer::ComputerStrength;
use crate::pages::{PlayViewCallbacks, PlayViewProps, render_play_with_computer_page};
use crate::theme::ThemePalette;
use crate::ui::top_bar::render_top_bar;

use super::{
    ActivePane, BOARD_CELL_PX, ICON_SEARCH, KChessApp, SEARCH_BOX_HEIGHT, SIDEBAR_WIDTH,
    TOP_CHROME_HEIGHT, TOP_SEARCH_MAX_WIDTH, TOP_SEARCH_OVERLAY_TOP,
};

fn sidebar_item(
    icon_path: &'static str,
    label: &'static str,
    active: bool,
    focused: bool,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .px_2()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(palette.text_secondary)
        .hover(|this| this.bg(palette.surface_hover))
        .when(active, |this| {
            this.bg(palette.surface_alt)
                .border_1()
                .border_color(palette.border)
        })
        .when(focused, |this| {
            this.border_1()
                .border_color(palette.input_focus_border)
                .bg(palette.accent_bg)
        })
        .child(
            svg()
                .path(icon_path)
                .w(px(14.0))
                .h(px(14.0))
                .text_color(palette.text_muted),
        )
        .child(div().flex_1().child(label))
}

impl Render for KChessApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search_query = self.current_search_query(cx);
        let command_matches = match_commands(&search_query);
        if search_query != self.navigation.last_search_query {
            self.navigation.last_search_query = search_query.clone();
            self.navigation.reset_selection(command_matches.len());
        } else {
            self.navigation.sync_selection(command_matches.len());
        }

        let show_play_with_computer = self.navigation.active_pane == ActivePane::PlayWithComputer;
        let show_history = self.navigation.active_pane == ActivePane::History;
        let show_settings = self.navigation.active_pane == ActivePane::Settings;
        if show_play_with_computer {
            self.maybe_schedule_computer_move(cx);
        }
        let window_size = window.bounds().size;
        let window_width = f32::from(window_size.width);
        let window_height = f32::from(window_size.height);
        let board_size = BOARD_CELL_PX * 8.0;
        let content_width = (window_width - SIDEBAR_WIDTH).max(0.0);
        let wide_layout_min_width = board_size + 280.0 + 24.0 + 48.0;
        let narrow_layout = content_width < wide_layout_min_width;
        let _right_width = (window_width - SIDEBAR_WIDTH).max(board_size);
        let _main_height = (window_height - TOP_CHROME_HEIGHT).max(board_size);
        let palette = self.theme_state.read(cx).palette();
        let (move_history, move_history_scroll_handle, can_step_back, can_step_forward) = {
            let board_view = self.board_view.read(cx);
            (
                board_view.move_history(),
                board_view.move_history_scroll_handle(),
                board_view.can_step_back_view(),
                board_view.can_step_forward_view(),
            )
        };

        let shortcut_hint = if self.cmd_held { "⌘ + F" } else { "" };
        let search_has_focus = self.search_input.read(cx).is_focused(window);
        let show_suggestions = self.navigation.search_active || search_has_focus;
        let play_focused = self.play_focus.is_focused(window);
        let history_focused = self.history_focus.is_focused(window);
        let settings_focused = self.settings_focus.is_focused(window);
        let view = cx.entity();
        let engine_ready = self.engine_state.read(cx).resolved_engine_path().is_some();
        let computer_status = self.computer_controller.status().map(ToOwned::to_owned);
        let computer_error = self.computer_controller.error().map(ToOwned::to_owned);
        let computer_strength = self.computer_controller.strength();
        let computer_user_side = self.computer_controller.user_side();
        let computer_dropdown_open = self.play_screen.computer_dropdown_open();
        let computer_thinking = self.computer_controller.thinking();

        div()
            .size_full()
            .track_focus(&self.root_focus)
            .on_any_mouse_down(cx.listener(Self::on_root_mouse_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::focus_search_action))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::open_play_action))
            .on_action(cx.listener(Self::open_history_action))
            .on_action(cx.listener(Self::search_up_action))
            .on_action(cx.listener(Self::search_down_action))
            .on_action(cx.listener(Self::search_confirm_action))
            .on_action(cx.listener(Self::search_cancel_action))
            .on_action(cx.listener(Self::play_history_step_back_action))
            .on_action(cx.listener(Self::play_history_step_forward_action))
            .on_action(cx.listener(Self::flip_board_action))
            .on_action(cx.listener(Self::show_help_action))
            .relative()
            .flex()
            .flex_col()
            .bg(palette.app_bg)
            .child(render_top_bar(
                div()
                    .relative()
                    .w_full()
                    .max_w(px(TOP_SEARCH_MAX_WIDTH))
                    .min_w(px(0.0))
                    .on_any_mouse_down(cx.listener(Self::on_search_region_mouse_down))
                    .child(
                        div()
                            .h(px(SEARCH_BOX_HEIGHT))
                            .w_full()
                            .px_2()
                            .rounded_md()
                            .bg(palette.input_bg)
                            .border_1()
                            .border_color(palette.input_border)
                            .focus(|style| style.border_color(palette.input_focus_border))
                            .flex()
                            .items_center()
                            .justify_between()
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_search_click))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(
                                        svg()
                                            .path(ICON_SEARCH)
                                            .w(px(12.0))
                                            .h(px(12.0))
                                            .text_color(palette.text_muted),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(self.search_input.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child(shortcut_hint),
                            ),
                    )
                    .when(show_suggestions, |this| {
                        this.child(deferred(
                            div()
                                .id("top-search-overlay")
                                .key_context("SearchOverlay")
                                .on_any_mouse_down(cx.listener(Self::on_search_region_mouse_down))
                                .absolute()
                                .left(px(0.0))
                                .top(px(TOP_SEARCH_OVERLAY_TOP))
                                .w_full()
                                .rounded_md()
                                .bg(palette.overlay_bg)
                                .border_1()
                                .border_color(palette.border)
                                .shadow_lg()
                                .p_2()
                                .when(command_matches.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .px_2()
                                            .py_2()
                                            .text_sm()
                                            .text_color(palette.text_muted)
                                            .child("No commands found"),
                                    )
                                })
                                .children(command_matches.iter().enumerate().map(
                                    |(ix, command)| {
                                        let selected = ix == self.navigation.search_selection;
                                        let action = command.action;
                                        let view = view.clone();
                                        let show_group_heading = ix == 0
                                            || command_matches[ix - 1].group != command.group;

                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .when(show_group_heading, |this| {
                                                this.child(
                                                    div()
                                                        .px_2()
                                                        .pt_2()
                                                        .text_xs()
                                                        .text_color(palette.text_muted)
                                                        .child(command.group.heading()),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_sm()
                                                    .cursor_pointer()
                                                    .hover(|this| this.bg(palette.surface_hover))
                                                    .when(selected, |this| {
                                                        this.bg(palette.surface_selected)
                                                    })
                                                    .on_mouse_up(
                                                        MouseButton::Left,
                                                        move |_, window, app| {
                                                            view.update(app, |view, cx| {
                                                                view.execute_command(
                                                                    action, window, cx,
                                                                );
                                                            });
                                                        },
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .text_sm()
                                                            .text_color(palette.text_secondary)
                                                            .child(
                                                                svg()
                                                                    .path(command.icon_path)
                                                                    .w(px(12.0))
                                                                    .h(px(12.0))
                                                                    .text_color(palette.text_muted),
                                                            )
                                                            .child(command.label),
                                                    ),
                                            )
                                    },
                                )),
                        ))
                    }),
                TOP_CHROME_HEIGHT,
                palette,
            ))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(
                        div()
                            .w(px(SIDEBAR_WIDTH))
                            .h_full()
                            .min_h(px(0.0))
                            .p_3()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .bg(palette.sidebar_bg)
                            .border_r_1()
                            .border_color(palette.sidebar_border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .id("sidebar-primary-nav")
                                    .overflow_y_scroll()
                                    .scrollbar_width(px(8.0))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div().flex().flex_col().gap_1().pt_1().child(
                                            div()
                                                .track_focus(&self.play_focus)
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(Self::show_play_with_computer),
                                                )
                                                .child(sidebar_item(
                                                    TAB_OPTIONS[0].icon_path,
                                                    TAB_OPTIONS[0].label,
                                                    show_play_with_computer,
                                                    play_focused,
                                                    palette,
                                                )),
                                        ),
                                    )
                                    .child(
                                        div().flex().flex_col().gap_1().pt_1().child(
                                            div()
                                                .track_focus(&self.history_focus)
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(Self::show_history),
                                                )
                                                .child(sidebar_item(
                                                    TAB_OPTIONS[1].icon_path,
                                                    TAB_OPTIONS[1].label,
                                                    show_history,
                                                    history_focused,
                                                    palette,
                                                )),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .track_focus(&self.settings_focus)
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(Self::show_settings),
                                    )
                                    .child(sidebar_item(
                                        TAB_OPTIONS[2].icon_path,
                                        TAB_OPTIONS[2].label,
                                        show_settings,
                                        settings_focused,
                                        palette,
                                    )),
                            ),
                    )
                    .child(
                        div().flex_1().min_h(px(0.0)).relative().child(
                            div()
                                .size_full()
                                .bg(palette.panel_bg)
                                .when(show_play_with_computer, |this| {
                                    this.child(
                                        div()
                                            .size_full()
                                            .track_focus(&self.play_surface_focus)
                                            .key_context("PlayMoveHistory")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(Self::on_play_surface_mouse_down),
                                            )
                                            .child(render_play_with_computer_page(
                                                PlayViewProps {
                                                    board_view: self.board_view.clone(),
                                                    move_history: move_history.clone(),
                                                    board_size,
                                                    narrow_layout,
                                                    move_history_scroll_handle: move_history_scroll_handle
                                                        .clone(),
                                                    can_step_back,
                                                    can_step_forward,
                                                    palette,
                                                    computer_dropdown_open,
                                                    selected_strength: computer_strength,
                                                    user_side: computer_user_side,
                                                    computer_thinking,
                                                    engine_ready,
                                                    computer_status: computer_status.clone(),
                                                    computer_error: computer_error.clone(),
                                                },
                                                PlayViewCallbacks {
                                                    on_toggle_computer_dropdown: Rc::new({
                                                        let view = view.clone();
                                                        move |app| {
                                                            view.update(app, |view, cx| {
                                                                view.toggle_computer_dropdown(cx);
                                                            });
                                                        }
                                                    }),
                                                    on_select_low: Rc::new({
                                                        let view = view.clone();
                                                        move |app| {
                                                            view.update(app, |view, cx| {
                                                                view.select_computer_strength(
                                                                    ComputerStrength::Low,
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                    on_select_medium: Rc::new({
                                                        let view = view.clone();
                                                        move |app| {
                                                            view.update(app, |view, cx| {
                                                                view.select_computer_strength(
                                                                    ComputerStrength::Medium,
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                    on_select_high: Rc::new({
                                                        let view = view.clone();
                                                        move |app| {
                                                            view.update(app, |view, cx| {
                                                                view.select_computer_strength(
                                                                    ComputerStrength::High,
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                    on_flip_board: Rc::new({
                                                        let view = view.clone();
                                                        move |app| {
                                                            view.update(app, |view, cx| {
                                                                view.apply_flip_board(cx);
                                                            });
                                                        }
                                                    }),
                                                    on_computer_control_mouse_down: Rc::new({
                                                        let view = view.clone();
                                                        move |_, app| {
                                                            view.update(app, |view, cx| {
                                                                view.on_computer_control_region_mouse_down(
                                                                    cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                },
                                            )),
                                    )
                                })
                                .when(show_history, |this| this.child(self.history_view.clone()))
                                .when(show_settings, |this| this.child(self.settings_view.clone())),
                        ),
                    ),
            )
    }
}
