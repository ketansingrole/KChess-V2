use gpui::{Entity, IntoElement, div, prelude::*, px};
use kchess_board::ChessBoardView;

pub fn render_play_with_computer_page(
    board_view: Entity<ChessBoardView>,
    board_left: f32,
    board_top: f32,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(board_left))
        .top(px(board_top))
        .child(board_view)
}
