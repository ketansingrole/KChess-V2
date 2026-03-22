use gpui::{IntoElement, div, prelude::*, rgb, rgba};

fn settings_item(label: &'static str, detail: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .px_4()
        .py_3()
        .rounded_lg()
        .bg(rgba(0xffffffeb))
        .border_1()
        .border_color(rgba(0xd6dae1ff))
        .child(div().text_color(rgb(0x242424)).text_base().child(label))
        .child(div().text_color(rgb(0x666f78)).text_sm().child(detail))
}

pub fn render_settings_page() -> impl IntoElement {
    div()
        .h_full()
        .w_full()
        .p_8()
        .flex()
        .flex_col()
        .gap_4()
        .child(div().text_color(rgb(0x1f1f1f)).text_xl().child("Settings"))
        .child(
            div()
                .text_color(rgb(0x666f78))
                .text_sm()
                .child("Settings page is active. Add real settings options here next."),
        )
        .child(settings_item("General", "Configure app-level behavior"))
        .child(settings_item("Board", "Choose board and piece preferences"))
        .child(settings_item(
            "Engine",
            "Configure computer strength and search",
        ))
        .child(settings_item("Appearance", "Adjust native look and feel"))
}
