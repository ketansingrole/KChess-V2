//! The playing surface every board page shares (`.play-layout` in `main.css`): the board as large
//! as the window's height and the page's width allow, and a side panel beside it, centred.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::widgets;

/// The page's padding: 24px above, 40px below, and `clamp(14px, 2.6vw, 36px)` at the sides.
pub const PAGE_TOP: Pixels = px(24.);
pub const PAGE_BOTTOM: Pixels = px(40.);
pub const PANEL_MIN: Pixels = px(300.);
pub const PANEL_MAX: Pixels = px(560.);
/// Zen mode keeps the panel compact beside the board.
pub const PANEL_ZEN: Pixels = px(340.);
pub const LAYOUT_GAP: Pixels = px(24.);

/// `clamp(14px, 2.6vw, 36px)`: 2.6% of the window's width.
pub fn side_padding(window: &Window) -> Pixels {
    (window.viewport_size().width * 0.026)
        .max(px(14.))
        .min(px(36.))
}

/// An area measured as it is laid out, so the next frame can size what goes in it.
#[derive(Default)]
pub struct Measured {
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Measured {
    /// The area at the last layout; empty before the first.
    pub fn size(&self) -> Size<Pixels> {
        self.bounds.get().size
    }

    /// Put this, absolutely positioned to fill it, in the element to measure; a change of size
    /// redraws the view.
    pub fn probe<V: 'static>(&self, cx: &mut Context<V>) -> impl IntoElement + use<V> {
        let bounds = Rc::clone(&self.bounds);
        let entity = cx.entity().downgrade();
        canvas(
            move |area, _, cx| {
                if bounds.get() != area {
                    bounds.set(area);
                    let _ = entity.update(cx, |_, cx| cx.notify());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

/// A page's measured area, so its board is sized for the space it has.
#[derive(Default)]
pub struct PlayLayout {
    area: Measured,
}

impl PlayLayout {
    /// The board's side (`--board-size`): the height left after the page's padding and the player
    /// lines, or the width left beside a 344px panel, between 260px and `--board-max`.
    pub fn board_size(&self, window: &Window) -> Pixels {
        self.board_size_beside(px(0.), window)
    }

    /// Put in a page laid out by hand, so the board is sized for the area it has.
    pub fn probe<V: 'static>(&self, cx: &mut Context<V>) -> impl IntoElement + use<V> {
        self.area.probe(cx)
    }

    /// The page's width at the last layout, or the window's before it.
    pub fn width(&self, window: &Window) -> Pixels {
        let page = self.area.size();
        if page.width > px(0.) {
            page.width
        } else {
            window.viewport_size().width
        }
    }

    /// The board's side with another column of `extra` beside it (a lesson's chapters).
    pub fn board_size_beside(&self, extra: Pixels, window: &Window) -> Pixels {
        self.board_size_in(extra, px(0.), window)
    }

    /// The board's side with `extra_width` beside it and `extra_height` above (the study bar).
    pub fn board_size_in(
        &self,
        extra_width: Pixels,
        extra_height: Pixels,
        window: &Window,
    ) -> Pixels {
        let page = self.area.size();
        let width = self.width(window) - extra_width;
        let height = if page.height > px(0.) {
            page.height
        } else {
            window.viewport_size().height - crate::shell::TOPBAR_HEIGHT
        } - extra_height;
        let chrome = PAGE_TOP + PAGE_BOTTOM + widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.;
        let by_height = height - chrome;
        let by_width = width - side_padding(window) * 2. - px(344.);
        by_height.min(by_width).max(px(260.)).min(px(1200.))
    }

    /// The panel's height: the board's column, player lines included.
    pub fn panel_height(&self, board: Pixels) -> Pixels {
        board + widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.
    }

    /// The page with a row above the board and panel (the analysis board's study bar).
    pub fn render_under<V: 'static>(
        &self,
        id: &'static str,
        header: impl IntoElement,
        board_column: impl IntoElement,
        panel: impl IntoElement,
        window: &Window,
        cx: &mut Context<V>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .size_full()
            .relative()
            .overflow_y_scroll()
            .child(self.area.probe(cx))
            .child(div().pt(PAGE_TOP).px(side_padding(window)).child(header))
            .child(
                h_flex()
                    .items_start()
                    .justify_center()
                    .gap(LAYOUT_GAP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .child(board_column)
                    .child(panel),
            )
    }

    /// The page: the board's column and the panel, measured so the next frame fits them.
    pub fn render<V: 'static>(
        &self,
        id: &'static str,
        board_column: impl IntoElement,
        panel: impl IntoElement,
        window: &Window,
        cx: &mut Context<V>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .size_full()
            .relative()
            .overflow_y_scroll()
            // Size the board for the new area on the next frame.
            .child(self.area.probe(cx))
            .child(
                h_flex()
                    .items_start()
                    .justify_center()
                    .gap(LAYOUT_GAP)
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .child(board_column)
                    .child(panel),
            )
    }
}
