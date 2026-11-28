//! The side panel's building blocks: headings, hints, cards, columns and empty states.

use super::*;

/// The panel beside a board.
pub fn side_panel(palette: &Palette) -> Div {
    v_flex()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .p(px(16.))
        .gap(px(12.))
        .border_1()
        .border_color(palette.border)
        .rounded(radii::BOX)
        .bg(palette.elevated)
}

/// A panel's or section's heading.
pub fn heading(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.highlighted)
        .child(text.into())
}

/// A short muted line under a heading.
pub fn hint(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(palette.text_muted)
        .child(text.into())
}

/// One choice of a row of filters (a speed, a rating) that is on or off: filled while on.
pub fn toggle_button(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Button {
    Button::new(id)
        .small()
        .label(label.into())
        .when(on, |button| button.primary())
        .when(!on, |button| button.ghost())
}

/// A button that reads as text: small, but still a 24 px target (WCAG 2.5.8),
/// which its text alone is not.
pub fn text_button(id: impl Into<ElementId>) -> Button {
    // gpui-lints: allow(unnamed_button) its callers name it, with a label or `trailing_icon`
    Button::new(id).text().xsmall().min_h(px(24.))
}

/// `button` reading `label` with `glyph` after it (Next →, View all →), named as it reads: a
/// kit button's own icon goes before its label, and text drawn as a child names nothing.
pub fn trailing_icon(button: Button, label: &'static str, glyph: &'static str) -> Button {
    button.accessibility_label(label).child(
        h_flex()
            .gap(px(6.))
            .child(label)
            .child(icon(glyph).size(px(14.))),
    )
}

/// The way out of a failed load that fills its place: one primary Try again.
pub fn try_again(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .primary()
        .icon(icon("rotate-cw"))
        .label("Try again")
}

/// A load that failed inside a card: what went wrong, in `color`, and a small Try again.
pub fn retry_line(
    id: &'static str,
    message: impl Into<SharedString>,
    color: Hsla,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    h_flex()
        .flex_wrap()
        .gap(px(8.))
        .text_size(px(13.))
        .text_color(color)
        .child(message.into())
        .child(
            text_button(id)
                .icon(icon("rotate-cw"))
                .label("Try again")
                .on_click(retry),
        )
}

/// Something loading: the turning spinner and a muted line ("Loading…"), as a hint reads.
pub fn loading(text: impl Into<SharedString>, palette: &Palette) -> Div {
    h_flex()
        .gap(px(8.))
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(palette.text_muted)
        .child(spinner(px(12.)))
        .child(text.into())
}

/// A page loading its cards: `columns` columns of placeholder cards, each a heading and result
/// rows in the kit's pulsing skeleton, so the page keeps its shape until they come. Read as a
/// status named `text` ("Loading insights…").
pub fn loading_cards(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    columns: usize,
    palette: &Palette,
) -> impl IntoElement {
    use gpui_kit::component::skeleton::Skeleton;
    // Rows per card, varied so the columns read as cards of different lengths.
    const ROWS: [usize; 6] = [2, 4, 3, 5, 3, 4];
    let columns = columns.max(1);
    // The kit's skeleton is the accented background, one step from a card's in light but too
    // close to it in dark, where its bars all but vanished: dark takes the next step up.
    let bar = if palette.dark {
        palette.border_accented
    } else {
        palette.accented
    };
    let card = |rows: usize| {
        card(palette)
            .w_full()
            .gap(px(12.))
            .child(
                Skeleton::new()
                    .bg(bar)
                    .w(px(120.))
                    .h(px(14.))
                    .rounded(radii::BOX)
                    .mb(px(4.)),
            )
            .children((0..rows).map(|row| {
                v_flex()
                    .gap(px(6.))
                    .child(
                        Skeleton::new()
                            .bg(bar)
                            .w(relative(if row % 2 == 0 { 0.45 } else { 0.6 }))
                            .h(px(10.))
                            .rounded(radii::BOX),
                    )
                    .child(
                        Skeleton::new()
                            .bg(bar.opacity(0.5))
                            .h(px(8.))
                            .rounded(radii::BOX),
                    )
            }))
    };
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(text.into())
        .items_start()
        .gap(px(16.))
        .children((0..columns).map(|column| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(16.))
                .children((0..2).map(|n| card(ROWS[(column * 2 + n) % ROWS.len()])))
        }))
}

/// "Move 3 · You play White · Your move": parts joined by middle dots that wrap whole, so a
/// line never ends on a dot or breaks inside a part.
pub fn dotted(parts: impl IntoIterator<Item = impl Into<SharedString>>) -> Div {
    h_flex()
        .flex_wrap()
        .children(parts.into_iter().enumerate().map(|(index, part)| {
            let part: SharedString = part.into();
            div().whitespace_nowrap().child(if index == 0 {
                part
            } else {
                format!("\u{a0}·\u{a0}{part}").into()
            })
        }))
}

/// A section title with a muted note beside it, cut short when it does not fit.
pub fn section_title(
    title: &'static str,
    count: Option<String>,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .items_baseline()
        .justify_between()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title),
        )
        .children(count.map(|count| {
            div()
                .min_w_0()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(count)
        }))
}
/// A grid's columns for a width: as many as fit at
/// `min`, `gap` apart, sharing the width.
pub fn columns(width: Pixels, min: Pixels, gap: Pixels) -> (usize, Pixels) {
    let count = ((width + gap) / (min + gap)).floor().max(1.) as usize;
    let item = (width - gap * (count as f32 - 1.)) / count as f32;
    (count, item)
}

/// A share of a rounded bar, rounded at the bar's (`first`, `last`) ends: GPUI clips a child to
/// its parent's rectangle, not to the parent's rounded corners. `radius` is the bar's, less any
/// border.
pub fn bar_segment(segment: Div, (first, last): (bool, bool), radius: Pixels) -> Div {
    segment
        .when(first, |segment| segment.rounded_l(radius))
        .when(last, |segment| segment.rounded_r(radius))
}

/// A bordered, raised box with the page's standard padding.
pub fn card(palette: &Palette) -> Div {
    v_flex()
        .p(px(20.))
        .rounded(radii::BOX)
        .border_1()
        .border_color(palette.border)
        .bg(palette.elevated)
}
/// An icon, a title and an optional line under it; the caller adds the actions.
pub fn empty_state(
    glyph: Icon,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    palette: &Palette,
) -> Div {
    v_flex()
        .items_center()
        .gap(px(8.))
        .child(glyph.size(px(24.)).text_color(palette.text_muted))
        .child(
            div()
                .text_size(px(16.))
                .text_center()
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title.into()),
        )
        .children(description.map(|description| {
            div()
                .text_size(px(14.))
                .text_center()
                .text_color(palette.text_muted)
                .child(description)
        }))
}
