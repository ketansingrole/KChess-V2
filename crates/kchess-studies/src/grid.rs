//! The studies' cards in a grid, of which only the rows in view are made.

use super::*;

/// The rows' heights as laid out at one width of card: at another, a card's text wraps
/// otherwise, so a row measured before (one made at the first frame's width, then scrolled out of
/// view) no longer stands for its height.
#[derive(Default)]
pub(super) struct RowHeights {
    card_width: Pixels,
    pub(super) heights: Vec<Pixels>,
}

impl RowHeights {
    /// `rows` rows of cards `card_width` wide: measurements at another width are let go.
    pub(super) fn fit(&mut self, card_width: Pixels, rows: usize) {
        if self.card_width != card_width {
            self.card_width = card_width;
            self.heights.clear();
        }
        self.heights.resize(rows, Pixels::ZERO);
    }

    /// A row laid out at `height`; whether that is new.
    pub(super) fn record(&mut self, row: usize, height: Pixels) -> bool {
        match self.heights.get_mut(row) {
            Some(kept) if *kept != height => {
                *kept = height;
                true
            }
            _ => false,
        }
    }
}

/// A row of cards' height before any is laid out: the square picture and the text under it.
pub fn estimated_row(card_width: Pixels) -> Pixels {
    card_width + px(150.)
}

/// Each row's height as measured, or as tall as the first measured row for one not yet laid out
/// (`ZERO`); `estimate` before any is.
pub(super) fn resolved_heights(measured: &[Pixels], estimate: Pixels) -> Vec<Pixels> {
    let usual = measured
        .iter()
        .copied()
        .find(|height| *height > Pixels::ZERO)
        .unwrap_or(estimate);
    measured
        .iter()
        .map(|height| {
            if *height > Pixels::ZERO {
                *height
            } else {
                usual
            }
        })
        .collect()
}

/// What of the grid to make, from its top: what the page's viewport shows of it and as much again
/// (at least 200 px) above and below, so a scroll finds its cards made. Before the first layout,
/// the window's height from the top.
pub(super) fn view_span(
    viewport: Bounds<Pixels>,
    grid: Bounds<Pixels>,
    window: Pixels,
) -> (Pixels, Pixels) {
    let (top, bottom) = if viewport.size.height > Pixels::ZERO && grid.size.height > Pixels::ZERO {
        (viewport.top() - grid.top(), viewport.bottom() - grid.top())
    } else {
        (Pixels::ZERO, window)
    };
    let margin = (bottom - top).max(px(200.));
    (top - margin, bottom + margin)
}

/// The rows of a grid to make: those reaching into `top..bottom` (from the grid's top), with the
/// heights the rows left out take above and below them. Rows are `gap` apart.
pub(super) fn rows_in_view(
    heights: &[Pixels],
    gap: Pixels,
    top: Pixels,
    bottom: Pixels,
) -> (std::ops::Range<usize>, Pixels, Pixels) {
    let (mut first, mut last) = (heights.len(), 0);
    let (mut above, mut below) = (Pixels::ZERO, Pixels::ZERO);
    let mut y = Pixels::ZERO;
    for (row, height) in heights.iter().enumerate() {
        let space = if row > 0 { gap } else { Pixels::ZERO };
        let (start, end) = (y + space, y + space + *height);
        if end < top {
            above = end;
        } else if start <= bottom {
            first = first.min(row);
            last = row + 1;
        } else {
            below += space + *height;
        }
        y = end;
    }
    if first >= last {
        // Nothing reaches into view (an empty grid): every row is space above.
        return (0..0, y, Pixels::ZERO);
    }
    (first..last, above, below)
}

