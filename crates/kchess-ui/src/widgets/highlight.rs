//! Search matches in text.

use super::*;

/// Text with the characters a search matched in the accent colour, so a result shows why it is
/// there: ⌘K and the lists searched in place (History, Studies, Openings) draw their names with
/// it. `ranges` are inclusive character ranges, as `quick_pick` gives them; the text keeps its
/// own style (and truncation) elsewhere.
pub fn highlighted(
    text: impl Into<SharedString>,
    ranges: &[(usize, usize)],
    palette: &Palette,
) -> StyledText {
    let text = text.into();
    let style = HighlightStyle {
        color: Some(palette.primary),
        font_weight: Some(FontWeight::MEDIUM),
        ..HighlightStyle::default()
    };
    let highlights: Vec<_> = crate::quick_pick::byte_ranges(&text, ranges)
        .into_iter()
        .map(|range| (range, style))
        .collect();
    StyledText::new(text).with_highlights(highlights)
}
