use std::rc::Rc;

use gpui::{
    App, Entity, FontWeight, IntoElement, MouseButton, ScrollHandle, Window, deferred, div,
    prelude::*, px, svg,
};
use kchess_board::{ChessBoardView, MoveHistoryRow, Side};

use crate::computer::ComputerStrength;
use crate::theme::ThemePalette;
use crate::ui::move_table::render_move_table;

const ICON_CHEVRON_LEFT: &str = "assets/icons/chevron-left.svg";
const ICON_CHEVRON_RIGHT: &str = "assets/icons/chevron-right.svg";
const ICON_FLIP_BOARD: &str = "assets/icons/flip-board.svg";
const ICON_COMPUTER: &str = "assets/icons/computer.svg";
const ICON_CHEVRON_DOWN: &str = "assets/icons/chevron-down.svg";

pub struct PlayViewProps {
    pub board_view: Entity<ChessBoardView>,
    pub move_history: Vec<MoveHistoryRow>,
    pub board_size: f32,
    pub narrow_layout: bool,
    pub move_history_scroll_handle: ScrollHandle,
    pub can_step_back: bool,
    pub can_step_forward: bool,
    pub palette: ThemePalette,
    pub computer_dropdown_open: bool,
    pub selected_strength: Option<ComputerStrength>,
    pub user_side: Option<Side>,
    pub computer_thinking: bool,
    pub engine_ready: bool,
    pub computer_status: Option<String>,
    pub computer_error: Option<String>,
}

pub struct PlayViewCallbacks {
    pub on_toggle_computer_dropdown: AppCallback,
    pub on_select_low: AppCallback,
    pub on_select_medium: AppCallback,
    pub on_select_high: AppCallback,
    pub on_flip_board: AppCallback,
    pub on_computer_control_mouse_down: WindowAppCallback,
}

struct ComputerControlProps {
    dropdown_open: bool,
    selected_strength: Option<ComputerStrength>,
    user_side: Option<Side>,
    computer_thinking: bool,
    engine_ready: bool,
    status_message: Option<String>,
    error_message: Option<String>,
    palette: ThemePalette,
}

pub type AppCallback = Rc<dyn for<'a> Fn(&'a mut App)>;
pub type WindowAppCallback = Rc<dyn for<'a, 'b> Fn(&'a mut Window, &'b mut App)>;

struct ComputerControlCallbacks {
    on_toggle_dropdown: AppCallback,
    on_select_low: AppCallback,
    on_select_medium: AppCallback,
    on_select_high: AppCallback,
}

fn reset_button(board_view: Entity<ChessBoardView>, palette: ThemePalette) -> impl IntoElement {
    div()
        .group("reset-button")
        .relative()
        .w(px(36.0))
        .h(px(36.0))
        .rounded_lg()
        .cursor_pointer()
        .bg(palette.surface_alt)
        .border_1()
        .border_color(palette.border_muted)
        .hover(|this| this.bg(palette.surface_hover).border_color(palette.border))
        .flex()
        .items_center()
        .justify_center()
        .text_lg()
        .text_color(palette.text_secondary)
        .on_mouse_up(MouseButton::Left, move |_, _, app| {
            board_view.update(app, |view, cx| {
                view.reset_to_start();
                cx.notify();
            });
        })
        .child("↺")
        .child(
            div()
                .absolute()
                .right(px(0.0))
                .bottom(px(44.0))
                .w(px(220.0))
                .p_2()
                .rounded_md()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .shadow_lg()
                .text_xs()
                .text_color(palette.text_primary)
                .opacity(0.0)
                .group_hover("reset-button", |this| this.opacity(1.0))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Reset Board"))
                .child(
                    div()
                        .pt_1()
                        .text_color(palette.text_muted)
                        .child("Return all pieces to their starting positions."),
                ),
        )
}

fn history_nav_button(
    icon_path: &'static str,
    disabled: bool,
    tooltip_title: &'static str,
    tooltip_body: &'static str,
    on_click: impl Fn(&mut gpui::App) + 'static,
    palette: ThemePalette,
) -> impl IntoElement {
    let group_name = tooltip_title;
    div()
        .group(group_name)
        .relative()
        .w(px(36.0))
        .h(px(36.0))
        .rounded_lg()
        .bg(palette.surface_alt)
        .border_1()
        .border_color(if disabled {
            palette.border_muted
        } else {
            palette.border
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette.surface_hover))
                .on_mouse_up(MouseButton::Left, move |_, _, app| {
                    on_click(app);
                })
        })
        .when(disabled, |this| this.opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icon_path)
                .w(px(16.0))
                .h(px(16.0))
                .text_color(palette.text_secondary),
        )
        .child(
            div()
                .absolute()
                .right(px(0.0))
                .bottom(px(44.0))
                .w(px(220.0))
                .p_2()
                .rounded_md()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .shadow_lg()
                .text_xs()
                .text_color(palette.text_primary)
                .opacity(0.0)
                .group_hover(group_name, |this| this.opacity(1.0))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(tooltip_title))
                .child(
                    div()
                        .pt_1()
                        .text_color(palette.text_muted)
                        .child(tooltip_body),
                ),
        )
}

