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
    let fill = look.fill;
    h_flex()
        .id(id)
        .when(fill, |row| row.w_full())
        // Pills too wide for one row take another rather than run past the edge.
        .flex_wrap()
        .p(BAR_PADDING)
        .gap(PILL_GAP)
        .rounded(radii::BOX)
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .children(tabs.into_iter().enumerate().map(move |(index, tab)| {
            pill(id, index, tab, selected, choose.clone(), &look, palette)
        }))
}

/// Around a bar's pills: 2px of padding inside a 1px border, on each side.
const BAR_PADDING: Pixels = px(2.);
const BAR_BORDER: Pixels = px(1.);
/// Between two pills, and between two rows of them.
const PILL_GAP: Pixels = px(4.);
/// A full-size pill's height, the side padding beside its label and the gap between its parts.
const PILL_HEIGHT: Pixels = px(26.);
const PILL_PADDING: Pixels = px(12.);
const PILL_INNER_GAP: Pixels = px(8.);
const PILL_ICON: Pixels = px(18.);
const PILL_TEXT: Pixels = px(14.);
const BADGE_TEXT: Pixels = px(11.);
const BADGE_PADDING: Pixels = px(4.);

/// Which of a bar's pills go on each of its rows: as many in order as fit at their labels'
/// widths, then another row. A page's tabs decide this as they draw, so the page knows the
/// bar's height before layout: a bar left to wrap itself is only as tall as it turns out once
/// laid out, a frame after whatever was sized below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PillRows {
    /// How many pills each row holds, in order; none for no pills.
    counts: Vec<usize>,
}

impl PillRows {
    /// The rows of full-size `tabs` in a bar `width` wide, measured with the window's font.
    pub fn new(tabs: &[Pill], width: Pixels, window: &Window) -> PillRows {
        let widths: Vec<Pixels> = tabs.iter().map(|tab| pill_width(tab, window)).collect();
        PillRows::fitting(&widths, width - (BAR_PADDING + BAR_BORDER) * 2.)
    }

    /// Pills `widths` wide, in rows at most `room` wide; a pill wider than the room has a row
    /// to itself.
    fn fitting(widths: &[Pixels], room: Pixels) -> PillRows {
        let mut counts = Vec::new();
        let mut used = px(0.);
        let mut count = 0;
        for &width in widths {
            if count > 0 && used + PILL_GAP + width > room {
                counts.push(count);
                count = 0;
            }
            used = if count == 0 {
                width
            } else {
                used + PILL_GAP + width
            };
            count += 1;
        }
        if count > 0 {
            counts.push(count);
        }
        PillRows { counts }
    }

    /// The bar's height, border included.
    pub fn height(&self) -> Pixels {
        let rows = self.counts.len().max(1) as f32;
        PILL_HEIGHT * rows + PILL_GAP * (rows - 1.) + (BAR_PADDING + BAR_BORDER) * 2.
    }
}

/// A full-size pill's width at its content's: its label in the window's font, its icon and its
/// badge, and the padding beside them, as `pill` lays them out.
fn pill_width(tab: &Pill, window: &Window) -> Pixels {
    let mut style = window.text_style();
    style.font_weight = FontWeight::MEDIUM;
    let text = |text: &SharedString, size: Pixels| {
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.clone(), size, &[run], None)
            .width
    };
    let mut width = PILL_PADDING * 2. + text(&tab.label, PILL_TEXT);
    if tab.icon.is_some() {
        width += PILL_ICON + PILL_INNER_GAP;
    }
    if let Some(badge) = &tab.badge {
        width += PILL_INNER_GAP + BADGE_PADDING * 2. + text(badge, BADGE_TEXT);
    }
    // The pill is laid out on whole pixels; a fraction short would squeeze its label.
    width.ceil()
}

/// A page's tabs above its board: full-width pills on the rows `rows` puts them in, each row's
/// pills filling it, so the bar is `rows.height()` tall whatever its labels.
pub fn pill_bar(
    id: &'static str,
    tabs: Vec<Pill>,
    rows: &PillRows,
    selected: usize,
    choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    let look = PillRow {
        compact: false,
        wrap: false,
        fill: true,
    };
    let mut tabs = tabs.into_iter().enumerate();
    let rows = rows.counts.iter().map(|&count| {
        h_flex().w_full().gap(PILL_GAP).children(
            tabs.by_ref()
                .take(count)
                .map(|(index, tab)| pill(id, index, tab, selected, choose.clone(), &look, palette))
                .collect::<Vec<_>>(),
        )
    });
    v_flex()
        .id(id)
        .w_full()
        .p(BAR_PADDING)
        .gap(PILL_GAP)
        .rounded(radii::BOX)
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .children(rows.collect::<Vec<_>>())
}

/// One pill: its icon, label and badge, chosen or not.
fn pill<Choose: Fn(usize, &mut Window, &mut App) + 'static>(
    id: &'static str,
    index: usize,
    tab: Pill,
    selected: usize,
    choose: Choose,
    look: &PillRow,
    palette: &Palette,
) -> impl IntoElement + use<Choose> {
    let &PillRow {
        compact,
        wrap,
        fill,
    } = look;
    let chosen = index == selected;
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
        .gap(if compact { px(6.) } else { PILL_INNER_GAP })
        .h(if compact { px(24.) } else { PILL_HEIGHT })
        .px(if compact { px(8.) } else { PILL_PADDING })
        .rounded(radii::BOX)
        .text_size(if compact { px(12.) } else { PILL_TEXT })
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
        .children(tab.icon.map(|glyph| glyph.size(PILL_ICON)))
        .child(tab.label)
        .children(tab.badge.map(|badge| {
            div()
                .px(BADGE_PADDING)
                .rounded(radii::SMALL)
                .text_size(BADGE_TEXT)
                .line_height(px(16.))
                .when(chosen, |badge| {
                    badge.bg(palette.bg).text_color(palette.highlighted)
                })
                .when(!chosen, |badge| {
                    badge.bg(palette.elevated).text_color(palette.toned)
                })
                .child(badge)
        }))
}

#[cfg(test)]
mod tests {
    use super::PillRows;
    use gpui_kit::{Pixels, px};

    fn counts(widths: &[f32], room: f32) -> Vec<usize> {
        let widths: Vec<Pixels> = widths.iter().map(|&width| px(width)).collect();
        PillRows::fitting(&widths, px(room)).counts
    }

    #[test]
    fn pills_take_another_row_only_when_the_next_does_not_fit() {
        // 100 + 4 + 100 + 4 + 100 = 308.
        assert_eq!(counts(&[100., 100., 100.], 308.), vec![3]);
        assert_eq!(counts(&[100., 100., 100.], 307.), vec![2, 1]);
        assert_eq!(counts(&[100., 100., 100.], 150.), vec![1, 1, 1]);
        // A pill wider than the bar still has its row.
        assert_eq!(counts(&[400., 100.], 300.), vec![1, 1]);
        assert_eq!(counts(&[], 300.), Vec::<usize>::new());
    }

    #[test]
    fn a_bar_is_as_tall_as_its_rows() {
        let rows = |n| PillRows { counts: vec![1; n] };
        assert_eq!(rows(1).height(), px(32.));
        assert_eq!(rows(2).height(), px(62.));
        assert_eq!(rows(0).height(), px(32.));
    }
}
