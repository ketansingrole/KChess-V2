//! The shared engine budget: one lease at a time, in priority order.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;

use futures_util::poll;
use kchess_core::engine::scheduler::{Priority, Scheduler};
use kchess_core::engine::uci::search_cancelled;
use kchess_core::error::CoreError;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn preempts_lower_priority_work_and_releases_queued_work_in_priority_order() {
    let scheduler = Scheduler::with_budget(1, || false);
    let review = CancellationToken::new();
    let release = scheduler
        .acquire(Priority::Review, &review)
        .await
        .expect("first lease");

    // One poll queues each acquisition: the waiter is in line before its first await.
    let analysis_token = CancellationToken::new();
    let analysis = scheduler.acquire(Priority::Analysis, &analysis_token);
    tokio::pin!(analysis);
    assert!(poll!(&mut analysis).is_pending());
    assert!(review.is_cancelled(), "analysis preempts the review");

    let computer_token = CancellationToken::new();
    let computer = scheduler.acquire(Priority::Computer, &computer_token);
    tokio::pin!(computer);
    assert!(poll!(&mut computer).is_pending());
    assert!(!analysis_token.is_cancelled(), "a waiter is not preempted");

    drop(release);
    let release_computer = computer.await.expect("computer lease");
    assert!(
        poll!(&mut analysis).is_pending(),
        "analysis waits while the computer holds the budget"
    );

    drop(release_computer);
    let Poll::Ready(release_analysis) = poll!(&mut analysis) else {
        panic!("analysis is granted once the computer is done");
    };
    drop(release_analysis.expect("analysis lease"));
}

#[tokio::test]
async fn removes_cancelled_queued_work() {
    let scheduler = Scheduler::with_budget(1, || false);
    let release = scheduler
        .acquire(Priority::Computer, &CancellationToken::new())
        .await
        .expect("first lease");
    let cancel = CancellationToken::new();
    let queued = scheduler.acquire(Priority::Review, &cancel);
    tokio::pin!(queued);
    assert!(poll!(&mut queued).is_pending());
    cancel.cancel();
    let error = queued.await.map(|_| ()).unwrap_err();
    assert_eq!(error, search_cancelled());
    drop(release);
    // The cancelled waiter left no lease behind: the budget is free again.
    let next = tokio::time::timeout(
        Duration::from_secs(1),
        scheduler.acquire(Priority::Review, &CancellationToken::new()),
    )
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
        let work = scheduler.with_lease(Priority::Review, &cancel, async {
            match outcome {
                "failure" => Err(CoreError::new("search failed")),
                "cancel" => {
                    cancel.cancel();
                    Err(search_cancelled())
                }
                _ => Ok("bestmove"),
            }
        });
        match outcome {
            "success" => assert_eq!(work.await.expect("result"), "bestmove"),
            _ => assert!(work.await.is_err(), "{outcome} reports an error"),
        }
        // Whatever the outcome, the lease was released: a new acquisition does not wait.
        let lease = tokio::time::timeout(
            Duration::from_secs(1),
            scheduler.acquire(Priority::Review, &CancellationToken::new()),
        )
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
        .with_lease(Priority::Review, &cancel, async {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok::<(), CoreError>(())
        })
        .await;
    assert!(result.is_err());
    assert_eq!(ran.load(Ordering::SeqCst), 0);
}

/// The scheduler asks for a fresh battery reading before every lease, without waiting for it.
#[tokio::test]
async fn every_lease_refreshes_the_battery_reading() {
    struct Counted(std::sync::Arc<AtomicUsize>);
    impl kchess_core::engine::scheduler::Power for Counted {
        fn on_battery(&self) -> bool {
            false
        }
        fn refresh(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let refreshes = std::sync::Arc::new(AtomicUsize::new(0));
    let scheduler = Scheduler::with_budget(1, Counted(std::sync::Arc::clone(&refreshes)));
    for _ in 0..2 {
        let lease = scheduler
            .acquire(Priority::Review, &CancellationToken::new())
            .await
            .expect("lease");
        drop(lease);
    }
    assert_eq!(refreshes.load(Ordering::SeqCst), 2);
}
