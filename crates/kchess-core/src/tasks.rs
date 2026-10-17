//! Spawned work that panics must still end the way failed work does. tokio catches a task's
//! panic and drops it, so a loop that reports its own end (a stream's last state, an engine's
//! failure) would otherwise stop silently and leave its owner waiting. The panic hook
//! (`panics.rs`) has already logged the panic; these helpers let the owner report it.

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

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
