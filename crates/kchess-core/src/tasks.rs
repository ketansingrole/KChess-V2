//! Spawned work that panics must still end the way failed work does. tokio catches a task's
//! panic and drops it, so a loop that reports its own end (a stream's last state, an engine's
//! failure) would otherwise stop silently and leave its owner waiting. The panic hook
//! (`panics.rs`) has already logged the panic; these helpers let the owner report it.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::task::{Context, Poll, Waker};

use futures_util::FutureExt;

use kchess_contracts::error::CoreError;

/// The message of work stopped because its caller cancelled it.
pub const ABORTED: &str = "This operation was aborted";

/// The error of work stopped because its caller cancelled it.
pub(crate) fn aborted() -> CoreError {
    CoreError::aborted(ABORTED)
}

/// Starts `work` here and now, and hands back the rest of it to await or spawn.
///
/// A spawned task first runs whenever a worker picks it up, so two calls spawned one after the
/// other can start in either order: a stop sent before a start could reach the engine after it.
/// What runs `work` up to its first wait here keeps the order its caller made the calls in.
/// The rest runs wherever the returned future is awaited, which polls `work` again with its own
/// waker before it waits.
pub fn started<F>(work: F) -> impl Future<Output = F::Output> + Send + 'static + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let mut work = Box::pin(work);
    // Past tokio's budget for this task, a lock or a queue answers "not yet" without taking a
    // place in line, which would give the order away.
    let first = tokio::task::unconstrained(work.as_mut())
        .poll_unpin(&mut Context::from_waker(Waker::noop()));
    async move {
        match first {
            Poll::Ready(output) => output,
            Poll::Pending => work.await,
        }
    }
}

/// `work` panicked; the panic hook has logged where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Panicked;

/// Runs `work`, turning a panic inside it into `Err(Panicked)`.
pub(crate) async fn caught<T>(work: impl Future<Output = T>) -> Result<T, Panicked> {
    AssertUnwindSafe(work)
        .catch_unwind()
        .await
        .map_err(|_| Panicked)
}

/// The value behind `mutex`, even when a holder panicked. Google's Rust style guide says never to
/// reach inside a poisoned mutex; the core does on purpose. A panic in spawned work is logged and
/// reported as a failure (`caught`) and the core keeps serving, so a lock that every later request
/// shares (the Lichess counters, an engine's pipes, the online state) must not fail them all.
/// Every holder leaves its data whole between statements, so what a panic leaves is usable.
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs its closure when dropped: bookkeeping a call adds before an `.await` and must undo after
/// it, on every path. A call whose future is dropped (a socket client cancels it or disconnects,
/// which aborts its task) never reaches the code after the `.await`.
pub(crate) struct OnDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(undo) = self.0.take() {
            undo();
        }
    }
}

/// `undo` runs when the returned guard is dropped.
pub(crate) fn on_drop<F: FnOnce()>(undo: F) -> OnDrop<F> {
    OnDrop(Some(undo))
}

/// `tokio::spawn` for detached work: `on_panic` runs if `work` panics.
pub(crate) fn spawn_guarded(
    work: impl Future<Output = ()> + Send + 'static,
    on_panic: impl FnOnce() + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if caught(work).await.is_err() {
            on_panic();
        }
    })
}

/// The answer of a run going, once it has one.
type Answer<T> = tokio::sync::watch::Receiver<Option<T>>;

/// Runs by key, one at a time: a run asked for while one of its key is going joins it and
/// answers with its result. A forgotten key starts afresh on the next run, while the run going
/// still answers those waiting on it.
pub(crate) struct SingleFlight<T> {
    runs: std::sync::Mutex<std::collections::HashMap<String, (u64, Answer<T>)>>,
    next: std::sync::atomic::AtomicU64,
}

