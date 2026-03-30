use gpui::{
    Entity, FontWeight, IntoElement, MouseButton, ScrollHandle, div, prelude::*, px, rgb, rgba, svg,
};
use kchess_board::{ChessBoardView, MoveHistoryCell, MoveHistoryRow};

const ICON_CHEVRON_LEFT: &str = "assets/icons/chevron-left.svg";
const ICON_CHEVRON_RIGHT: &str = "assets/icons/chevron-right.svg";

fn move_cell(
    cell: Option<&MoveHistoryCell>,
    board_view: Entity<ChessBoardView>,
) -> impl IntoElement {
    let is_latest = cell.is_some_and(|cell| cell.is_latest);
    let is_current = cell.is_some_and(|cell| cell.is_current);
    let notation = cell.map(|cell| cell.notation.clone()).unwrap_or_default();
    let move_id = cell.map(|cell| cell.id);
    let board_for_click = board_view.clone();

    div()
        .flex_1()
        .min_h(px(44.0))
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .text_center()
        .text_color(rgb(0x2d3748))
        .when(is_current, |this| {
            this.bg(rgb(0xdbeafe))
                .text_color(rgb(0x1d4ed8))
                .font_weight(FontWeight::BOLD)
        })
        .when(is_latest && !is_current, |this| {
            this.bg(rgba(0xe2e8f066)).text_color(rgb(0x334155))
        })
        .when(cell.is_some(), |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xe2e8f0b3)))
                .on_mouse_up(MouseButton::Left, move |_, _, app| {
                    if let Some(move_id) = move_id {
                        board_for_click.update(app, |view, cx| {
                            if view.jump_to_move(move_id) {
                                cx.notify();
                            }
                        });
                    }
                })
        })
        .child(notation)
}

fn move_row(row: &MoveHistoryRow, board_view: Entity<ChessBoardView>) -> impl IntoElement {
    let board_for_white = board_view.clone();
    let board_for_black = board_view.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .px_3()
        .py_1()
        .child(
            div()
                .w(px(34.0))
                .min_h(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(0x718096))
                .child(row.move_number.to_string()),
        )
        .child(move_cell(row.white.as_ref(), board_for_white))
        .child(move_cell(row.black.as_ref(), board_for_black))
}

fn reset_button(board_view: Entity<ChessBoardView>) -> impl IntoElement {
    div()
        .group("reset-button")
        .relative()
        .w(px(36.0))
        .h(px(36.0))
        .rounded_lg()
        .cursor_pointer()
        .bg(rgba(0xf8fafcff))
        .border_1()
        .border_color(rgba(0x94a3b833))
        .hover(|this| this.bg(rgba(0xf1f5f9ff)).border_color(rgba(0x64748b55)))
        .flex()
        .items_center()
        .justify_center()
        .text_lg()
        .text_color(rgb(0x334155))
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
                .bg(rgb(0x0f172a))
                .border_1()
                .border_color(rgba(0xffffff33))
                .shadow_lg()
                .text_xs()
                .text_color(rgb(0xf8fafc))
                .opacity(0.0)
                .group_hover("reset-button", |this| this.opacity(1.0))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Reset Board"))
                .child(
                    div()
                        .pt_1()
                        .text_color(rgb(0xcbd5e1))
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
) -> impl IntoElement {
    let group_name = tooltip_title;
    div()
        .group(group_name)
        .relative()
        .w(px(36.0))
        .h(px(36.0))
        .rounded_lg()
        .bg(rgba(0xf8fafcff))
        .border_1()
        .border_color(if disabled {
            rgba(0x94a3b833)
        } else {
            rgba(0x64748b55)
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgba(0xf1f5f9ff)))
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
                .text_color(rgb(0x334155)),
        )
        .child(
            div()
                .absolute()
                .right(px(0.0))
                .bottom(px(44.0))
                .w(px(220.0))
                .p_2()
                .rounded_md()
                .bg(rgb(0x0f172a))
                .border_1()
                .border_color(rgba(0xffffff33))
                .shadow_lg()
                .text_xs()
                .text_color(rgb(0xf8fafc))
                .opacity(0.0)
                .group_hover(group_name, |this| this.opacity(1.0))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(tooltip_title))
                .child(div().pt_1().text_color(rgb(0xcbd5e1)).child(tooltip_body)),
        )
}

pub fn render_play_with_computer_page(
    board_view: Entity<ChessBoardView>,
    move_history: Vec<MoveHistoryRow>,
    board_size: f32,
    move_history_scroll_handle: ScrollHandle,
    can_step_back: bool,
    can_step_forward: bool,
) -> impl IntoElement {
    let history_empty = move_history.is_empty();
    let board_for_button = board_view.clone();
    let board_for_step_back = board_view.clone();
    let board_for_step_forward = board_view.clone();
    let history_list = div()
        .flex_1()
        .id("move-history-list")
        .track_scroll(&move_history_scroll_handle)
        .overflow_y_scroll()
        .py_2()
        .when(history_empty, |this| {
            this.child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_6()
                    .text_sm()
                    .text_color(rgb(0x8b929b))
                    .child("Moves will appear here as you play."),
            )
        })
        .children(
            move_history
                .iter()
                .map(|row| move_row(row, board_view.clone())),
        );

    div()
        .size_full()
        .p_6()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .gap_6()
                .child(
                    div().flex().flex_none().child(
                        div()
                            .w(px(board_size))
                            .h(px(board_size))
                            .rounded_xl()
                            .overflow_hidden()
                            .child(board_view),
                    ),
                )
                .child(
                    div()
                        .h(px(board_size))
                        .max_w(px(360.0))
                        .min_w(px(280.0))
                        .flex()
                        .flex_col()
                        .rounded_xl()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgba(0x94a3b833))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(rgba(0x94a3b822))
                                .child(
                                    div()
                                        .w(px(34.0))
                                        .text_center()
                                        .text_xs()
                                        .text_color(rgb(0x64748b))
                                        .child("#"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_center()
                                        .text_xs()
                                        .text_color(rgb(0x475569))
                                        .child("White"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_center()
                                        .text_xs()
                                        .text_color(rgb(0x475569))
                                        .child("Black"),
                                ),
                        )
                        .child(history_list)
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .border_t_1()
                                .border_color(rgba(0x94a3b822))
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
                                ))
                                .child(reset_button(board_for_button)),
                        ),
                ),
        )
}
