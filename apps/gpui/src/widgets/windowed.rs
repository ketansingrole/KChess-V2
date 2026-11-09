//! A long list of rows in a scrolled page, of which only the rows in view (and as much again
//! around them) are made, laid out and drawn; spaces as tall as the rows left out keep the list's
//! height and the page's scroll. Fifty study cards (the library's limit) and the openings' families
//! were most of drawing their pages.
//!
//! Each row made records its height as laid out, for one width (at another, text wraps
//! otherwise); a row not yet made is taken to be as tall as the first measured, or as the page's
//! estimate before any is. A scroll that asks for no frame (the page's own) still brings its rows:
//! the list's place in the window is measured every frame, and a move draws the page again.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::play_layout::Measured;

/// The list's place and its rows' heights, kept from frame to frame by the page that shows it.
#[derive(Clone, Default)]
pub struct Windowed {
    area: Measured,
    heights: Rc<RefCell<RowHeights>>,
}

impl Windowed {
    /// Rows `0..count`, `gap` apart, of a list in the page scrolled by `scroll`: `row` makes each
    /// row in view. Heights measured at a `width` other than this one are let go; `estimate` is a
    /// row's height before any is measured.
    #[allow(clippy::too_many_arguments)]
    pub fn render<V: 'static>(
        &self,
        count: usize,
        width: Pixels,
        estimate: Pixels,
        gap: Pixels,
        scroll: &ScrollHandle,
        window: &Window,
        cx: &mut Context<V>,
        mut row: impl FnMut(usize, &mut Context<V>) -> AnyElement,
    ) -> AnyElement {
        let heights = {
            let mut measured = self.heights.borrow_mut();
            measured.fit(width, count);
            resolved_heights(&measured.heights, estimate)
        };
        let (top, bottom) = view_span(
            scroll.bounds(),
            self.area.bounds(),
            window.viewport_size().height,
        );
        let (shown, above, below) = rows_in_view(&heights, gap, top, bottom);
        let mut rows: Vec<AnyElement> = Vec::new();
        for index in shown {
            let element = row(index, cx);
            let heights = Rc::clone(&self.heights);
            let entity = cx.entity().downgrade();
            // The row's height as laid out, for the rows' spacing while it is out of view.
            let probe = canvas(
                move |area, window, _| {
                    if heights.borrow_mut().record(index, area.size.height) {
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
            rows.push(
                div()
                    .relative()
                    .when(index > 0, |line| line.mt(gap))
                    .child(probe)
                    .child(element)
                    .into_any_element(),
            );
        }
        v_flex()
            .relative()
            .child(self.area.probe(cx))
            .child(div().h(above))
            .children(rows)
            .child(div().h(below))
            .into_any_element()
    }

    /// The rows' heights as measured (`ZERO` for a row not yet made), and the list's height.
    #[cfg(test)]
    pub(crate) fn layout(&self) -> (Vec<Pixels>, Pixels) {
        (
            self.heights.borrow().heights.clone(),
            self.area.bounds().size.height,
        )
    }
}

/// Each row's height as measured, or as tall as the first measured row for one not yet laid out
/// (`ZERO`); `estimate` before any is.
pub(crate) fn resolved_heights(measured: &[Pixels], estimate: Pixels) -> Vec<Pixels> {
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

/// What of the list to make, from its top: what the page's viewport shows of it and as much again
/// (at least 200 px) above and below, so a scroll finds its rows made. Before the first layout,
/// the window's height from the top.
pub(crate) fn view_span(
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
pub(crate) fn rows_in_view(
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

/// The rows' heights as laid out at one width: at another, a row's text wraps otherwise, so a
/// row measured before (one made at the first frame's width, then scrolled out of view) no longer
/// stands for its height.
#[derive(Default)]
pub(crate) struct RowHeights {
    width: Pixels,
    heights: Vec<Pixels>,
}

impl RowHeights {
    /// `rows` rows laid out `width` wide: measurements at another width are let go.
    pub(crate) fn fit(&mut self, width: Pixels, rows: usize) {
        if self.width != width {
            self.width = width;
            self.heights.clear();
        }
        self.heights.resize(rows, Pixels::ZERO);
    }

    /// A row laid out at `height`; whether that is new.
    pub(crate) fn record(&mut self, row: usize, height: Pixels) -> bool {
        match self.heights.get_mut(row) {
            Some(kept) if *kept != height => {
                *kept = height;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolved_heights, rows_in_view, view_span};
    use gpui_kit::{Bounds, point, px, size};

    /// Only the rows reaching into view are made; the spaces left above and below keep the grid
    /// as tall as it is, row gaps included, so scrolling lands where it did.
    #[test]
    fn only_the_rows_in_view_are_made() {
        let heights = [px(100.); 10];
        let gap = px(10.);
        // Rows start at 0, 110, 220, … and end 100 later; the grid is 1090 tall.
        let (rows, above, below) = rows_in_view(&heights, gap, px(0.), px(250.));
        assert_eq!((rows, above, below), (0..3, px(0.), px(770.)));
        // A row starting where the view ends is in it.
        assert_eq!(rows_in_view(&heights, gap, px(0.), px(330.)).0, 0..4);
        let (rows, above, below) = rows_in_view(&heights, gap, px(300.), px(500.));
        assert_eq!((rows, above, below), (2..5, px(210.), px(550.)));
        // Above + made rows (with their gaps) + below is the whole grid.
        let made = px(100.) * 3. + gap * 3.;
        assert_eq!(above + made + below, px(1090.));
        // A row ending where the view starts is still in it.
        let (rows, above, below) = rows_in_view(&heights, gap, px(980.), px(2000.));
        assert_eq!((rows, above, below), (8..10, px(870.), px(0.)));
        // Scrolled past the grid, and an empty grid.
        let (rows, above, _) = rows_in_view(&heights, gap, px(5000.), px(6000.));
        assert_eq!((rows, above), (0..0, px(1090.)));
        assert_eq!(rows_in_view(&[], gap, px(0.), px(100.)).0, 0..0);
        // A row taller than the others counts as tall as it is.
        let mixed = [px(100.), px(300.), px(100.)];
        assert_eq!(
            rows_in_view(&mixed, gap, px(450.), px(600.)),
            (2..3, px(410.), px(0.))
        );
    }

    /// Rows measured at one card width are let go at another: a row made at the first frame's
    /// width and then scrolled out of view kept that height, and the grid came out 18.5 px short.
    #[test]
    fn row_heights_are_kept_for_one_card_width() {
        let mut rows = super::RowHeights::default();
        rows.fit(px(210.), 3);
        assert!(rows.record(1, px(387.)));
        assert!(!rows.record(1, px(387.)), "the same height is nothing new");
        assert!(!rows.record(5, px(10.)), "no such row");
        rows.fit(px(210.), 4);
        assert_eq!(rows.heights, [px(0.), px(387.), px(0.), px(0.)]);
        rows.fit(px(240.), 4);
        assert_eq!(rows.heights, [px(0.); 4]);
    }

    /// A row not yet laid out is taken to be as tall as the first one measured, and every row as
    /// tall as the estimate before any is.
    #[test]
    fn rows_not_laid_out_take_the_measured_height() {
        let zero = px(0.);
        assert_eq!(
            resolved_heights(&[zero, px(320.), px(330.), zero], px(100.)),
            [px(320.), px(320.), px(330.), px(320.)]
        );
        assert_eq!(
            resolved_heights(&[zero, zero], px(100.)),
            [px(100.), px(100.)]
        );
    }

    /// The span to make is what the viewport shows of the grid, measured from the grid's top, and
    /// as much again above and below (at least 200 px); before the first layout, the window.
    #[test]
    fn the_span_made_is_the_view_and_a_margin() {
        let at = |top: f32, height: f32| Bounds {
            origin: point(px(0.), px(top)),
            size: size(px(800.), px(height)),
        };
        // The viewport shows 100..700 of the window; the grid starts 1000 above it (scrolled).
        assert_eq!(
            view_span(at(100., 600.), at(-900., 5000.), px(800.)),
            (px(400.), px(2200.))
        );
        // A short viewport keeps a 200 px margin.
        assert_eq!(
            view_span(at(0., 50.), at(0., 5000.), px(800.)),
            (px(-200.), px(250.))
        );
        // Not laid out yet: the window from the top, and as much again.
        let none = Bounds::default();
        assert_eq!(
            view_span(none, at(0., 100.), px(800.)),
            (px(-800.), px(1600.))
        );
        assert_eq!(
            view_span(at(0., 600.), none, px(800.)),
            (px(-800.), px(1600.))
        );
    }
}