impl<T> Default for SingleFlight<T> {
    fn default() -> Self {
        SingleFlight {
            runs: std::sync::Mutex::new(std::collections::HashMap::new()),
            next: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl<T: Clone> SingleFlight<T> {
    /// `run`'s result, or that of the run of `key` already going; `cancelled` when that one was
    /// dropped before it answered.
    pub(crate) async fn run(
        &self,
        key: &str,
        run: impl Future<Output = T>,
        cancelled: impl Fn() -> T,
    ) -> T {
        let joined = lock(&self.runs)
            .get(key)
            .map(|(_, outcome)| outcome.clone());
        if let Some(mut outcome) = joined {
            loop {
                if let Some(result) = outcome.borrow().clone() {
                    return result;
                }
                if outcome.changed().await.is_err() {
                    return cancelled();
                }
            }
        }
        let generation = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (send, receive) = tokio::sync::watch::channel(None);
        lock(&self.runs).insert(key.to_string(), (generation, receive));
        // Forgotten however the run ends: a cancelled call once left its entry behind, and every
        // later run of its key joined it and failed at once.
        let forget = on_drop(|| {
            let mut runs = lock(&self.runs);
            if runs
                .get(key)
                .is_some_and(|(current, _)| *current == generation)
            {
                runs.remove(key);
            }
        });
        let result = run.await;
        drop(forget);
        let _ = send.send(Some(result.clone()));
        result
    }

    /// Later runs of `key` start afresh.
    pub(crate) fn forget(&self, key: &str) {
        lock(&self.runs).remove(key);
    }

    /// The keys running now.
    pub(crate) fn running(&self) -> Vec<String> {
        lock(&self.runs).keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A run asked for while its key runs answers with that run; a cancelled run leaves nothing
    /// for the next to join.
    #[tokio::test]
    async fn a_second_run_joins_the_first() {
        let flight = SingleFlight::<u32>::default();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let first = flight.run(
            "a",
            async {
                let _ = wait.await;
                1
            },
            || 0,
        );
        let second = flight.run("a", async { 2 }, || 0);
        let (first, second) = tokio::join!(first, async {
            tokio::task::yield_now().await;
            let _ = release.send(());
            second.await
        });
        assert_eq!((first, second), (1, 1));
        assert!(flight.running().is_empty());
        let cancelled = flight.run("b", std::future::pending::<u32>(), || 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), cancelled)
                .await
                .is_err()
        );
        assert_eq!(flight.run("b", async { 3 }, || 0).await, 3);
    }

    /// A forgotten key's next run starts afresh while the first still runs, and the first one
    /// ending leaves the new run in place for later runs to join.
    #[tokio::test]
    async fn a_forgotten_key_runs_afresh() {
        let flight = Arc::new(SingleFlight::<u32>::default());
        let start = |value: u32| {
            let (release, wait) = tokio::sync::oneshot::channel::<()>();
            let flight = Arc::clone(&flight);
            let run = tokio::spawn(async move {
                let work = async move {
                    let _ = wait.await;
                    value
                };
                flight.run("a", work, || 0).await
            });
            (release, run)
        };
        let (release_first, first) = start(1);
        tokio::task::yield_now().await;
        assert_eq!(flight.running(), ["a"]);
        flight.forget("a");
        assert!(flight.running().is_empty());
        let (release_second, second) = start(2);
        tokio::task::yield_now().await;
        let _ = release_first.send(());
        assert_eq!(first.await.unwrap(), 1);
        assert_eq!(flight.running(), ["a"]);
        let (_, third) = start(3);
        tokio::task::yield_now().await;
        let _ = release_second.send(());
        assert_eq!((second.await.unwrap(), third.await.unwrap()), (2, 2));
    }

    #[tokio::test]
    async fn a_panic_becomes_an_error() {
        assert_eq!(caught(async { 7 }).await, Ok(7));
        assert_eq!(
            caught(async { panic!("boom") }).await,
            Err::<(), _>(Panicked)
        );
    }

    #[tokio::test]
    async fn detached_work_reports_its_panic() {
        let reported = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&reported);
        spawn_guarded(async { panic!("boom") }, move || {
            flag.store(true, Ordering::SeqCst)
        })
        .await
        .unwrap();
        assert!(reported.load(Ordering::SeqCst));
        let quiet = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&quiet);
        spawn_guarded(async {}, move || flag.store(true, Ordering::SeqCst))
            .await
            .unwrap();
        assert!(!quiet.load(Ordering::SeqCst));
    }
}
