//! A page's requests to the core: the latest answer alone counts, the page says it is loading
//! until then, and a failure is logged as well as shown.

use std::future::Future;

use gpui_kit::*;
use kchess_contracts::{CoreError, Level};

/// One request of a page's: its answer is the page's only while no later one was sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn(u64);

/// Which of a page's requests is the latest: only its answer counts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Epoch(u64);

impl Epoch {
    /// A request goes out: answers to the ones before it no longer count.
    pub fn start(&mut self) -> Turn {
        self.0 += 1;
        Turn(self.0)
    }

    /// Answers to every request so far no longer count.
    pub fn invalidate(&mut self) {
        self.0 += 1;
    }

    /// The latest request.
    pub fn current(&self) -> Turn {
        Turn(self.0)
    }

    /// Whether `turn` is the latest request.
    pub fn is_current(&self, turn: Turn) -> bool {
        turn == Turn(self.0)
    }
}

impl std::fmt::Display for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Where a page's loading of something is: whether it waits for an answer, and what failed.
#[derive(Debug, Default)]
pub struct Loading {
    epoch: Epoch,
    busy: bool,
    error: Option<SharedString>,
}

impl Loading {
    /// A request goes out: answers to the ones before it no longer count.
    pub fn start(&mut self) -> Turn {
        self.busy = true;
        self.error = None;
        self.epoch.start()
    }

