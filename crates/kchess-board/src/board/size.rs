//! The board's size the player chose: dragged from the handle in the board's bottom-right corner
//! on the board pages, kept as a view preference (`kchess:board-size`) so every board page and
//! every KChess app on the profile draws the board at that size. Nothing chosen, or the largest
//! that fits, is the board as large as the page allows; a smaller window still makes it smaller.
//!
//! The layout sizes boards from the window alone (`kchess_nav::play_layout`), so the choice is
//! kept here for it to read: the preference as the shell last read it, and the size being
//! dragged until the drag ends.

use std::cell::Cell;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use super::ChessBoard;

/// The view preference: the board's side in pixels, or null for as large as fits.
pub const KEY: &str = "kchess:board-size";
/// The smallest board the handle makes.
pub const MIN: Pixels = px(200.);

thread_local! {
    /// The preference as the shell last read it.
    static CHOSEN: Cell<Option<Pixels>> = const { Cell::new(None) };
    /// The size being dragged, until the drag ends.
    static DRAGGED: Cell<Option<Pixels>> = const { Cell::new(None) };
    /// Where the drag started and the board's side then.
    static FROM: Cell<Option<(Point<Pixels>, Pixels)>> = const { Cell::new(None) };
    /// The largest board the page had room for at its last layout.
    static FIT: Cell<Pixels> = const { Cell::new(px(0.)) };
}

/// The side the player chose, being dragged or kept; `None` for as large as fits.
pub fn chosen() -> Option<Pixels> {
    DRAGGED.get().or(CHOSEN.get())
}

/// Read the preference again, as the shell does each frame: another app on the profile may have
/// changed it.
pub fn read(cx: &App) {
    let side: Option<f32> = kchess_app::prefs::get(cx, KEY, None);
    CHOSEN.set(side.map(px));
}

/// The board on a page whose layout had room for a board `fit` wide: the chosen side, at most
/// `fit`, or `fit` when nothing is chosen.
pub fn fitted(fit: Pixels) -> Pixels {
    FIT.set(fit);
    chosen().map_or(fit, |side| side.max(MIN).min(fit))
}

/// The handle's drag.
struct Resize;

/// What follows the pointer while the handle is dragged: nothing, as the board itself resizes.
struct Nothing;

impl Render for Nothing {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// The handle's side.
const HANDLE: Pixels = px(18.);

/// End a drag: keep the size, or nothing when it is as large as fits.
fn finish(cx: &mut App) {
    let Some(side) = DRAGGED.take() else {
        return;
    };
    FROM.set(None);
    let kept = (side < FIT.get() - px(1.)).then_some(side);
    CHOSEN.set(kept);
    kchess_app::prefs::set(cx, KEY, kept.map(f32::from));
}

/// The board on a board page: `cached`, with the handle in its bottom-right corner that sizes it.
/// Dragging it sizes the board as it goes and keeps the size when it is let go; a double click
/// makes the board as large as fits again.
pub fn resizable(board: &Entity<ChessBoard>) -> AnyElement {
    let measured = board.downgrade();
    // A corner bracket, as a window's size grip reads.
    let grip = div()
        .absolute()
        .right(px(4.))
        .bottom(px(4.))
        .size(px(8.))
        .border_r_2()
        .border_b_2()
        .rounded_br(crate::theme::radii::SMALL)
        .border_color(hsla(0., 0., 0., 0.45));
    let handle = div()
        .id("board-resize")
        .test_support()
        .aria_label("Board size")
        .absolute()
        .right_0()
        .bottom_0()
        .size(HANDLE)
        .cursor(CursorStyle::ResizeUpLeftDownRight)
        .opacity(0.55)
        .hover(|handle| handle.opacity(1.))
        .tooltip(|window, cx| {
            Tooltip::new("Drag to resize the board · double-click to fit").build(window, cx)
        })
        .child(grip)
        // From where the button went down: a drag starts only once the pointer has moved a
        // little, and measured from there the board lagged the pointer by that much.
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            let side = measured
                .upgrade()
                .map_or(px(0.), |board| board.read(cx).bounds.get().size.width);
            FROM.set(Some((event.position, side)));
        })
        .on_drag(Resize, |_, _, _, cx| cx.new(|_| Nothing))
        .on_drag_move::<Resize>(|event, window, _| {
            let Some((start, side)) = FROM.get() else {
                return;
            };
            let moved = event.event.position - start;
            // Along the diagonal, as the corner moves.
            let wanted = side + (moved.x + moved.y) / 2.;
            DRAGGED.set(Some(wanted.max(MIN).min(FIT.get().max(MIN))));
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| finish(cx))
        .on_mouse_up_out(MouseButton::Left, |_, _, cx| finish(cx))
        .on_click(|event, _, cx| {
            if event.click_count() == 2 {
                DRAGGED.set(None);
                CHOSEN.set(None);
                kchess_app::prefs::set(cx, KEY, None::<f32>);
            }
        });
    div()
        .relative()
        .w_full()
        .child(super::cached(board))
        .child(handle)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::{CHOSEN, DRAGGED, MIN, fitted};

    /// The chosen size bounds the board and never grows it past what fits; nothing chosen is as
    /// large as fits; a size being dragged counts over the one kept; and a handle dragged below
    /// the smallest board stops there.
    #[test]
    fn the_board_is_the_chosen_size_within_what_fits() {
        CHOSEN.set(None);
        DRAGGED.set(None);
        assert_eq!(fitted(px(640.)), px(640.), "nothing chosen");
        CHOSEN.set(Some(px(480.)));
        assert_eq!(fitted(px(640.)), px(480.), "smaller than fits");
        assert_eq!(
            fitted(px(400.)),
            px(400.),
            "a smaller window still shrinks it"
        );
        DRAGGED.set(Some(px(520.)));
        assert_eq!(fitted(px(640.)), px(520.), "the drag counts");
        DRAGGED.set(Some(px(40.)));
        assert_eq!(fitted(px(640.)), MIN, "no smaller than the smallest");
        DRAGGED.set(None);
        CHOSEN.set(None);
    }
}
