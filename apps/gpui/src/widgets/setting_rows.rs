//! Rows of a settings section: a label, its hint and its control.

use super::*;

/// The blindfold and zen mode toggles every game page shows.
/// A switch with its label beside it: the switch is a 24 px target (WCAG 2.5.8) named by the
/// label, which takes the row's text size and toggles it too.
pub fn labelled_switch(
    id: &'static str,
    label: &'static str,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
    let on_change = std::rc::Rc::new(on_change);
    let on_label = std::rc::Rc::clone(&on_change);
    h_flex()
        .gap(px(8.))
        .items_center()
        .child(
            gpui_kit::component::switch::Switch::new(id)
                .large()
                .checked(checked)
                .disabled(disabled)
                .accessibility_label(label)
                .on_click(move |on, window, cx| on_change(on, window, cx)),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-label")))
                .min_w_0()
                .when(!disabled, |label| label.cursor_pointer())
                .when(!disabled, |label| {
                    label.on_click(move |_, window, cx| on_label(&!checked, window, cx))
                })
                .child(label),
        )
}

pub fn view_toggles(blindfold: bool, zen: bool) -> impl IntoElement {
    let toggle =
        |id: &'static str, glyph: gpui_kit::component::Icon, label: &'static str, on: bool| {
            Button::new(id)
                .small()
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
                .icon(glyph)
                .tooltip(label)
                .accessibility_label(label)
        };
    h_flex()
        .gap(px(2.))
        .child(
            toggle(
                "blindfold",
                icon("eye-off"),
                "Blindfold: hide the pieces",
                blindfold,
            )
            .on_click(|_, _, cx| {
                crate::settings::change(cx, |s| s.frontend.blindfold = !s.frontend.blindfold)
            }),
        )
        .child(
            toggle("zen", icon("maximize"), "Zen mode (Z)", zen).on_click(|_, _, cx| {
                crate::settings::change(cx, |s| s.frontend.zen_mode = !s.frontend.zen_mode)
            }),
        )
}

/// A setting's title and its optional hint, the left of a setting row.
pub fn setting_info(
    title: impl Into<SharedString>,
    hint: Option<SharedString>,
    palette: &Palette,
) -> Div {
    let title = title.into();
    v_flex()
        .gap(px(4.))
        .min_w_0()
        .child(
            div()
                .id(ElementId::NamedChild(
                    std::sync::Arc::new("setting-title".into()),
                    title.clone(),
                ))
                .test_support()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title),
        )
        .children(hint.map(|hint| {
            div()
                .text_size(px(12.))
                .line_height(relative(1.45))
                .text_color(palette.text_muted)
                .child(hint)
        }))
}

/// A setting row: its title (and hint) on the left, taking what the control leaves, and the
/// control at the right edge, at its own width.
pub fn setting_row(info: Div, control: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .justify_between()
        .gap(px(24.))
        .child(info.flex_1())
        .child(h_flex().flex_none().justify_end().child(control))
}

/// The width of a setting's field (a list, a slider), so the fields of a list line up.
pub const SETTING_FIELD: Pixels = px(240.);

/// A setting's field at `SETTING_FIELD`.
pub fn setting_field(control: impl IntoElement) -> Div {
    div().w(SETTING_FIELD).child(control)
}
/// Setting rows one under another, with dividers between them.
pub fn setting_list(rows: impl IntoIterator<Item = Div>, palette: &Palette) -> Div {
    v_flex().children(rows.into_iter().enumerate().map(|(index, row)| {
        row.py(px(12.)).when(index > 0, |row| {
            row.border_t_1().border_color(palette.border)
        })
    }))
}

/// A form field: its label above, and a problem below when there is one.
pub fn field(
    label: &'static str,
    control: impl IntoElement,
    problem: Option<SharedString>,
    palette: &Palette,
) -> Div {
    v_flex()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(label),
        )
        .child(control)
        .children(problem.map(|problem| {
            div()
                .text_size(px(13.))
                .text_color(palette.error)
                .child(problem)
        }))
}
