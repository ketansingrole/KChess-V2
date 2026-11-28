//! A scroll area inside a scrolling page.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// Longer than this between two scroll events, and the second starts another stream: a
/// trackpad's momentum follows its lifted finger within a frame or two.
const STREAM_GAP: Duration = Duration::from_millis(150);

/// What a scroll area inside a page keeps between frames: its scroll, and who has the swipe
/// going on (`nested_scroll`).
#[derive(Clone, Default)]
pub struct NestedScroll {
    handle: ScrollHandle,
    swipe: Rc<Cell<Swipe>>,
}

impl NestedScroll {
    pub fn new() -> NestedScroll {
        NestedScroll::default()
    }
}

/// The swipe or wheel the area last saw, and when.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Swipe {
    owner: Owner,
    last: Option<Instant>,
}

/// Who a swipe belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Owner {
    /// Nothing seen yet.
    #[default]
    None,
    /// A swipe began over the area; its first move decides whose it is.
    Undecided,
    /// The area scrolls for the whole swipe, its momentum included, even past its end.
    Area,
    /// The page scrolls for the whole swipe: it began elsewhere, or with the area at its end
    /// that way.
    Page,
    /// A mouse wheel, which has no swipe: each notch goes to the area until it is at its end.
    Wheel,
}

/// What one scroll event over the area does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// The area keeps it: the page under it does not move.
    Keep,
    /// The page has it: the area's own move by it is taken back.
    GiveBack,
    /// The page has it and the area did not move.
    PassOn,
}

/// One event's facts, as the area sees it after GPUI has moved it by `delta`.
#[derive(Clone, Copy, Debug)]
struct Event {
    phase: TouchPhase,
    /// A trackpad's, which comes in swipes, not a wheel's notches.
    precise: bool,
    /// Within `STREAM_GAP` of the last event the area saw.
    continuing: bool,
    offset: Pixels,
    max: Pixels,
    delta: Pixels,
}

/// `area` scrolls up and down by itself, inside a page that scrolls too. GPUI hands a scroll to
/// every scroll area under the pointer, so the page would move with it, and it decides event by
/// event. Here, as on macOS, a swipe is decided once, as it begins: one that starts over the area
/// scrolls the area to its end and stops there, its momentum included; one that starts with the
/// area at its end that way, or elsewhere on the page, scrolls the page, the area left alone as
/// it passes under the pointer. A mouse wheel's notches go to the area until it is at its end.
pub fn nested_scroll(area: Stateful<Div>, scroll: &NestedScroll) -> Stateful<Div> {
    let NestedScroll { handle, swipe } = scroll.clone();
    area.overflow_y_scroll()
        .track_scroll(&scroll.handle)
        .on_scroll(move |event: &ScrollEvent, window, cx| {
            let now = cx.background_executor().now();
            let seen = swipe.get();
            let offset = handle.offset();
            let delta = event.delta.pixel_delta(window.line_height()).y;
            let (owner, action) = decide(
                seen.owner,
                Event {
                    phase: event.touch_phase,
                    precise: event.delta.precise(),
                    continuing: seen.last.is_some_and(|last| now - last < STREAM_GAP),
                    offset: offset.y,
                    max: handle.max_offset().y,
                    delta,
                },
            );
            swipe.set(Swipe {
                owner,
                last: Some(now),
            });
            match action {
                Action::Keep => cx.stop_propagation(),
                Action::GiveBack => handle.set_offset(point(offset.x, offset.y - delta)),
                Action::PassOn => {}
            }
        })
}

/// Whose the swipe is after `event`, and what the event does.
fn decide(owner: Owner, event: Event) -> (Owner, Action) {
    let owner = if event.phase == TouchPhase::Started {
        Owner::Undecided
    } else if !event.continuing {
        // A stream the area did not see begin: a trackpad's began elsewhere on the page.
        if event.precise {
            Owner::Page
        } else {
            Owner::Wheel
        }
    } else if !event.precise && owner != Owner::Wheel {
        Owner::Wheel
    } else {
        owner
    };
    let can_move = keeps_wheel(event.offset, event.max, event.delta);
    let owner = match owner {
        Owner::Undecided if event.delta != px(0.) => {
            if can_move {
                Owner::Area
            } else {
                Owner::Page
            }
        }
        owner => owner,
    };
    let action = match owner {
        Owner::Area => Action::Keep,
        Owner::Page if event.delta != px(0.) => Action::GiveBack,
        Owner::Wheel if can_move => Action::Keep,
        _ => Action::PassOn,
    };
    (owner, action)
}

/// Whether an area can take a scroll's `delta` (up is positive): it has moved by it already to
/// `offset` (0 at the top, `-max` at the end), and it could unless it was at its end that way.
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
    use super::{Action, Event, Owner, decide, keeps_wheel};
    use gpui_kit::{TouchPhase, px};

    /// The area can take a scroll while it can still move that way, and not at its ends or
    /// with nothing to scroll.
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

    /// Who has a swipe, decided as it begins and kept to its end, momentum included; a wheel
    /// decided notch by notch. Each row: the owner before, the event (phase, a trackpad's, within
    /// the stream, offset after it, max, delta), then the owner after and the action.
    #[test]
    fn a_swipe_is_decided_once_as_it_begins() {
        use Action::*;
        use Owner::*;
        use TouchPhase::*;
        let rows = [
            // A swipe begins over the area: nothing yet, then its first move takes the area.
            (None, Started, true, false, 0., 200., 0., Undecided, PassOn),
            (Undecided, Moved, true, true, -40., 200., -40., Area, Keep),
            // The area at its end mid-swipe: the swipe stays the area's, the page held still.
            (Area, Moved, true, true, -240., 200., -40., Area, Keep),
            // The finger lifts and momentum follows: still the area's.
            (Area, Ended, true, true, -200., 200., 0., Area, Keep),
            (Area, Moved, true, true, -230., 200., -30., Area, Keep),
            // A swipe that begins with the area at its end that way is the page's.
            (
                Area, Started, true, true, -200., 200., 0., Undecided, PassOn,
            ),
            (
                Undecided, Moved, true, true, -240., 200., -40., Page, GiveBack,
            ),
            (Page, Moved, true, true, -240., 200., -40., Page, GiveBack),
            // Up from that end, a new swipe is the area's again.
            (
                Page, Started, true, false, -200., 200., 0., Undecided, PassOn,
            ),
            (Undecided, Moved, true, true, -160., 200., 40., Area, Keep),
            // A trackpad's stream the area did not see begin began on the page: the area,
            // passing under the pointer, gives its move back.
            (None, Moved, true, false, -40., 200., -40., Page, GiveBack),
            (Area, Moved, true, false, -40., 200., -40., Page, GiveBack),
            // A wheel: the area's until it is at its end, then the page's.
            (None, Moved, false, false, -40., 200., -40., Wheel, Keep),
            (Wheel, Moved, false, true, -240., 200., -40., Wheel, PassOn),
            (Page, Moved, false, true, -40., 200., -40., Wheel, Keep),
        ];
        for (before, phase, precise, continuing, offset, max, delta, owner, action) in rows {
            let event = Event {
                phase,
                precise,
                continuing,
                offset: px(offset),
                max: px(max),
                delta: px(delta),
            };
            assert_eq!(
                decide(before, event),
                (owner, action),
                "{before:?} then {event:?}"
            );
        }
    }
}
