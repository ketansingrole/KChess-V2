//! A real game review with a native Stockfish as the bundled engine (`support::stockfish`).
//!
//! Reviews write castling as king-takes-rook, as the rest of the app does. Without Chess960 mode
//! the bundled engine took `e1h1` for an illegal move and never answered `go`, so every review of
//! a game with castling failed with "Stockfish timed out".

mod support;

use std::sync::Arc;
use std::time::Duration;

use kchess_core::engine::EngineContext;
use kchess_core::engine::managed::Target;
use kchess_core::engine::review::ReviewSettings;
use kchess_core::engine::review::{ReviewRequest, Reviews};
use kchess_core::engine::status::EngineLocations;
use support::review_store::{MemoryStore, ScriptedHost};
use support::wait_until;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// 1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. O-O, castling written king-takes-rook.
const CASTLED: [&str; 7] = ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1h1"];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bundled_stockfish_reviews_a_game_with_castling() {
    let Some(engine) = support::stockfish() else {
        return;
    };
    let host = support::host();
    let locations = EngineLocations {
        managed_dir: std::env::temp_dir().join("kchess-no-managed-engine"),
        target: Target::current(),
        bundled: engine,
    };
    let ctx = EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        support::scheduler(4, false),
        locations,
    );
    let store = MemoryStore::new();
    let reviews = Reviews::new(ctx, store.clone());
    let review_host = ScriptedHost::new(
        store,
        ReviewSettings {
            review_auto: "off".into(),
            review_on_battery: true,
            engine_path: String::new(),
        },
    );
    reviews.setup_reviews(review_host.clone());
    reviews
        .request_review(ReviewRequest {
            fen: START.to_string(),
            moves: CASTLED.iter().map(|mv| (*mv).to_string()).collect(),
            game_id: None,
            account: None,
        })
        .await
        .expect("the request");
    let done = wait_until(Duration::from_secs(40), "the complete review", || {
        review_host.complete_updates().into_iter().next()
    })
    .await;
    assert_eq!(done.evals.len(), CASTLED.len() + 1);
    assert!(done.evals.iter().all(Option::is_some));
    reviews.stop_reviews().await;
}
