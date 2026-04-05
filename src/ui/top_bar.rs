use gpui::{IntoElement, div, prelude::*, px};

use crate::theme::ThemePalette;

const TITLEBAR_SAFE_LEFT: f32 = 86.0;
const TITLEBAR_SAFE_RIGHT: f32 = 86.0;

pub fn render_top_bar(
    center_content: impl IntoElement,
    height: f32,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .h(px(height))
        .w_full()
        .px_4()
        .bg(palette.top_bar_bg)
        .border_b_1()
        .border_color(palette.top_bar_border)
        .flex()
        .items_center()
        .justify_between()
        .child(div().w(px(TITLEBAR_SAFE_LEFT)))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .justify_center()
                .child(center_content),
        )
        .child(div().w(px(TITLEBAR_SAFE_RIGHT)))
}
