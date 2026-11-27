//! The playing surface every board page shares: the board as large
//! as the window's height and the page's width allow, and a side panel beside it, centred. A
//! narrow page (620px or less, or a portrait window up to 900px, which a large interface size
//! makes common) stacks the panel under the board instead.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::page::PageTab;
use crate::widgets;

/// The page's padding: 24px above, 40px below, and 2.6% of the width at the sides, from 14px
/// to 36px.
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

/// 2.6% of the window's width.
pub fn side_padding(window: &Window) -> Pixels {
    (window.viewport_size().width * 0.026)
        .max(px(14.))
        .min(px(36.))
}

/// `side`, down to a board whose squares are each a whole number of device pixels at `scale`.
/// Layout puts the board's edges on device pixels; a fractional square would put the squares
/// between them and resample every piece, which a 1x display shows as blur.
fn whole_squares(side: Pixels, scale: f32) -> Pixels {
    let square = (side.as_f32() * scale / 8.).floor().max(1.);
    px(square * 8. / scale)
}

thread_local! {
    /// Each window's page area at its last layout, as the shell measures it. The shell is drawn
    /// every frame; a page only while it shows, so its own measures keep the sizes they had when
    /// it was last shown. Opened again after the window or the sidebar changed, Over the board
    /// drew its board at 504px under the panel, then at 628px beside it, then the panel's actions
    /// on one row, before settling: three layouts in three frames.
    static PAGE_AREAS: RefCell<HashMap<WindowId, Size<Pixels>>> = RefCell::default();
}

