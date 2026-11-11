//! Pill tabs and pill choices.

use super::*;

/// A setting's choices: pills as wide as their labels, at the row's right edge.
pub fn setting_pills(
    id: &'static str,
    tabs: Vec<(Option<Icon>, SharedString)>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    pill_row(
        id,
        tabs.into_iter()
            .map(|(glyph, label)| Pill {
                icon: glyph,
                label,
                badge: None,
                disabled: false,
            })
            .collect(),
        selected,
        choose,
        PillRow {
            compact: false,
            wrap: false,
            fill: false,
        },
        palette,
    )
}
/// A full-width row of choices, the chosen one a step up from the row: the
/// accent is left to the page's main action. `choose` gets the index picked.
pub fn pill_tabs(
    id: &'static str,
    tabs: Vec<(Option<Icon>, SharedString)>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    pills(
        id,
        tabs.into_iter()
            .map(|(glyph, label)| Pill {
                icon: glyph,
                label,
                badge: None,
                disabled: false,
            })
            .collect(),
        selected,
        choose,
        false,
        palette,
    )
}

/// One choice of `pills`: its label, an icon or a count beside it, and whether it can be chosen.
pub struct Pill {
    pub icon: Option<Icon>,
    pub label: SharedString,
    pub badge: Option<SharedString>,
    pub disabled: bool,
}

/// Pill tabs, full width or as wide as its choices.
pub fn pills(
    id: &'static str,
    tabs: Vec<Pill>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    compact: bool,
    palette: &Palette,
) -> impl IntoElement {
    pill_row(
        id,
        tabs,
        selected,
        choose,
        PillRow {
            compact,
            wrap: false,
            fill: !compact,
        },
        palette,
    )
}

/// A page header's tabs: full-size pills, only as wide as their labels.
pub fn header_pills(
    id: &'static str,
    tabs: Vec<Pill>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    pill_row(
        id,
        tabs,
        selected,
        choose,
        PillRow {
            compact: false,
            wrap: false,
            fill: false,
        },
        palette,
    )
}

/// Full-width pills that take another row when they do not fit on one, each row's pills
/// filling it (the coordinates drill's modes).
pub fn wrapping_pills(
    id: &'static str,
    tabs: Vec<Pill>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    pill_row(
        id,
        tabs,
        selected,
        choose,
        PillRow {
            compact: false,
            wrap: true,
            fill: true,
        },
        palette,
    )
}

/// How a pill row is drawn: small pills, wrapping to more rows, or filling the width.
struct PillRow {
    compact: bool,
    wrap: bool,
    fill: bool,
}

fn pill_row(
    id: &'static str,
    tabs: Vec<Pill>,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    look: PillRow,
    palette: &Palette,
) -> impl IntoElement {
    let PillRow {
        compact,
        wrap,
        fill,
    } = look;
    h_flex()
        .id(id)
        .when(fill, |row| row.w_full())
        // Pills too wide for one row take another rather than run past the edge.
        .flex_wrap()
        .p(px(2.))
        .gap(px(4.))
        .rounded(radii::MD)
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .children(tabs.into_iter().enumerate().map(move |(index, tab)| {
            let chosen = index == selected;
            let choose = choose.clone();
            h_flex()
                .id((id, index))
                .test_support()
                .role(Role::Tab)
                .keyboard_focus(palette.primary)
                .aria_label(tab.label.clone())
                .aria_selected(chosen)
                .when(fill && !wrap, |tab| tab.flex_1())
                .when(wrap, |tab| tab.flex_grow(1.))
                .justify_center()
                .gap(px(if compact { 6. } else { 8. }))
                .h(px(if compact { 24. } else { 26. }))
                .px(px(if compact { 8. } else { 12. }))
                .rounded(radii::SM)
                .text_size(px(if compact { 12. } else { 14. }))
                .font_weight(FontWeight::MEDIUM)
                .when(chosen, |tab| {
                    tab.bg(palette.accented).text_color(palette.highlighted)
                })
                .when(!chosen && tab.disabled, |tab| {
                    tab.text_color(palette.text_muted).opacity(0.75)
                })
                .when(!chosen && !tab.disabled, |tab| {
                    tab.cursor_pointer()
                        .text_color(palette.text_muted)
                        .pressable_text(palette.highlighted)
                })
                .when(!tab.disabled, |row| {
                    row.on_click(move |_, window, cx| choose(index, window, cx))
                })
                .children(tab.icon.map(|glyph| glyph.size(px(18.))))
                .child(tab.label)
                .children(tab.badge.map(|badge| {
                    div()
                        .px(px(4.))
                        .rounded(radii::XS)
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .when(chosen, |badge| {
                            badge.bg(palette.bg).text_color(palette.highlighted)
                        })
                        .when(!chosen, |badge| {
                            badge.bg(palette.elevated).text_color(palette.toned)
                        })
                        .child(badge)
                }))
        }))
}
