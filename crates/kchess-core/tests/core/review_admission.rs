use crate::support;
use kchess_core::engine::review::{ReviewRequest, Reviews};
use std::task::Poll;
use std::time::Duration;
use support::reviews::ReviewDb;

fn request(fen: &str) -> ReviewRequest {
    ReviewRequest {
        fen: fen.into(),
        moves: vec!["e2e4".into()],
        game_id: None,
        account: None,
        variant: None,
    }
}

#[tokio::test]
async fn cancel_during_storage_admission_does_not_enqueue_later() {
    // How long admission may take before the test calls it stuck (a regression
    // must fail here, not hang CI until the job timeout).
    const LIMIT: Duration = Duration::from_secs(5);
    let store = ReviewDb::new();
    let recording = support::host();
    let context = support::context(
        &recording,
        support::scheduler(4, false),
        "review",
        &support::log_path("audit-cancel"),
    );
    let reviews = Reviews::new(context, store.database.clone());
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let key = kchess_domain::records::review::review_key(fen, &["e2e4".to_string()], None);
    // The store is busy, so the request waits for it with its review admitted.
    let held = store
        .database
        .hold_execution()
        .await
        .expect("hold the store");
    let pending = reviews.request_review(request(fen));
    tokio::pin!(pending);
    assert!(
        matches!(futures_util::poll!(&mut pending), Poll::Pending),
        "the request waits for the store"
    );
    reviews.cancel_review(&key);
    drop(held);
    tokio::time::timeout(LIMIT, &mut pending)
        .await
        .expect("the cancelled request returns after storage")
        .unwrap();
    let waiting = reviews.review_status().waiting;
    // A later explicit request of the same key is independent of the cancelled one.
    tokio::time::timeout(LIMIT, reviews.request_review(request(fen)))
        .await
        .expect("a later request completes")
        .unwrap();
    assert_eq!(
        reviews.review_status().waiting,
        1,
        "a new request may queue"
    );
    reviews.stop_reviews().await;
    assert_eq!(
        waiting, 0,
        "cancelled request must not be enqueued after storage returns"
    );
}
