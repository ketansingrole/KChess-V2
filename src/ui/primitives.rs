use gpui::{IntoElement, MouseButton, Window, div, prelude::*, px};

use crate::theme::ThemePalette;

pub fn action_button(
    label: impl IntoElement,
    disabled: bool,
    palette: ThemePalette,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(if disabled {
            palette.border_muted
        } else {
            palette.accent_border
        })
        .bg(if disabled {
            palette.surface_alt
        } else {
            palette.accent_bg
        })
        .text_sm()
        .text_color(if disabled {
            palette.text_muted
        } else {
            palette.accent_text
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette.accent_bg_hover))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .child(label)
}

pub fn danger_button(
    label: impl IntoElement,
    disabled: bool,
    palette: ThemePalette,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(palette.danger_border)
        .bg(if disabled {
            palette.surface_alt
        } else {
            palette.danger_bg
        })
        .text_sm()
        .text_color(if disabled {
            palette.text_muted
        } else {
            palette.danger_text
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette.danger_bg_hover))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .child(label)
}

pub fn compact_chip(
    label: impl IntoElement,
    active: bool,
    enabled: bool,
    palette: ThemePalette,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(if active {
            palette.accent_border
        } else {
            palette.input_border
        })
        .bg(if active {
            palette.accent_bg
        } else {
            palette.input_bg
        })
        .text_xs()
        .text_color(if enabled {
            if active {
                palette.accent_text
            } else {
                palette.text_secondary
            }
        } else {
            palette.text_muted
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette.surface_hover))
                .on_mouse_up(MouseButton::Left, move |_, window, app| {
                    on_click(window, app);
                })
        })
        .when(!enabled, |this| this.opacity(0.45))
        .child(label)
}

pub fn dropdown_trigger(
    label: impl IntoElement,
    palette: ThemePalette,
    trailing: impl IntoElement,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(34.0))
        .rounded_full()
        .border_1()
        .border_color(palette.input_border)
        .bg(palette.surface_alt)
        .cursor_pointer()
        .hover(|this| this.bg(palette.surface_hover))
        .on_mouse_up(MouseButton::Left, move |_, window, app| {
            on_click(window, app);
        })
        .child(
            div()
                .h_full()
                .w_full()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text_secondary)
                        .child(label),
                )
                .child(trailing),
        )
}

pub fn section_card(
    title: &'static str,
    description: &'static str,
    body: impl IntoElement,
    palette: ThemePalette,
) -> impl IntoElement {
    div()
        .w_full()
        .max_w(px(720.0))
        .flex()
        .flex_col()
        .gap_4()
        .px_4()
        .py_4()
        .rounded_xl()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_base()
                        .text_color(palette.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_sm()
                        .text_color(palette.text_muted)
                        .whitespace_normal()
                        .child(description),
                ),
        )
        .child(body)
}
