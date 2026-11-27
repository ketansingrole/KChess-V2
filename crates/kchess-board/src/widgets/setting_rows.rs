//! Rows of a settings section: a label, its hint and its control.

use super::*;

/// A switch with its label beside it: the switch is a 24 px target (WCAG 2.5.8) named by the
/// label, which takes the row's text size and toggles it too.
pub fn labelled_switch(
    id: &'static str,
    label: &'static str,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
    let (switch, label) = switch_parts(id, label, checked, disabled, on_change);
    h_flex()
        .gap(px(8.))
        .items_center()
        .child(switch)
        .child(label)
}

/// A panel's yes/no option on a row of its own: its label, then the switch at the row's end, as
/// Settings lays its switches out. The label toggles it too.
pub fn switch_row(
    id: &'static str,
    label: &'static str,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
    let (switch, label) = switch_parts(id, label, checked, disabled, on_change);
    h_flex()
        .w_full()
        .gap(px(12.))
        .items_center()
        .justify_between()
        .child(label.flex_1())
        .child(div().flex_none().child(switch))
}

/// A switch named by `label`, and its label, which toggles it.
fn switch_parts(
    id: &'static str,
    label: &'static str,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> (gpui_kit::component::switch::Switch, Div) {
    let on_change = std::rc::Rc::new(on_change);
    let on_label = std::rc::Rc::clone(&on_change);
    let switch = gpui_kit::component::switch::Switch::new(id)
        .large()
        .checked(checked)
        .disabled(disabled)
        .accessibility_label(label)
        .on_click(move |on, window, cx| on_change(on, window, cx));
    let label = div().min_w_0().child(
        div()
            .id(SharedString::from(format!("{id}-label")))
            .test_support()
            .when(!disabled, |label| label.cursor_pointer())
            .when(!disabled, |label| {
                label.on_click(move |_, window, cx| on_label(&!checked, window, cx))
            })
            .child(label),
    );
    (switch, label)
}

/// Blindfold and zen mode, as a board page offers them; zen mode's tooltip names its key.
pub fn view_toggles(cx: &App) -> impl IntoElement {
    let frontend = crate::settings::frontend(cx);
    let (blindfold, zen) = (frontend.blindfold, frontend.zen_mode);
    let zen_label: SharedString = match crate::keys(&crate::ToggleZen, cx) {
        Some(key) => format!("Zen mode ({key})").into(),
        None => "Zen mode".into(),
    };
    let toggle =
        |id: &'static str, glyph: gpui_kit::component::Icon, label: SharedString, on: bool| {
            Button::new(id)
                .small()
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
                .icon(glyph)
                .tooltip(label.clone())
                .accessibility_label(label)
        };
    h_flex()
        .gap(px(2.))
        .child(
            toggle(
                "blindfold",
                icon("eye-off"),
                "Blindfold: hide the pieces".into(),
                blindfold,
            )
            .on_click(|_, _, cx| {
                crate::settings::change(cx, |s| s.frontend.blindfold = !s.frontend.blindfold)
            }),
        )
        .child(
            toggle("zen", icon("maximize"), zen_label, zen).on_click(|_, _, cx| {
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

/// A setting row that ends in its reset slot (`setting_reset`), as Settings' rows do.
pub fn resettable_row(info: Div, control: impl IntoElement, reset: Option<AnyElement>) -> Div {
    setting_row(
        info,
        h_flex()
            .gap(px(8.))
            .child(control)
            .child(setting_reset(reset)),
    )
}

/// The end of a setting row, where its reset button (`setting_reset_button`) shows while the
/// setting differs from its default: kept empty otherwise, so the control does not move when the
/// button appears under the pointer that changed it.
pub fn setting_reset(reset: Option<AnyElement>) -> Div {
    h_flex()
        .flex_none()
        .size(px(24.))
        .justify_center()
        .children(reset)
}

/// A setting's reset button: a small icon whose tooltip names the default, `label` naming the
/// setting too for a screen reader.
pub fn setting_reset_button(
    id: impl Into<ElementId>,
    tooltip: SharedString,
    label: SharedString,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon("rotate-ccw"))
        .tooltip(tooltip)
        .accessibility_label(label)
        .on_click(on_click)
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