fn computer_dropdown_option(
    label: String,
    selected: bool,
    on_click: AppCallback,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .text_color(if selected {
            palette.accent_text
        } else {
            palette.text_secondary
        })
        .bg(if selected {
            palette.accent_bg
        } else {
            palette.surface
        })
        .hover(|this| this.bg(palette.surface_hover))
        .on_mouse_up(MouseButton::Left, move |_, _, app| {
            (on_click)(app);
        })
        .child(label)
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::White => "White",
        Side::Black => "Black",
    }
}

fn computer_control(
    props: ComputerControlProps,
    callbacks: ComputerControlCallbacks,
) -> impl IntoElement {
    let ComputerControlProps {
        dropdown_open,
        selected_strength,
        user_side,
        computer_thinking,
        engine_ready,
        status_message,
        error_message,
        palette,
    } = props;
    let ComputerControlCallbacks {
        on_toggle_dropdown,
        on_select_low,
        on_select_medium,
        on_select_high,
    } = callbacks;

    let strength_label = selected_strength
        .map(ComputerStrength::display_label)
        .unwrap_or_else(|| "Select strength".to_string());
    let helper_text = if engine_ready {
        user_side
            .map(|side| format!("You are playing {}", side_label(side)))
            .unwrap_or_else(|| "Select a level to start a game against computer.".to_string())
    } else {
        "Install Stockfish in Settings to enable computer play.".to_string()
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(236.0))
                        .flex_none()
                        .relative()
                        .child(
                            div()
                                .h(px(36.0))
                                .w_full()
                                .px_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(if engine_ready {
                                    palette.border
                                } else {
                                    palette.border_muted
                                })
                                .bg(if engine_ready {
                                    palette.surface
                                } else {
                                    palette.surface_alt
                                })
                                .when(engine_ready, |this| this.cursor_pointer())
                                .when(engine_ready, |this| {
                                    this.on_mouse_up(MouseButton::Left, move |_, _, app| {
                                        (on_toggle_dropdown)(app);
                                    })
                                })
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            svg()
                                                .path(ICON_COMPUTER)
                                                .w(px(14.0))
                                                .h(px(14.0))
                                                .text_color(palette.text_muted),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(if engine_ready {
                                                    palette.text_secondary
                                                } else {
                                                    palette.text_muted
                                                })
                                                .child(strength_label),
                                        ),
                                )
                                .child(
                                    svg()
                                        .path(ICON_CHEVRON_DOWN)
                                        .w(px(12.0))
                                        .h(px(12.0))
                                        .text_color(palette.text_muted),
                                ),
                        )
                        .when(dropdown_open && engine_ready, |this| {
                            this.child(
                                deferred(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(42.0))
                                        .w(px(236.0))
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(palette.border)
                                        .bg(palette.overlay_bg)
                                        .shadow_lg()
                                        .p_2()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(computer_dropdown_option(
                                            ComputerStrength::Low.display_label(),
                                            selected_strength == Some(ComputerStrength::Low),
                                            on_select_low,
                                            palette,
                                        ))
                                        .child(computer_dropdown_option(
                                            ComputerStrength::Medium.display_label(),
                                            selected_strength == Some(ComputerStrength::Medium),
                                            on_select_medium,
                                            palette,
                                        ))
                                        .child(computer_dropdown_option(
                                            ComputerStrength::High.display_label(),
                                            selected_strength == Some(ComputerStrength::High),
                                            on_select_high,
                                            palette,
                                        )),
                                )
                                .priority(1),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_center()
                        .text_color(palette.text_muted)
                        .whitespace_normal()
                        .child(helper_text),
                ),
        )
        .when(computer_thinking, |this| {
            this.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_muted)
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .whitespace_normal()
                    .child("Computer is thinking..."),
            )
        })
        .when(status_message.is_some(), |this| {
            this.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_muted)
                    .text_xs()
                    .text_color(palette.status_info_text)
                    .whitespace_normal()
                    .child(status_message.unwrap_or_default()),
            )
        })
        .when(error_message.is_some(), |this| {
            this.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_muted)
                    .text_xs()
                    .text_color(palette.status_error_text)
                    .whitespace_normal()
                    .child(error_message.unwrap_or_default()),
            )
        })
}

