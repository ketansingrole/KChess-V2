use gpui::{IntoElement, ObjectFit, div, img, prelude::*, px, rgb, rgba};

const APP_ICON_PATH: &str = "assets/app/kchess.icon/Assets/Image.png";

pub fn render_top_bar(title: &'static str, height: f32) -> impl IntoElement {
    div()
        .h(px(height))
        .w_full()
        .bg(rgba(0xf7f8faf7))
        .border_b_1()
        .border_color(rgba(0xd6d9deff))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(rgb(0x636c76))
                .child(
                    img(APP_ICON_PATH)
                        .w(px(18.0))
                        .h(px(18.0))
                        .object_fit(ObjectFit::Contain),
                )
                .child(title),
        )
}