    /// Ask the core for what the page shows, the way every page loads: answers to earlier loads no
    /// longer count, the page is busy until this one's answer, and a failure is logged under
    /// `area` and shown (`error`). `done` receives the latest answer, a failure included, to keep
    /// or clear what the page showed; `slot` finds this `Loading` in the view again.
    pub fn load<V: 'static, T: 'static>(
        &mut self,
        cx: &mut Context<V>,
        area: &'static str,
        slot: fn(&mut V) -> &mut Loading,
        request: impl Future<Output = Result<T, CoreError>> + 'static,
        done: impl FnOnce(&mut V, Result<T, CoreError>, &mut Context<V>) + 'static,
    ) {
        let turn = self.start();
        cx.spawn(async move |this, cx| {
            let answer = request.await;
            this.update(cx, |view, cx| {
                if slot(view).settle(turn, area, &answer) {
                    done(view, answer, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The answer to `turn` arrived: whether it is the latest request's, which ends the loading,
    /// and if it failed, the failure is logged under `area` and shown.
    pub fn settle<T>(&mut self, turn: Turn, area: &str, answer: &Result<T, CoreError>) -> bool {
        if !self.answered(turn) {
            return false;
        }
        if let Err(cause) = answer {
            self.failed(area, cause);
        }
        true
    }

    /// Nothing is asked after all (no account to ask for): earlier answers no longer count, and
    /// no failure is shown.
    pub fn stop(&mut self) {
        self.epoch.invalidate();
        self.busy = false;
        self.error = None;
    }

    /// Nothing is asked because what would be asked is wrong (a name that cannot be one): earlier
    /// answers no longer count, and `message` is shown as the failure.
    pub fn refuse(&mut self, message: impl Into<SharedString>) {
        self.stop();
        self.error = Some(message.into());
    }

    /// What failed is fine now (a later save worked): no failure is shown.
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Show `message` in place of the failure's own (the logged one stays the core's).
    pub fn show_error(&mut self, message: impl Into<SharedString>) {
        self.error = Some(message.into());
    }

    /// The latest request, for the requests that follow from its answer (a player's games after
    /// their profile): they count only while it is still the latest.
    pub fn current(&self) -> Turn {
        self.epoch.current()
    }

    /// Whether `turn` is the latest request, without ending the loading.
    pub fn is_current(&self, turn: Turn) -> bool {
        self.epoch.is_current(turn)
    }

    /// The answer to `turn` arrived: whether it is the latest request's, which ends the loading.
    pub fn answered(&mut self, turn: Turn) -> bool {
        if !self.epoch.is_current(turn) {
            return false;
        }
        self.busy = false;
        true
    }

    /// The request failed: logged under `area`, and shown as its message.
    pub fn failed(&mut self, area: &str, cause: &CoreError) {
        crate::log::write(Level::Warn, area, &cause.message);
        self.error = Some(cause.message.clone().into());
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }
}

/// A failure the page shows outside a `Loading` (an action's, a stream's): logged under `area`,
/// and its message to show.
pub fn failure(area: &str, cause: &CoreError) -> String {
    crate::log::write(Level::Warn, area, &cause.message);
    cause.message.clone()
}

/// Ask the core for something the page shows when it can: its answer goes to `done`, and a
/// failure is logged under `area`, the page keeping what it showed.
pub fn fetch<V: 'static, T: 'static>(
    cx: &mut Context<V>,
    area: &'static str,
    request: impl Future<Output = Result<T, CoreError>> + 'static,
    done: impl FnOnce(&mut V, T, &mut Context<V>) + 'static,
) {
    cx.spawn(async move |this, cx| match request.await {
        Ok(answer) => {
            this.update(cx, |this, cx| {
                done(this, answer, cx);
                cx.notify();
            })
            .ok();
        }
        Err(cause) => crate::log::write(Level::Warn, area, &cause.message),
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::Loading;
    use kchess_contracts::CoreError;

    /// An answer to a request made before the latest one is dropped, and loading goes on.
    #[test]
    fn a_stale_answer_does_not_count() {
        let mut loading = Loading::default();
        let first = loading.start();
        let second = loading.start();
        assert!(!loading.answered(first));
        assert!(loading.busy());
        assert!(loading.answered(second));
        assert!(!loading.busy());
        loading.failed("loading-test", &CoreError::new("offline"));
        assert_eq!(loading.error().map(|error| error.as_ref()), Some("offline"));
        assert!(loading.start() != second && loading.error().is_none());
        loading.stop();
        assert!(!loading.busy());
    }

    /// Nothing asked shows no failure; a refused request shows its own message.
    #[test]
    fn stopping_clears_the_failure_and_refusing_shows_its_own() {
        let mut loading = Loading::default();
        let turn = loading.start();
        loading.failed("loading-test", &CoreError::new("offline"));
        loading.stop();
        assert!(loading.error().is_none() && !loading.busy());
        assert!(
            !loading.is_current(turn),
            "an answer from before stopping no longer counts"
        );
        loading.start();
        loading.refuse("Enter a name");
        assert_eq!(
            loading.error().map(|error| error.as_ref()),
            Some("Enter a name")
        );
        assert!(!loading.busy());
        let turn = loading.start();
        assert!(loading.is_current(turn) && loading.current() == turn);
        loading.show_error("Lichess is busy");
        assert_eq!(
            loading.error().map(|error| error.as_ref()),
            Some("Lichess is busy")
        );
        loading.clear_error();
        assert!(
            loading.error().is_none() && loading.busy(),
            "only the failure is cleared"
        );
    }

    /// A failure the page shows reaches the log too, under its area.
    #[test]
    fn a_shown_failure_is_logged() {
        crate::tests::logged();
        let shown = super::failure("loading-test-area", &CoreError::new("the disk is full"));
        assert_eq!(shown, "the disk is full");
        let mut loading = Loading::default();
        let turn = loading.start();
        let settled = loading.settle(
            turn,
            "loading-test-settle",
            &Err::<(), _>(CoreError::new("no answer")),
        );
        assert!(settled && !loading.busy());
        assert_eq!(
            loading.error().map(|error| error.as_ref()),
            Some("no answer")
        );
        let log = crate::tests::logged();
        assert!(
            log.contains("[loading-test-area] the disk is full"),
            "{log}"
        );
        assert!(log.contains("[loading-test-settle] no answer"), "{log}");
    }

    /// `load`: only the latest request's answer reaches the page, a failure included, however
    /// the answers interleave (`SEED=<n>` replays an order); a stale failure is never shown.
    #[gpui_kit::test(iterations = 64)]
    fn load_shows_only_the_latest_answer(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;

        #[derive(Default)]
        struct Page {
            loading: Loading,
            shown: Vec<Result<u64, String>>,
        }
        const REQUESTS: u64 = 6;
        let executor = cx.executor();
        let page = cx.new(|_| Page::default());
        page.update(cx, |page, cx| {
            for request in 0..REQUESTS {
                let delay = executor.simulate_random_delay();
                // Every answer but the last fails: none of those failures may show.
                let answer = async move {
                    delay.await;
                    if request + 1 == REQUESTS {
                        Ok(request)
                    } else {
                        Err(CoreError::new(format!("failure {request}")))
                    }
                };
                page.loading.load(
                    cx,
                    "loading-test",
                    |page: &mut Page| &mut page.loading,
                    answer,
                    |page, answer, _| page.shown.push(answer.map_err(|cause| cause.message)),
                );
            }
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(
                page.shown,
                [Ok(REQUESTS - 1)],
                "only the latest answer shows"
            );
            assert!(!page.loading.busy());
            assert!(page.loading.error().is_none(), "no stale failure shows");
        });
    }

    /// `load`'s failure is shown, and `done` still hears of it to clear what it showed.
    #[gpui_kit::test]
    fn load_shows_the_latest_failure(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;

        #[derive(Default)]
        struct Page {
            loading: Loading,
            report: Option<u32>,
        }
        let page = cx.new(|_| Page {
            report: Some(7),
            ..Page::default()
        });
        page.update(cx, |page, cx| {
            page.loading.load(
                cx,
                "loading-test",
                |page: &mut Page| &mut page.loading,
                async { Err::<u32, _>(CoreError::new("Lichess did not answer")) },
                |page, answer, _| page.report = answer.ok(),
            );
            assert!(page.loading.busy());
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert!(!page.loading.busy());
            assert_eq!(page.report, None);
            assert_eq!(
                page.loading.error().map(|error| error.as_ref()),
                Some("Lichess did not answer")
            );
        });
    }

    /// `fetch` hands its answer to the page; a failure is logged, and the page keeps what it
    /// showed.
    #[gpui_kit::test]
    fn fetch_shows_an_answer_and_logs_a_failure(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;

        struct Page {
            shown: Option<u32>,
        }
        let page = cx.new(|_| Page { shown: None });
        page.update(cx, |_, cx| {
            super::fetch(
                cx,
                "fetch-test",
                async { Ok::<u32, CoreError>(7) },
                |page, answer, _| page.shown = Some(answer),
            );
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.shown, Some(7)));
        let before = crate::tests::logged_in("fetch-test");
        page.update(cx, |_, cx| {
            super::fetch(
                cx,
                "fetch-test",
                async { Err::<u32, _>(CoreError::new("Lichess did not answer")) },
                |page, answer, _| page.shown = Some(answer),
            );
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.shown, Some(7)));
        assert_eq!(crate::tests::logged_in("fetch-test"), before + 1);
    }

    /// Answers come back in whatever order the core sends them: GPUI's test scheduler runs the
    /// tasks in a different order under each seed (`SEED=<n>` replays one). The page shows the
    /// latest request's answer however the earlier ones interleave with it, and stops loading.
    #[gpui_kit::test(iterations = 64)]
    fn the_latest_request_wins_in_any_order_of_answers(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;

        #[derive(Default)]
        struct Page {
            loading: Loading,
            shown: Option<u64>,
            answers: u64,
        }
        const REQUESTS: u64 = 6;
        let executor = cx.executor();
        let page = cx.new(|_| Page::default());
        page.update(cx, |page, cx| {
            for request in 0..REQUESTS {
                let turn = page.loading.start();
                let delay = executor.simulate_random_delay();
                cx.spawn(async move |this, cx| {
                    delay.await;
                    this.update(cx, |page: &mut Page, _| {
                        page.answers += 1;
                        if page.loading.answered(turn) {
                            page.shown = Some(request);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        });
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.answers, REQUESTS, "every answer arrived");
            assert_eq!(
                page.shown,
                Some(REQUESTS - 1),
                "only the latest request's shows"
            );
            assert!(!page.loading.busy());
        });
    }
}
