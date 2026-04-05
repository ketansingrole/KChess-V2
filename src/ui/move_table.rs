use gpui::{Entity, FontWeight, IntoElement, MouseButton, ScrollHandle, div, prelude::*, px};
use kchess_board::{ChessBoardView, MoveHistoryCell, MoveHistoryRow};

use crate::theme::ThemePalette;

const MOVE_INDEX_COL_WIDTH: f32 = 34.0;

fn move_cell(
    cell: Option<&MoveHistoryCell>,
    board_view: Entity<ChessBoardView>,
    palette: ThemePalette,
) -> impl IntoElement {
    let is_latest = cell.is_some_and(|cell| cell.is_latest);
    let is_current = cell.is_some_and(|cell| cell.is_current);
    let notation = cell.map(|cell| cell.notation.clone()).unwrap_or_default();
    let move_id = cell.map(|cell| cell.id);
    let board_for_click = board_view.clone();

    div()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(44.0))
        .rounded_sm()
        .text_sm()
        .text_center()
        .text_color(palette.text_secondary)
        .when(is_current, |this| {
            this.bg(palette.accent_bg)
                .text_color(palette.accent_text)
                .font_weight(FontWeight::BOLD)
        })
        .when(is_latest && !is_current, |this| {
            this.bg(palette.surface_alt)
                .text_color(palette.text_secondary)
        })
        .when(cell.is_some(), |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette.surface_hover))
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
        .child(
            div()
                .size_full()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .justify_center()
                .child(notation),
        )
}

fn move_row(
    row: &MoveHistoryRow,
    board_view: Entity<ChessBoardView>,
    palette: ThemePalette,
) -> impl IntoElement {
    let board_for_white = board_view.clone();
    let board_for_black = board_view.clone();
    div()
        .flex()
        .justify_start()
        .gap_3()
        .w_full()
        .px_4()
        .py_1()
        .child(
            div()
                .flex_none()
                .w(px(MOVE_INDEX_COL_WIDTH))
                .min_h(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(palette.text_muted)
                .child(row.move_number.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(44.0))
                .relative()
                .child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .top(px(0.0))
                        .bottom(px(0.0))
                        .child(move_cell(row.white.as_ref(), board_for_white, palette)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(44.0))
                .relative()
                .child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .top(px(0.0))
                        .bottom(px(0.0))
                        .child(move_cell(row.black.as_ref(), board_for_black, palette)),
                ),
        )
}

pub fn render_move_table(
    move_history: Vec<MoveHistoryRow>,
    board_view: Entity<ChessBoardView>,
    move_history_scroll_handle: ScrollHandle,
    palette: ThemePalette,
) -> impl IntoElement {
    let history_empty = move_history.is_empty();
    let history_list = div()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .id("move-history-list")
        .track_scroll(&move_history_scroll_handle)
        .overflow_y_scroll()
        .py_2()
        .flex()
        .flex_col()
        .children(
            move_history
                .iter()
                .map(|row| move_row(row, board_view.clone(), palette)),
        );

    div()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .w_full()
                .justify_start()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(palette.border_muted)
                .child(
                    div()
                        .flex_none()
                        .w(px(MOVE_INDEX_COL_WIDTH))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child("#"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_xs()
                        .text_color(palette.text_secondary)
                        .child("White"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_xs()
                        .text_color(palette.text_secondary)
                        .child("Black"),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .relative()
                .child(history_list)
                .when(history_empty, |this| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(0.0))
                            .right(px(0.0))
                            .top(px(0.0))
                            .bottom(px(0.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_6()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(
                                div()
                                    .max_w(px(260.0))
                                    .text_center()
                                    .child("Moves will appear here as you play."),
                            ),
                    )
                }),
        )
}