pub fn render_play_with_computer_page(
    props: PlayViewProps,
    callbacks: PlayViewCallbacks,
) -> impl IntoElement {
    let PlayViewProps {
        board_view,
        move_history,
        board_size,
        narrow_layout,
        move_history_scroll_handle,
        can_step_back,
        can_step_forward,
        palette,
        computer_dropdown_open,
        selected_strength,
        user_side,
        computer_thinking,
        engine_ready,
        computer_status,
        computer_error,
    } = props;
    let PlayViewCallbacks {
        on_toggle_computer_dropdown,
        on_select_low,
        on_select_medium,
        on_select_high,
        on_flip_board,
        on_computer_control_mouse_down,
    } = callbacks;

    let history_panel_height = if narrow_layout {
        (board_size * 0.72).max(320.0)
    } else {
        board_size
    };
    let board_for_button = board_view.clone();
    let board_for_step_back = board_view.clone();
    let board_for_step_forward = board_view.clone();
    let board_column = div()
        .w(px(board_size))
        .h(px(board_size))
        .flex_none()
        .relative()
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .bottom(px(board_size + 10.0))
                .w(px(board_size))
                .on_any_mouse_down(move |_, window, app| {
                    (on_computer_control_mouse_down)(window, app);
                })
                .child(computer_control(
                    ComputerControlProps {
                        dropdown_open: computer_dropdown_open,
                        selected_strength,
                        user_side,
                        computer_thinking,
                        engine_ready,
                        status_message: computer_status,
                        error_message: computer_error,
                        palette,
                    },
                    ComputerControlCallbacks {
                        on_toggle_dropdown: on_toggle_computer_dropdown,
                        on_select_low,
                        on_select_medium,
                        on_select_high,
                    },
                )),
        )
        .child(
            div()
                .w(px(board_size))
                .h(px(board_size))
                .rounded_xl()
                .overflow_hidden()
                .child(board_view.clone()),
        );
    let history_panel = div()
        .h(px(history_panel_height))
        .w_full()
        .max_w(px(if narrow_layout { board_size } else { 360.0 }))
        .min_w(px(if narrow_layout { 0.0 } else { 280.0 }))
        .flex_none()
        .flex()
        .flex_col()
        .rounded_xl()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border_muted)
        .child(render_move_table(
            move_history,
            board_view.clone(),
            move_history_scroll_handle,
            palette,
        ))
        .child(
            div()
                .px_4()
                .py_3()
                .border_t_1()
                .border_color(palette.border_muted)
                .flex()
                .items_center()
                .gap_2()
                .justify_end()
                .child(history_nav_button(
                    ICON_CHEVRON_LEFT,
                    !can_step_back,
                    "Previous Move",
                    "Step one move backward in read-only history view.",
                    move |app| {
                        board_for_step_back.update(app, |view, cx| {
                            if view.step_back_view() {
                                cx.notify();
                            }
                        });
                    },
                    palette,
                ))
                .child(history_nav_button(
                    ICON_CHEVRON_RIGHT,
                    !can_step_forward,
                    "Next Move",
                    "Step one move forward in read-only history view.",
                    move |app| {
                        board_for_step_forward.update(app, |view, cx| {
                            if view.step_forward_view() {
                                cx.notify();
                            }
                        });
                    },
                    palette,
                ))
                .child(history_nav_button(
                    ICON_FLIP_BOARD,
                    false,
                    "Flip Board",
                    "Rotate the board perspective (shortcut: F).",
                    move |app| {
                        (on_flip_board)(app);
                    },
                    palette,
                ))
                .child(reset_button(board_for_button, palette)),
        );

    div()
        .size_full()
        .min_h(px(0.0))
        .id("play-with-computer-scroll")
        .p_6()
        .flex()
        .justify_center()
        .overflow_y_scroll()
        .when(narrow_layout, |this| this.items_start())
        .when(!narrow_layout, |this| this.items_center())
        .child(
            div()
                .w_full()
                .flex()
                .gap_6()
                .when(narrow_layout, |this| this.flex_col().items_center())
                .when(!narrow_layout, |this| this.items_center().justify_center())
                .child(board_column)
                .child(history_panel),
        )
}
