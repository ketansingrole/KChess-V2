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
        .rounded(radii::XL)
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

/// A button that reads as text: small, but still a 24 px target (WCAG 2.5.8),
/// which its text alone is not.
pub fn text_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).text().xsmall().min_h(px(24.))
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
        .rounded(radii::XL)
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

/// A piece colour, the same light or dark disc in either theme.
pub fn side_dot(color: shakmaty::Color) -> Div {
    div()
        .size(px(9.))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(rgb(0x6b6b6b))
        .bg(rgb(if color == shakmaty::Color::White {
            0xf4f4f2
        } else {
            0x1f1f1f
        }))
}
