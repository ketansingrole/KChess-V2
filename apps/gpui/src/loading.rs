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
    pub fn next(&mut self) -> Turn {
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
        self.epoch.next()
    }

    /// Nothing is asked after all (no account to ask for): earlier answers no longer count.
    pub fn stop(&mut self) {
        self.epoch.invalidate();
        self.busy = false;
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
            let _ = this.update(cx, |this, cx| {
                done(this, answer, cx);
                cx.notify();
            });
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
        loading.failed("test", &CoreError::new("offline"));
        assert_eq!(loading.error().map(|error| error.as_ref()), Some("offline"));
        assert!(loading.start() != second && loading.error().is_none());
        loading.stop();
        assert!(!loading.busy());
    }
}