impl StudiesPage {
    pub(super) fn render_cards(
        &mut self,
        studies: Vec<Study>,
        found: Vec<String>,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let now = now_ms();
        // Pictures of studies no longer listed (edited, removed, filtered out) are let go; those
        // of listed studies scrolled out of view are kept for when they come back.
        let listed: std::collections::HashSet<String> = studies
            .iter()
            .filter_map(|study| self.summary(&study.pgn).map(|summary| summary.fen))
            .collect();
        self.previews.retain(|fen| listed.contains(fen));
        let rows = studies.len().div_ceil(count);
        let heights = {
            let mut measured = self.row_heights.borrow_mut();
            measured.fit(card_width, rows);
            resolved_heights(&measured.heights, estimated_row(card_width))
        };
        let (top, bottom) = view_span(
            self.scroll.bounds(),
            self.grid.bounds(),
            window.viewport_size().height,
        );
        let (shown, above, below) = rows_in_view(&heights, CARD_GAP, top, bottom);
        let mut grid_rows: Vec<AnyElement> = Vec::new();
        for row in shown.clone() {
            let cards: Vec<AnyElement> = studies
                .iter()
                .skip(row * count)
                .take(count)
                .map(|study| {
                    let (id, name) = (study.id.clone(), study.name.clone());
                    let summary = self.summary(&study.pgn);
                    let board = summary
                        .as_ref()
                        .map(|summary| self.preview(&summary.fen, cx).into());
                    let event = summary
                        .as_ref()
                        .map(|summary| summary.event.as_str())
                        .filter(|event| *event != name);
                    let players = summary.as_ref().map(|summary| summary.players.as_str());
                    let description = [players, event]
                        .into_iter()
                        .flatten()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let chapters = study.chapters.len();
                    let cloud = study.cloud.as_ref();
                    let location = format!(
                        "{}{}",
                        if cloud.is_none() {
                            "On this device"
                        } else {
                            "Offline · Lichess"
                        },
                        if chapters > 0 {
                            format!(
                                " · {chapters} {}",
                                if chapters == 1 { "chapter" } else { "chapters" }
                            )
                        } else {
                            String::new()
                        }
                    );
                    let upload_label = if cloud.is_some_and(|cloud| cloud.structure_changed) {
                        "Upload as new cloud study"
                    } else if cloud.is_none() {
                        "Upload to Lichess"
                    } else {
                        "Upload changes"
                    };
                    let uploading = self.uploading.as_deref() == Some(id.as_str());
                    let page = cx.entity().downgrade();
                    let upload_id = id.clone();
                    let open_id = id.clone();
                    widgets::StudyCard {
                        id: id.clone(),
                        name,
                        board,
                        location,
                        description: (!description.is_empty()).then_some(description),
                        edited: Some(study.updated_at_ms())
                            .filter(|at| *at > 0.)
                            .map(|at| format!("Edited {}", time_ago(at, now))),
                        rows: 3,
                        menu: Some(self.menu(study).into_any_element()),
                        action: Button::new(SharedString::from(format!("upload-{id}")))
                            .outline()
                            .small()
                            .w_full()
                            .icon(icon("cloud-upload"))
                            .label(upload_label)
                            .loading(uploading)
                            .disabled(self.uploading.is_some())
                            .on_click(move |_, window, cx| {
                                let _ = page.update(cx, |page, cx| {
                                    page.upload(upload_id.clone(), false, window, cx)
                                });
                            })
                            .into_any_element(),
                        width: card_width,
                        on_open: std::rc::Rc::new(move |window, cx| {
                            Self::open(open_id.clone(), window, cx)
                        }),
                        found: found.clone(),
                    }
                    .into_any_element()
                })
                .collect();
            let heights = std::rc::Rc::clone(&self.row_heights);
            let entity = cx.entity().downgrade();
            // The row's height as laid out, for the rows' spacing while it is out of view.
            let probe = canvas(
                move |area, window, _| {
                    if heights.borrow_mut().record(row, area.size.height) {
                        let entity = entity.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = entity.update(cx, |_, cx| cx.notify());
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full();
            grid_rows.push(
                h_flex()
                    .relative()
                    .items_stretch()
                    .gap(CARD_GAP)
                    .when(row > 0, |line| line.mt(CARD_GAP))
                    .child(probe)
                    .children(cards)
                    .into_any_element(),
            );
        }
        v_flex()
            .relative()
            .child(self.grid.probe(cx))
            .child(div().h(above))
            .children(grid_rows)
            .child(div().h(below))
            .into_any_element()
    }
}
