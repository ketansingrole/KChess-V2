use gpui::{IntoElement, ObjectFit, div, img, prelude::*, px, rgb, rgba};

const APP_ICON_PATH: &str = "assets/app/kchess.icon/Assets/Image.png";
const TITLEBAR_SAFE_LEFT: f32 = 86.0;
const TITLEBAR_SAFE_RIGHT: f32 = 86.0;

pub fn render_top_bar(title: &'static str, height: f32) -> impl IntoElement {
    div()
        .h(px(height))
        .w_full()
        .px_4()
        .bg(rgba(0xf7f8faf7))
        .border_b_1()
        .border_color(rgba(0xd6d9deff))
        .flex()
        .items_center()
        .justify_between()
        .child(div().w(px(TITLEBAR_SAFE_LEFT)))
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(rgb(0x636c76))
                .child(
                    img(APP_ICON_PATH)
                        .w(px(16.0))
                        .h(px(16.0))
                        .object_fit(ObjectFit::Contain),
                )
                .child(title),
        )
        .child(div().w(px(TITLEBAR_SAFE_RIGHT)))
}
