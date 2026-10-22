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

/// A page's measured area, so its board is sized for the space it has.
#[derive(Default)]
pub struct PlayLayout {
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl PlayLayout {
    /// The board's side (`--board-size`): the height left after the page's padding and the player
    /// lines, or the width left beside a 344px panel, between 260px and `--board-max`.
    pub fn board_size(&self, window: &Window) -> Pixels {
        let page = self.bounds.get().size;
        let width = if page.width > px(0.) {
            page.width
        } else {
            window.viewport_size().width
        };
        let height = if page.height > px(0.) {
            page.height
        } else {
            window.viewport_size().height - crate::shell::TOPBAR_HEIGHT
        };
        let chrome = PAGE_TOP + PAGE_BOTTOM + widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.;
        let by_height = height - chrome;
        let by_width = width - side_padding(window) * 2. - px(344.);
        by_height.min(by_width).max(px(260.)).min(px(1200.))
    }

    /// The panel's height: the board's column, player lines included.
    pub fn panel_height(&self, board: Pixels) -> Pixels {
        board + widgets::PLAYER_LINE * 2. + widgets::STACK_GAP * 2.
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
        let bounds = Rc::clone(&self.bounds);
        let entity = cx.entity().downgrade();
        div()
            .id(id)
            .size_full()
            .relative()
            .overflow_y_scroll()
            .child(
                canvas(
                    move |area, _, cx| {
                        if bounds.get() != area {
                            bounds.set(area);
                            // Size the board for the new area on the next frame.
                            let _ = entity.update(cx, |_, cx| cx.notify());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
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
