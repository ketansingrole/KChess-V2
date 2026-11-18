//! Calls that wait on something outside the main thread: the system, the core, the clipboard, the
//! disk. Each costs from a fraction of a millisecond to many, so none may run while a frame draws:
//! a view that made one on every frame took the analysis board's frames from 1.4 to 9 ms (#211,
//! the voice bar asking macOS about the microphone), and no test noticed for nine days.
//!
//! Such a call says so with `call` (`#[track_caller]`, as are the functions that make it, so the
//! report names the view that asked). The end-to-end tests draw every frame inside `drawing`,
//! which fails the test when a frame made one (with this crate's `test-support` feature, which the
//! app's tests turn on): every page, panel and dialog the tests open is
//! checked on every pull request, on each platform. Read the value once (when the page opens,
//! the window comes back or a setting changes) and keep it for drawing.

#[cfg(any(test, feature = "test-support"))]
use std::cell::{Cell, RefCell};
#[cfg(any(test, feature = "test-support"))]
use std::panic::Location;

/// A call that waits outside the main thread: `what` it asks, for the report.
#[track_caller]
pub fn call(what: &'static str) {
    #[cfg(any(test, feature = "test-support"))]
    if DRAWING.get() {
        let at = Location::caller();
        MADE.with_borrow_mut(|made| made.push((what, at)));
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = what;
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static DRAWING: Cell<bool> = const { Cell::new(false) };
    static MADE: RefCell<Vec<(&'static str, &'static Location<'static>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Draw a frame with `draw`, and fail if it made a slow call, naming each and where.
#[cfg(any(test, feature = "test-support"))]
pub fn drawing<R>(draw: impl FnOnce() -> R) -> R {
    let outer = DRAWING.replace(true);
    let result = draw();
    DRAWING.set(outer);
    let made = MADE.take();
    assert!(
        made.is_empty(),
        "a frame waited outside the main thread while drawing; read it before drawing and keep \
         it (slow_calls.rs):\n{}",
        made.iter()
            .map(|(what, at)| format!("  {what} at {at}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    result
}

#[cfg(test)]
mod tests {
    use super::{call, drawing};

    #[test]
    fn a_call_outside_a_frame_passes() {
        call("the system");
        assert_eq!(drawing(|| 7), 7);
    }

    /// The report names the call and the line that made it (on Windows with backslashes in the path).
    #[test]
    fn a_call_while_drawing_fails_naming_it() {
        let line = line!() + 1;
        let failed = std::panic::catch_unwind(|| drawing(|| call("the system")));
        let report = failed.expect_err("the frame fails");
        let report = report.downcast_ref::<String>().expect("a message");
        assert!(report.contains("  the system at "), "{report}");
        assert!(
            report.contains(&format!("slow_calls.rs:{line}:")),
            "{report}"
        );
    }

    #[test]
    fn a_failed_frame_leaves_the_next_one_clean() {
        let failed = std::panic::catch_unwind(|| drawing(|| call("the system")));
        assert!(failed.is_err());
        drawing(|| ());
        call("after the frame");
        drawing(|| ());
    }
}
