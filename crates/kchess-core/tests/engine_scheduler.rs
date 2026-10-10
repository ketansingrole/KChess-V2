//! Port of the shared-budget cases in `tests/core/uci-lifecycle.test.ts`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kchess_core::engine::scheduler::Scheduler;
use kchess_core::engine::uci::search_cancelled;
use kchess_core::error::CoreError;
use tokio_util::sync::CancellationToken;

/// Lets spawned acquisitions reach their queue position.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test]
async fn preempts_lower_priority_work_and_releases_queued_work_in_priority_order() {
    let scheduler = Scheduler::with_budget(1, || false);
    let stops = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&stops);
    let release = scheduler
        .acquire(
            1,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
            None,
        )
        .await
        .expect("first lease");

    let analysis = tokio::spawn({
        let scheduler = scheduler.clone();
        async move { scheduler.acquire(2, || {}, None).await }
    });
    settle().await;
    let computer = tokio::spawn({
        let scheduler = scheduler.clone();
        async move { scheduler.acquire(3, || {}, None).await }
    });
    settle().await;
    assert!(
        stops.load(Ordering::SeqCst) > 0,
        "higher priority preempts the active lease"
    );

    drop(release);
    let release_computer = computer.await.expect("join").expect("computer lease");
    settle().await;
    assert!(
        !analysis.is_finished(),
        "analysis waits while computer holds the budget"
    );

    drop(release_computer);
    let release_analysis = analysis.await.expect("join").expect("analysis lease");
    drop(release_analysis);
}

#[tokio::test]
async fn removes_cancelled_queued_work() {
    let scheduler = Scheduler::with_budget(1, || false);
    let release = scheduler
        .acquire(3, || {}, None)
        .await
        .expect("first lease");
    let cancel = CancellationToken::new();
    let queued = tokio::spawn({
        let scheduler = scheduler.clone();
        let cancel = cancel.clone();
        async move { scheduler.acquire(1, || {}, Some(&cancel)).await.map(|_| ()) }
    });
    settle().await;
    cancel.cancel();
    let error = queued.await.expect("join").unwrap_err();
    assert_eq!(error, search_cancelled());
    drop(release);
    // The cancelled waiter left no lease behind: the budget is free again.
    let next = tokio::time::timeout(Duration::from_secs(1), scheduler.acquire(1, || {}, None))
        .await
        .expect("budget is free")
        .expect("lease");
    drop(next);
}

#[tokio::test]
async fn releases_the_shared_engine_lease_after_success_failure_and_cancel() {
    for outcome in ["success", "failure", "cancel"] {
        let scheduler = Scheduler::with_budget(1, || false);
        let cancel = CancellationToken::new();
        let work = scheduler.with_lease(
            1,
            {
                let cancel = cancel.clone();
                move || cancel.cancel()
            },
            &cancel,
            async {
                match outcome {
                    "failure" => Err(CoreError::new("search failed")),
                    "cancel" => {
                        cancel.cancel();
                        Err(search_cancelled())
                    }
                    _ => Ok("bestmove"),
                }
            },
        );
        match outcome {
            "success" => assert_eq!(work.await.expect("result"), "bestmove"),
            _ => assert!(work.await.is_err(), "{outcome} reports an error"),
        }
        // Whatever the outcome, the lease was released: a new acquisition does not wait.
        let lease = tokio::time::timeout(Duration::from_secs(1), scheduler.acquire(1, || {}, None))
            .await
            .unwrap_or_else(|_| panic!("lease leaked after {outcome}"))
            .expect("lease");
        drop(lease);
    }
}

#[tokio::test]
async fn an_aborted_signal_never_starts_work() {
    let scheduler = Scheduler::with_budget(1, || false);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let ran = AtomicUsize::new(0);
    let result = scheduler
        .with_lease(1, || {}, &cancel, async {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok::<(), CoreError>(())
        })
        .await;
    assert!(result.is_err());
    assert_eq!(ran.load(Ordering::SeqCst), 0);
}
