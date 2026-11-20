//! What the board's tests and the pages' share: the app as a board page needs it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, Render, Subscription, Task, Window,
};
use kchess_app::core::{AppCore, ClientEvent, CoreEvent};
use kchess_app::tests::TestCore;
use kchess_contracts::calls;
use kchess_contracts::types::Assistance;

/// The app as a board page needs it: a core of the test's own, Settings, the accounts, what the
/// pages know of the live Lichess game (none is played: engine help is as the core lends it), the
/// theme and voice, and a window to put the page's views in.
pub struct Started {
    cx: gpui_kit::TestAppContext,
    window: AnyWindowHandle,
    _core: TestCore,
    /// Standing in for the app's live game, which none is played here (`follow_assistance`).
    _assistance: Option<(Subscription, Task<()>)>,
}

/// Reports of the core's engine help seen, so that a status answer asked before one is known for
/// out of date: the report is newer.
#[derive(Clone, Default)]
struct Reports(Rc<Cell<u64>>);

impl Reports {
    fn seen(&self) -> u64 {
        self.0.get()
    }

    fn report(&self) {
        self.0.set(self.0.get() + 1);
    }

    /// No report came since `asked` were seen.
    fn none_since(&self, asked: u64) -> bool {
        self.0.get() == asked
    }
}

/// What the app's live game does as it starts, with no game to play: asks the core for a game
/// left running, and lends engine help as the core says (`online:assistance`, whose lock is the
/// one that counts). A status answer older than a report is dropped, as the game drops it.
fn follow_assistance(cx: &mut App) -> (Subscription, Task<()>) {
    let core = cx.global::<AppCore>().clone();
    let reports = Reports::default();
    let tell = |assistance: Assistance, cx: &mut App| {
        crate::live_game::handle(cx).update(cx, |live, cx| {
            live.set(assistance == Assistance::Allowed, false, cx)
        });
    };
    let resume = core.request(calls::ResumeOnline {});
    let status = core.request(calls::AssistanceStatus {});
    let seen = reports.clone();
    let asked = reports.seen();
    let answered = cx.spawn(async move |cx| {
        resume.await.expect("the check for a game in progress");
        let assistance = status.await.expect("the core's engine help");
        cx.update(|cx| {
            if seen.none_since(asked) {
                tell(assistance, cx);
            }
        });
    });
    let reported = cx.subscribe(&core.events, move |_, event: &ClientEvent, cx| {
        if let ClientEvent::Event(CoreEvent::Assistance(assistance)) = event {
            reports.report();
            tell(*assistance, cx);
        }
    });
    (reported, answered)
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
        let mut started = Started::bare(cx, origin);
        started._assistance = Some(cx.update(follow_assistance));
        started
    }

    /// `with_lichess` for the app's own live game, which tells the pages what they know of it
    /// (`live_game`) itself: nothing stands in for it.
    pub fn bare(cx: &mut gpui_kit::TestAppContext, origin: Option<String>) -> Started {
        // The core answers on its Tokio threads; the test waits for them in real time.
        cx.executor().allow_parking();
        let core = cx.update(|cx| {
            gpui_kit::init(cx);
            let core = kchess_app::tests::start_core_delivering(cx, origin);
            kchess_app::settings::init_for_tests(cx);
            kchess_app::accounts::init(cx);
            crate::live_game::init(cx);
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
            _assistance: None,
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

    /// Wait until the core's startup check for a live Lichess game has ended and engine help is
    /// lent (a profile with no account has nothing to find). The core ends it at once, so a
    /// board that never lifts the lock fails within 10 s, inside the 20 s `cargo mutants` gives
    /// a test, which would otherwise report a hang instead of a caught mutant.
    pub fn wait_for_engine_help(&mut self) {
        self.wait_within(
            Duration::from_secs(10),
            "the startup check to end",
            |_, cx| crate::live_game::handle(cx).read(cx).assistance_allowed(),
        );
    }

    /// The test's app, to run its tasks or read its state.
    pub fn cx(&mut self) -> &mut gpui_kit::TestAppContext {
        &mut self.cx
    }
}

#[cfg(test)]
mod tests {
    use super::Started;

    /// A status answer counts only while no report has come since it was asked: a report is
    /// newer.
    #[test]
    fn a_report_outdates_the_status_answer() {
        let reports = super::Reports::default();
        let asked = reports.seen();
        assert!(reports.none_since(asked));
        reports.report();
        assert!(!reports.none_since(asked));
        let again = reports.seen();
        assert!(reports.none_since(again));
        reports.report();
        assert!(!reports.none_since(again));
    }

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
