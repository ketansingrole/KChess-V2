//! A scroll area inside a scrolling page.

use gpui_kit::*;

/// `area` scrolls up and down by itself, tracked by `scroll`, inside a page that scrolls too.
/// GPUI hands a wheel event to every scroll area under the pointer, so the page would move with
/// it: here the wheel moves the area alone, and the page only once the area is at its end that
/// way, as a browser does.
pub fn nested_scroll(area: Stateful<Div>, scroll: &ScrollHandle) -> Stateful<Div> {
    let handle = scroll.clone();
    area.overflow_y_scroll().track_scroll(scroll).on_scroll(
        move |event: &ScrollEvent, window, cx| {
            let delta = event.delta.pixel_delta(window.line_height()).y;
            if keeps_wheel(handle.offset().y, handle.max_offset().y, delta) {
                cx.stop_propagation();
            }
        },
    )
}

/// Whether an area keeps a wheel's `delta` (up is positive): it has moved by it already to
/// `offset` (0 at the top, `-max` at the end), and it keeps it unless it was at its end that way.
fn keeps_wheel(offset: Pixels, max: Pixels, delta: Pixels) -> bool {
    let before = offset - delta;
    if delta < px(0.) {
        before + max > px(0.)
    } else if delta > px(0.) {
        before < px(0.)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::keeps_wheel;
    use gpui_kit::px;

    /// The area keeps the wheel while it can still move that way, and passes it on at its ends
    /// and when it has nothing to scroll.
    #[test]
    fn the_wheel_passes_on_only_at_an_end() {
        // (offset after the event, max, delta, kept)
        for (offset, max, delta, kept) in [
            (-40., 200., -40., true),   // down from the top
            (-240., 200., -40., false), // down past the end
            (-180., 200., -40., true),  // down to near the end
            (0., 200., 40., true),      // up to the top
            (40., 200., 40., false),    // up at the top
            (-40., 0., -40., false),    // nothing to scroll
            (-100., 200., 0., false),   // sideways
        ] {
            assert_eq!(
                keeps_wheel(px(offset), px(max), px(delta)),
                kept,
                "offset {offset}, max {max}, delta {delta}"
            );
        }
    }
}