/// Put in the shell's page area, which every page fills, so a page opens laid out for the area
/// it opens in.
pub fn page_area_probe() -> impl IntoElement {
    canvas(
        |area, window, _| {
            let id = window.window_handle().window_id();
            PAGE_AREAS.with_borrow_mut(|areas| areas.insert(id, area.size));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// The page area of `window` at its last layout, where a shell measures it.
pub fn page_area(window: &Window) -> Option<Size<Pixels>> {
    let id = window.window_handle().window_id();
    PAGE_AREAS.with_borrow(|areas| areas.get(&id).copied())
}

/// The width of a page's column, centred inside the page's side padding and at most `max` wide
/// (Home's cards, the openings browser), from the page area: a page opened again lays out for the
/// window it opens in. `measured` is the column as it last measured, for a window with no shell;
/// before either, the window less its padding.
pub fn column_width(window: &Window, max: Pixels, measured: Pixels) -> Pixels {
    let page = page_area(window).map(|area| area.width - side_padding(window) * 2.);
    let width = match page {
        Some(width) => width,
        None if measured > px(0.) => measured,
        None => window.viewport_size().width - side_padding(window) * 2.,
    };
    width.min(max)
}

/// How much of the page sits above an area: a page's tab bar (`tab_bar_height`), whose height
/// follows the page's width.
pub type Above = fn(&Window) -> Pixels;

/// An area measured as it is laid out, so the next frame can size what goes in it.
#[derive(Default)]
pub struct Measured {
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The page area when the area was measured, where a shell measures one.
    page: Rc<Cell<Option<Size<Pixels>>>>,
    /// What sits above the area on its page, and how tall it was when the area was measured.
    above: Option<Above>,
    above_then: Rc<Cell<Pixels>>,
}

impl Measured {
    /// An area under `above` on its page, which may change height with the page's width.
    pub fn under(above: Above) -> Measured {
        Measured {
            above: Some(above),
            ..Measured::default()
        }
    }

    /// How tall what sits above the area is now.
    fn above(&self, window: &Window) -> Pixels {
        self.above.map_or(px(0.), |above| above(window))
    }

    /// The area at the last layout; empty before the first.
    pub fn size(&self) -> Size<Pixels> {
        self.bounds.get().size
    }

    /// The area now: as it last measured, grown or shrunk by as much as the page area has since,
    /// less what the bar above it has grown by. A page is drawn only while it shows, so an area
    /// measured before it was hidden keeps its inset from the page (none for a whole page, a tab
    /// bar's height under one, which wraps to another row in a narrower page); empty before the
    /// first measure.
    pub fn size_in(&self, window: &Window) -> Size<Pixels> {
        let measured = self.size();
        match (self.page.get(), page_area(window)) {
            (Some(then), Some(now)) if measured.width > px(0.) => {
                let grown = self.above(window) - self.above_then.get();
                size(
                    (measured.width + now.width - then.width).max(px(0.)),
                    (measured.height + now.height - then.height - grown).max(px(0.)),
                )
            }
            _ => measured,
        }
    }

    /// Where the area was at the last layout, in the window; empty before the first.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    /// Put this, absolutely positioned to fill it, in the element to measure; a change of size
    /// redraws the view.
    pub fn probe<V: 'static>(&self, cx: &mut Context<V>) -> impl IntoElement + use<V> {
        let bounds = Rc::clone(&self.bounds);
        let page = Rc::clone(&self.page);
        let above = self.above;
        let above_then = Rc::clone(&self.above_then);
        let entity = cx.entity().downgrade();
        canvas(
            move |area, window, _| {
                // The shell's page area is laid out before the page in it, and the bar above
                // this area for that page area's width.
                page.set(page_area(window));
                above_then.set(above.map_or(px(0.), |above| above(window)));
                if bounds.get() != area {
                    bounds.set(area);
                    // A notify while drawing schedules no frame: one change of size (a zoom
                    // step, unlike a resize) would otherwise leave the old layout up.
                    let entity = entity.clone();
                    window.on_next_frame(move |_, cx| {
                        entity.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

/// The rows of the tabs of a page drawn above its body (Practice, Puzzles), in the page's width
/// inside its side padding.
fn tab_rows<T: PageTab>(window: &Window) -> widgets::PillRows {
    let page = page_area(window).map_or(window.viewport_size().width, |area| area.width);
    widgets::PillRows::new(&T::pills(), page - side_padding(window) * 2., window)
}

/// How much of its page a row of tabs above the body takes: the page's top padding and the
/// tabs' rows. Known as the page draws, so the board under it is sized for the room it has in
/// its first frame: sized from the whole page, or from the tabs' rows at another width, a board
/// as tall as the room under them moved once they had been laid out.
pub fn tab_bar_height<T: PageTab>(window: &Window) -> Pixels {
    PAGE_TOP + tab_rows::<T>(window).height()
}

/// A page's tabs above its body, `tab_bar_height` tall: choosing one keeps it and redraws the
/// page.
pub fn tab_bar<T: PageTab>(
    id: &'static str,
    window: &Window,
    cx: &App,
) -> impl IntoElement + use<T> {
    let palette = cx.global::<kchess_ui::theme::Palette>();
    div()
        .flex_none()
        .px(side_padding(window))
        .pt(PAGE_TOP)
        .child(widgets::pill_bar(
            id,
            T::pills(),
            &tab_rows::<T>(window),
            T::stored(cx).index(),
            |index, window, cx| {
                T::ALL[index].store(cx);
                window.refresh();
            },
            palette,
        ))
}

/// Below this page width the panel goes under the board (`@container page (max-width: 620px)`).
const STACK_WIDTH: f32 = 620.;
/// A portrait window stacks up to this width (`@media (max-width: 900px) and (orientation:
/// portrait)`).
const PORTRAIT_STACK_WIDTH: f32 = 900.;
/// What a stacked board leaves of the window's height for the rest.
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
    /// A board under `above` on its page (a page's tabs, `tab_bar_height`): sized from the area
    /// the page leaves under it, before it has measured its own.
    pub fn under(above: Above) -> PlayLayout {
        PlayLayout {
            area: Measured::under(above),
            stacked: Cell::default(),
        }
    }

    /// The board's side: the height left after the page's padding and the player
    /// lines, or the width left beside a 344px panel, between 260px and the largest board
    /// size.
    pub fn board_size(&self, window: &Window) -> Pixels {
        self.board_size_beside(px(0.), window)
    }

    /// Put in a page laid out by hand, so the board is sized for the area it has.
    pub fn probe<V: 'static>(&self, cx: &mut Context<V>) -> impl IntoElement + use<V> {
        self.area.probe(cx)
    }

    /// The page's area now, though it was hidden when the window last changed; before its first
    /// layout, the shell's page area it opens in, less what sits above it there; empty before
    /// either.
    fn area(&self, window: &Window) -> Size<Pixels> {
        let area = self.area.size_in(window);
        if area.width > px(0.) {
            return area;
        }
        match page_area(window) {
            Some(page) => size(
                page.width,
                (page.height - self.area.above(window)).max(px(0.)),
            ),
            None => area,
        }
    }

    /// The page's width at the last layout, or the window's before it.
    pub fn width(&self, window: &Window) -> Pixels {
        let page = self.area(window);
        if page.width > px(0.) {
            page.width
        } else {
            window.viewport_size().width
        }
    }

    /// The panel's width beside a board column `column` wide, or under it when stacked, at most
    /// `max`, as the row lays it out: a page decides what fits in its panel in the frame it
    /// draws, where the panel's own measure is a frame late (and a page late when it was hidden).
    /// Call after sizing the board, which decides whether the panel is stacked.
    pub fn panel_width(&self, column: Pixels, max: Pixels, window: &Window) -> Pixels {
        let inside = self.width(window) - side_padding(window) * 2.;
        let room = match self.stacked.get() {
            Some(stacked) => stacked.min(inside),
            None => inside - column - LAYOUT_GAP,
        };
        room.min(max).max(PANEL_MIN)
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

    /// The board's side when its column holds `around` besides the board: nothing for a bare
    /// board, a material row and the move box for a puzzle.
    pub fn board_size_around(
        &self,
        extra_width: Pixels,
        extra_height: Pixels,
        around: Pixels,
        window: &Window,
    ) -> Pixels {
        let page = self.area(window);
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
            let size = whole_squares(
                (width - side_padding(window) * 2.)
                    .min((viewport.height - px(STACKED_CHROME)).max(floor))
                    .min(px(STACKED_BOARD_MAX))
                    .max(px(160.)),
                window.scale_factor(),
            );
            self.stacked.set(Some(size + extra_width));
            return size;
        }
        self.stacked.set(None);
        let chrome = PAGE_TOP + PAGE_BOTTOM + around;
        let by_height = height - chrome;
        let by_width = width - side_padding(window) * 2. - px(344.);
        whole_squares(
            by_height.min(by_width).max(px(260.)).min(px(1200.)),
            window.scale_factor(),
        )
    }

    /// The panel goes under the board on this page now.
    pub fn is_stacked(&self) -> bool {
        self.stacked.get().is_some()
    }

    /// The panel's height beside a board between two player lines (a game: `board_size`), so it
    /// ends with the lower line; its own when stacked. Only for those pages: it adds both lines
    /// whatever the column holds, so a board with other rows around it, or none, sizes with
    /// `board_and_panel` instead.
    pub fn panel_height(&self, board: Pixels) -> Length {
        self.fixed(self.panel_height_px(board))
    }

    /// A board whose column holds exactly `around` besides it, and no player lines: its side, and
    /// the height of the panel beside it, which ends level with the column (its own when
    /// stacked). `around` is what the column really holds: a puzzle board's rows
    /// (`PuzzleBoard::around`), the move box's row, or nothing for a bare board. Every page
    /// without player lines sizes here: `panel_height`'s two lines made such a panel run past
    /// the column, and a panel as tall as the board alone stopped short of the move box.
    pub fn board_and_panel(&self, around: Pixels, window: &Window) -> (Pixels, Length) {
        self.board_and_panel_beside(px(0.), around, window)
    }

    /// `board_and_panel` with `extra_width` beside the board's column as well (a lesson's
    /// chapters).
    pub fn board_and_panel_beside(
        &self,
        extra_width: Pixels,
        around: Pixels,
        window: &Window,
    ) -> (Pixels, Length) {
        let side = self.board_size_around(extra_width, px(0.), around, window);
        (side, self.fixed(side + around))
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
