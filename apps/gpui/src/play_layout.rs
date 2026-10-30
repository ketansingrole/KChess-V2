//! The playing surface every board page shares (`.play-layout` in `main.css`): the board as large
//! as the window's height and the page's width allow, and a side panel beside it, centred. A
//! narrow page (620px or less, or a portrait window up to 900px, which a large interface size
//! makes common) stacks the panel under the board instead.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
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

/// The panel beside the board drawn as a cached view (`crate::part::Part`): sized in the row as
/// `widgets::side_panel` sizes itself there, `height` high and at most `max_width` wide.
pub fn panel_style(height: Pixels, max_width: Pixels) -> StyleRefinement {
    let mut style = StyleRefinement {
        flex_grow: Some(1.),
        flex_shrink: Some(1.),
        flex_basis: Some(relative(0.).into()),
        ..Default::default()
    };
    style.min_size.width = Some(PANEL_MIN.into());
    style.max_size.width = Some(max_width.into());
    style.size.height = Some(height.into());
    style
}

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
            move |area, window, _| {
                if bounds.get() != area {
                    bounds.set(area);
                    // A notify while drawing schedules no frame: one change of size (a zoom
                    // step, unlike a resize) would otherwise leave the old layout up.
                    let entity = entity.clone();
                    window.on_next_frame(move |_, cx| {
                        let _ = entity.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

/// Below this page width the panel goes under the board (`@container page (max-width: 620px)`).
const STACK_WIDTH: f32 = 620.;
/// A portrait window stacks up to this width (`@media (max-width: 900px) and (orientation:
/// portrait)`).
const PORTRAIT_STACK_WIDTH: f32 = 900.;
/// What a stacked board leaves of the window's height for the rest (`100dvh - 260px`).
const STACKED_CHROME: f32 = 260.;
const STACKED_BOARD_MAX: f32 = 760.;

/// A page's measured area, so its board is sized for the space it has.
#[derive(Default)]
pub struct PlayLayout {
    area: Measured,
    /// The board column's width while the panel is stacked under it, from the last sizing.
    stacked: Cell<Option<Pixels>>,
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
        self.board_size_around(
            extra_width,
            extra_height,
            widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.,
            window,
        )
    }

    /// The board's side when its column holds `around` besides the board (`v-board-fit`'s
    /// measured chrome): nothing for a bare board, a material row and the move box for a puzzle.
    pub fn board_size_around(
        &self,
        extra_width: Pixels,
        extra_height: Pixels,
        around: Pixels,
        window: &Window,
    ) -> Pixels {
        let page = self.area.size();
        let page_width = self.width(window);
        let width = page_width - extra_width;
        let height = if page.height > px(0.) {
            page.height
        } else {
            window.viewport_size().height - crate::shell::TOPBAR_HEIGHT
        } - extra_height;
        let viewport = window.viewport_size();
        let portrait = viewport.height > viewport.width;
        if page_width <= px(STACK_WIDTH) || (portrait && page_width <= px(PORTRAIT_STACK_WIDTH)) {
            let floor = if portrait { px(260.) } else { px(0.) };
            let size = (width - side_padding(window) * 2.)
                .min((viewport.height - px(STACKED_CHROME)).max(floor))
                .min(px(STACKED_BOARD_MAX))
                .max(px(160.));
            self.stacked.set(Some(size + extra_width));
            return size;
        }
        self.stacked.set(None);
        let chrome = PAGE_TOP + PAGE_BOTTOM + around;
        let by_height = height - chrome;
        let by_width = width - side_padding(window) * 2. - px(344.);
        by_height.min(by_width).max(px(260.)).min(px(1200.))
    }

    /// The panel goes under the board on this page now.
    pub fn is_stacked(&self) -> bool {
        self.stacked.get().is_some()
    }

    /// The panel's height: the board's column, player lines included; its own when stacked.
    pub fn panel_height(&self, board: Pixels) -> Length {
        self.fixed(self.panel_height_px(board))
    }

    /// `height` beside the board, or the panel's own height under it.
    pub fn fixed(&self, height: Pixels) -> Length {
        if self.is_stacked() {
            Length::Auto
        } else {
            height.into()
        }
    }

    /// The board's column height, for a page that lays out its own row (Watch).
    pub fn panel_height_px(&self, board: Pixels) -> Pixels {
        board + widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.
    }

    /// The board's column and the panel, side by side or (stacked) the panel under the board at
    /// the column's width.
    fn row(&self, board_column: impl IntoElement, panel: impl IntoElement, window: &Window) -> Div {
        match self.stacked.get() {
            Some(column) => v_flex()
                .items_center()
                .gap(LAYOUT_GAP)
                .px(side_padding(window))
                .child(board_column)
                .child(div().w(column).max_w_full().child(panel)),
            None => h_flex()
                .items_start()
                .justify_center()
                .gap(LAYOUT_GAP)
                .px(side_padding(window))
                .child(board_column)
                .child(panel),
        }
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
            .child(self.row(board_column, panel, window).pb(PAGE_BOTTOM))
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
                self.row(board_column, panel, window)
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM),
            )
    }
}
