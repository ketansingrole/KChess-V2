//! Choice and study cards.

use super::*;

/// A select's entry, known by its value: two entries may share a label (a study and an opening).
#[derive(Clone)]
pub struct Choice {
    pub value: SharedString,
    pub label: SharedString,
}

impl gpui_kit::component::searchable_list::SearchableListItem for Choice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }
}

impl Choice {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// What a study card shows: a study or an opening as a card in a grid.
#[derive(IntoElement)]
pub struct StudyCard {
    pub id: String,
    pub name: String,
    /// The position as a small board; a book stands in when there is none.
    pub board: Option<AnyView>,
    /// The first detail line (where it is kept, or an opening's code).
    pub location: String,
    /// The second line (its players and event, or an opening's moves).
    pub description: Option<String>,
    /// "Edited 2 days ago".
    pub edited: Option<String>,
    /// Detail rows kept for every card, so cards in a grid line up (3 with dates, 2 without).
    pub rows: usize,
    pub menu: Option<AnyElement>,
    pub action: AnyElement,
    pub width: Pixels,
    /// Clicking its picture or its name opens it.
    pub on_open: OpenHandler,
}

impl RenderOnce for StudyCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        study_card(self, cx.global::<Palette>())
    }
}

fn study_card(card: StudyCard, palette: &Palette) -> AnyElement {
    let (width, open) = (card.width, card.on_open.clone());
    let open_title = open.clone();
    let row = |text: Option<String>| {
        div()
            .h(px(15.))
            .text_size(px(12.))
            .text_color(palette.text_muted)
            .truncate()
            .children(text)
    };
    let mut details = vec![
        row(Some(card.location)),
        row(card.description),
        row(card.edited),
    ];
    details.truncate(card.rows);
    v_flex()
        .id(SharedString::from(format!("card-{}", card.id)))
        .w(width)
        .overflow_hidden()
        .border_1()
        .border_color(palette.border)
        .rounded(radii::XL)
        .bg(palette.elevated)
        .child(
            div()
                .id(SharedString::from(format!("card-picture-{}", card.id)))
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .aria_label(format!("Open {}", card.name))
                .w_full()
                .aspect_ratio(1.)
                .flex()
                .items_center()
                .justify_center()
                // Inside the card's border; GPUI does not clip to the card's rounded corners.
                .rounded_t(radii::XL - px(1.))
                .bg(palette.bg)
                .cursor_pointer()
                .on_click(move |_, window, cx| open(window, cx))
                .child(match card.board {
                    Some(board) => {
                        // A still picture: kept from frame to frame until it changes.
                        let mut style = StyleRefinement::default();
                        style.size.width = Some(relative(1.).into());
                        style.size.height = Some(relative(1.).into());
                        div()
                            .size_full()
                            .child(board.cached(style))
                            .into_any_element()
                    }
                    None => icon("book-open")
                        .size(px(48.))
                        .text_color(palette.text_muted)
                        .into_any_element(),
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .p(px(12.))
                .gap(px(8.))
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(8.))
                        .child(
                            div()
                                .id(SharedString::from(format!("card-name-{}", card.id)))
                                .role(Role::Button)
                                .keyboard_focus(palette.primary)
                                .aria_label(card.name.clone())
                                .flex_1()
                                .min_w_0()
                                .min_h(px(44.))
                                .text_size(px(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .line_height(relative(1.4))
                                // `-webkit-line-clamp: 2`: a longer name ends in "…".
                                .line_clamp(2)
                                .text_ellipsis()
                                .cursor_pointer()
                                .on_click(move |_, window, cx| open_title(window, cx))
                                .child(card.name),
                        )
                        .children(card.menu),
                )
                .child(v_flex().gap(px(8.)).children(details))
                .child(div().mt_auto().pt(px(4.)).child(card.action)),
        )
        .into_any_element()
}
