mod support;
use futures_util::future::BoxFuture;
use kchess_core::engine::review::{
    GameToReview, ReviewRequest, ReviewStorage, ReviewUpdate, Reviews, StoredReview,
};
use kchess_core::error::Result;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;
struct Gated {
    read: Notify,
    release: Notify,
}
impl ReviewStorage for Gated {
    fn read_review<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Option<StoredReview>>> {
        Box::pin(async move {
            self.read.notify_one();
            self.release.notified().await;
            Ok(None)
        })
    }
    fn write_review<'a>(&'a self, _: StoredReview) -> BoxFuture<'a, Result<ReviewUpdate>> {
        Box::pin(async { unreachable!() })
    }
    fn has_account<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async { Ok(true) })
    }
    fn mark_checked<'a>(&'a self, _: &'a [String]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn games_to_review<'a>(
        &'a self,
        _: &'a [String],
        _: i64,
    ) -> BoxFuture<'a, Result<Vec<GameToReview>>> {
        Box::pin(async { Ok(vec![]) })
    }
}
#[tokio::test]
async fn cancel_during_storage_admission_does_not_enqueue_later() {
    // How long admission may take before the test calls it stuck (a regression
    // must fail here, not hang CI until the job timeout).
    const LIMIT: Duration = Duration::from_secs(5);
    let store = Arc::new(Gated {
        read: Notify::new(),
        release: Notify::new(),
    });
    let recording = support::host();
    let context = support::context(
        &recording,
        support::scheduler(4, false),
        "review",
        &support::log_path("audit-cancel"),
    );
    let reviews = Reviews::new(context, store.clone());
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let moves = vec!["e2e4".to_string()];
    let key: String = serde_json::from_str(
        &kchess_domain::api::call("reviewKey", &serde_json::json!([fen, moves]).to_string())
            .unwrap(),
    )
    .unwrap();
    let pending = reviews.request_review(ReviewRequest {
        fen: fen.into(),
        moves,
        game_id: None,
        account: None,
    });
    tokio::pin!(pending);
    tokio::time::timeout(LIMIT, async {
        tokio::select! { _ = &mut pending => panic!("request should wait"), _ = store.read.notified() => {} }
    })
    .await
    .expect("admission reaches storage");
    reviews.cancel_review(&key);
    store.release.notify_one();
    tokio::time::timeout(LIMIT, &mut pending)
        .await
        .expect("the cancelled request returns after storage")
        .unwrap();
    let waiting = reviews.review_status().waiting;
    // A later explicit request of the same key is independent of the cancelled one.
    store.release.notify_one();
    tokio::time::timeout(
        LIMIT,
        reviews.request_review(ReviewRequest {
            fen: fen.into(),
            moves: vec!["e2e4".into()],
            game_id: None,
            account: None,
        }),
    )
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
