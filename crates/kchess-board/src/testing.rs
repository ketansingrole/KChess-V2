//! What the board's tests and the pages' share: the app as a board page needs it.

use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, Window};
use kchess_app::tests::TestCore;

/// The app as a board page needs it: a core of the test's own, Settings, the accounts, the live
/// Lichess game, the theme and voice, and a window to put the page's views in.
pub struct Started {
    cx: gpui_kit::TestAppContext,
    window: AnyWindowHandle,
    _core: TestCore,
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div()
    }
}

impl Started {
    pub fn new(cx: &mut gpui_kit::TestAppContext) -> Started {
        Started::with_lichess(cx, None)
    }

    /// `new`, with the core's Lichess at `origin` (a local stand-in, `kchess_testkit::Server`).
    pub fn with_lichess(cx: &mut gpui_kit::TestAppContext, origin: Option<String>) -> Started {
        // The core answers on its Tokio threads; the test waits for them in real time.
        cx.executor().allow_parking();
        let core = cx.update(|cx| {
            gpui_kit::init(cx);
            let core = kchess_app::tests::start_core_with_lichess(cx, origin);
            kchess_app::settings::init_for_tests(cx);
            kchess_app::accounts::init(cx);
            crate::online::init(cx);
            let colors = kchess_ui::theme::colors("kchess", &[], false);
            cx.set_global(kchess_ui::theme::Palette::of(&colors, false));
            crate::voice::init(cx);
            core
        });
        let window = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Blank))
                .expect("a window")
                .0
        });
        Started {
            cx: cx.clone(),
            window,
            _core: core,
        }
    }

    /// Work in the window, as a page's views are made and changed.
    pub fn with_window<R>(&mut self, work: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| work(window, cx))
            .expect("the window")
    }

    /// Wait, in real time, until `ready` holds; `what` names it on failure. The core answers on
    /// its own threads, so the test looks again every 5 ms for up to 30 s, as the app's
    /// end-to-end harness does.
    pub fn wait_for(&mut self, what: &str, ready: impl FnMut(&mut Window, &mut App) -> bool) {
        self.wait_within(Duration::from_secs(30), what, ready);
    }

    /// `wait_for`, failing after `limit`: for an answer the core gives at once, so that a test
    /// whose answer never comes fails promptly instead of running into the runner's timeout
    /// (which reads as a hang, not a failure, to `cargo mutants`).
    pub fn wait_within(
        &mut self,
        limit: Duration,
        what: &str,
        mut ready: impl FnMut(&mut Window, &mut App) -> bool,
    ) {
        const POLL: Duration = Duration::from_millis(5);
        let started = Instant::now();
        while !self.with_window(&mut ready) {
            assert!(
                started.elapsed() < limit,
                "timed out after {limit:?} waiting for {what}"
            );
            std::thread::sleep(POLL);
            self.cx.executor().advance_clock(POLL);
            self.cx.run_until_parked();
        }
    }

    /// The test's app, to run its tasks or read its state.
    pub fn cx(&mut self) -> &mut gpui_kit::TestAppContext {
        &mut self.cx
    }
}

#[cfg(test)]
mod tests {
    use super::Started;

    /// A wait looks until what it waits for holds, and no longer.
    #[gpui_kit::test]
    fn a_wait_lasts_until_it_holds(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Started::new(cx);
        let mut looks = 0;
        app.wait_for("the third look", |_, _| {
            looks += 1;
            looks == 3
        });
        assert_eq!(looks, 3);
    }
}
