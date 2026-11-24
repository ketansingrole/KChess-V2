//! A list's pages: previous, the first page, the pages around the current one, the last page,
//! next, with "…" where pages are left out, so a list of thousands of pages keeps one short row.
//! It may also say which items show ("21–40 of 4,512 games") and offer the page size.

use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::select::{Select, SelectDelegate, SelectState};

use super::*;

/// One place in the row of page buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagerSlot {
    /// A page by its index from 0.
    Page(usize),
    /// Pages left out between two shown ones.
    Gap,
}

/// The pages shown out of `pages` with `current` chosen: the first, the last and the ones beside
/// the current one, with a gap wherever pages are left out. It takes the same few steps for any
/// number of pages.
pub fn pager_slots(current: usize, pages: usize) -> Vec<PagerSlot> {
    if pages == 0 {
        return Vec::new();
    }
    let last_page = pages - 1;
    let current = current.min(last_page);
    let mut shown = vec![
        0,
        current.saturating_sub(1),
        current,
        current + 1,
        last_page,
    ];
    shown.retain(|page| *page <= last_page);
    shown.sort_unstable();
    shown.dedup();
    let mut slots = Vec::with_capacity(shown.len() * 2);
    let mut previous: Option<usize> = None;
    for page in shown {
        if previous.is_some_and(|previous| page > previous + 1) {
            slots.push(PagerSlot::Gap);
        }
        slots.push(PagerSlot::Page(page));
        previous = Some(page);
    }
    slots
}

/// Which items a page shows: "21–40 of 4,512 games", or "0 games".
pub fn page_range(page: usize, page_size: usize, total: usize) -> String {
    if total == 0 || page_size == 0 {
        return format_counted(total as f64, "game", "games");
    }
    let from = page.saturating_mul(page_size).saturating_add(1).min(total);
    let to = from.saturating_add(page_size - 1).min(total);
    format!(
        "{}–{} of {}",
        format_count(from as f64),
        format_count(to as f64),
        format_counted(total as f64, "game", "games")
    )
}

/// What choosing a page runs, with the page's index from 0.
type PageHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

/// A list's pager (`pager`).
#[derive(IntoElement)]
pub struct Pager {
    id: SharedString,
    current: usize,
    pages: usize,
    on_page: PageHandler,
    range: Option<SharedString>,
    page_size: Option<AnyElement>,
}

/// The page buttons of a list of `pages` pages with `current` shown; `id` tells this pager's
/// buttons from another's. `on_page` takes the page chosen, as `cx.listener` gives it.
pub fn pager(
    id: impl Into<SharedString>,
    current: usize,
    pages: usize,
    on_page: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> Pager {
    Pager {
        id: id.into(),
        current,
        pages: pages.max(1),
        on_page: Rc::new(on_page),
        range: None,
        page_size: None,
    }
}

impl Pager {
    /// Also says which games the page shows (`page_range`), at the row's start.
    pub fn range(mut self, page_size: usize, total: usize) -> Pager {
        self.range = Some(page_range(self.current, page_size, total).into());
        self
    }

    /// Also offers how many games a page shows, before the page buttons.
    pub fn page_size<D: SelectDelegate + 'static>(
        mut self,
        sizes: &Entity<SelectState<D>>,
    ) -> Pager {
        self.page_size = Some(
            div()
                .w(px(130.))
                .child(
                    Select::new(sizes)
                        .small()
                        .accessibility_label("Games per page"),
                )
                .into_any_element(),
        );
        self
    }
}

impl RenderOnce for Pager {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Pager {
            id,
            current,
            pages,
            on_page,
            range,
            page_size,
        } = self;
        let current = current.min(pages - 1);
        let go = |label: SharedString, target: usize, disabled: bool, chosen: bool| {
            let on_page = on_page.clone();
            Button::new(SharedString::from(format!("{id}-page-{label}")))
                .small()
                .when(chosen, |button| button.primary())
                .when(!chosen, |button| button.outline())
                .label(label)
                .disabled(disabled)
                .on_click(move |_, window, cx| on_page(&target, window, cx))
        };
        let mut buttons = h_flex().gap(px(4.)).child(
            go(
                SharedString::new_static("‹"),
                current.saturating_sub(1),
                current == 0,
                false,
            )
            .accessibility_label("Previous page"),
        );
        for slot in pager_slots(current, pages) {
            buttons = match slot {
                PagerSlot::Page(page) => buttons.child(go(
                    (page + 1).to_string().into(),
                    page,
                    false,
                    page == current,
                )),
                PagerSlot::Gap => buttons.child(div().px(px(4.)).child("…")),
            };
        }
        let buttons = buttons.child(
            go(
                SharedString::new_static("›"),
                (current + 1).min(pages - 1),
                current + 1 >= pages,
                false,
            )
            .accessibility_label("Next page"),
        );
        let framed = range.is_some() || page_size.is_some();
        h_flex()
            .gap(px(8.))
            .children(range.map(|range| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(range)
            }))
            .when(framed, |row| row.child(div().flex_1()))
            .children(page_size)
            .child(buttons)
    }
}
