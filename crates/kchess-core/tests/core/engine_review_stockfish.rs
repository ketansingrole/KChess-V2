//! A real game review with a native Stockfish as the bundled engine (`support::stockfish`).
//!
//! Reviews write castling as king-takes-rook, as the rest of the app does. Without Chess960 mode
//! the bundled engine took `e1h1` for an illegal move and never answered `go`, so every review of
//! a game with castling failed with "Stockfish timed out".

use crate::support;

use std::sync::Arc;
use std::time::Duration;

use kchess_core::engine::EngineContext;
use kchess_core::engine::managed::Target;
use kchess_core::engine::review::ReviewSettings;
use kchess_core::engine::review::{ReviewRequest, Reviews};
use kchess_core::engine::scheduler::Scheduler;
use kchess_core::engine::status::EngineLocations;
use support::reviews::{ReviewDb, ScriptedHost};
use support::wait_until;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// 1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. O-O, castling written king-takes-rook.
const CASTLED: [&str; 7] = ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1h1"];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(
    not(kchess_stockfish),
    ignore = "needs a native Stockfish: set KCHESS_ENGINE_PATH"
)]
async fn bundled_stockfish_reviews_a_game_with_castling() {
    let engine = support::engine("KCHESS_ENGINE_PATH");
    let host = support::host();
    let locations = EngineLocations {
        managed_dir: std::env::temp_dir().join("kchess-no-managed-engine"),
        target: Target::current(),
        bundled: engine,
        variant: std::path::PathBuf::new(),
    };
    let ctx = EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        support::scheduler(4, false),
        locations,
    );
    let store = ReviewDb::new();
    let reviews = Reviews::new(ctx, store.database.clone());
    let review_host = ScriptedHost::new(
        store,
        ReviewSettings {
            review_auto: kchess_contracts::types::ReviewAuto::Off,
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
            variant: None,
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

/// A Najdorf of eighty moves that Stockfish played against itself at depth 8, castling written
/// king-takes-rook: eighty-one positions to review.
const EIGHTY: &str = concat!(
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 a7a6 f1e2 e7e5 d4f5 c8f5 e4f5 d6d5 ",
    "c1g5 b8d7 c3d5 d8a5 d5c3 f8a3 b2a3 a5c3 g5d2 c3c7 c2c4 f6e4 d2e3 e8h8 e1h1 e4c3 ",
    "d1e1 c3e2 e1e2 a8c8 a1c1 h7h6 h2h3 f8d8 f1d1 d7f6 e2b2 c7e7 d1d8 c8d8 b2b4 e7d7 ",
    "c1b1 b7b5 c4b5 f6d5 b4a5 d5e3 f2e3 a6b5 b1b5 e5e4 a5b6 d7d1 g1h2 d8d2 b5b2 d2b2 ",
    "b6b2 d1d6 h2g1 d6c5 b2b3 c5c1 g1h2 c1c7 h2g1 c7c1 g1f2 c1d2 f2g1 d2c1 g1h2 c1c7",
);

/// How long a review someone asked for takes over a real Stockfish, by how many engines the
/// computer has room for (one is a review as it ran before it had several: one engine with
/// every thread). `KCHESS_REVIEW_ENGINES` names the counts to time, `1,2,3,4` without it. Not
/// part of `check`; it prints the figures (`docs/benchmarks/2026-11-20-review-engines.md`):
/// `KCHESS_ENGINE_PATH=<stockfish> cargo test -p kchess-core --test core review_speed -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a benchmark: it needs a native Stockfish (KCHESS_ENGINE_PATH) and prints its figures"]
async fn review_speed_by_engines() {
    const GIB: u64 = 1 << 30;
    let engine = support::engine("KCHESS_ENGINE_PATH");
    let moves: Vec<String> = EIGHTY.split_whitespace().map(str::to_string).collect();
    let counts: Vec<u64> = kchess_host::env::var_os("KCHESS_REVIEW_ENGINES")
        .and_then(|counts| counts.into_string().ok())
        .unwrap_or_else(|| "1,2,3,4".to_string())
        .split(',')
        .map(|count| count.trim().parse().expect("a count of engines"))
        .collect();
    for engines in counts {
        let host = support::host();
        let locations = EngineLocations {
            managed_dir: std::env::temp_dir().join("kchess-no-managed-engine"),
            target: Target::current(),
            bundled: engine.clone(),
            variant: std::path::PathBuf::new(),
        };
        // Two gibibytes of memory make room for one engine.
        let scheduler = Scheduler::with_memory(4, Some(engines * 2 * GIB), || false);
        let ctx = EngineContext::new(
            host.clone(),
            Arc::new(support::hub(&host)),
            Arc::new(scheduler),
            locations,
        );
        let store = ReviewDb::new();
        let reviews = Reviews::new(ctx, store.database.clone());
        let review_host = ScriptedHost::new(
            store,
            ReviewSettings {
                review_auto: kchess_contracts::types::ReviewAuto::Off,
                review_on_battery: true,
                engine_path: String::new(),
            },
        );
        reviews.setup_reviews(review_host.clone());
        let started = std::time::Instant::now();
        reviews
            .request_review(ReviewRequest {
                fen: START.to_string(),
                moves: moves.clone(),
                game_id: None,
                account: None,
                variant: None,
            })
            .await
            .expect("the request");
        let done = wait_until(Duration::from_secs(900), "the complete review", || {
            review_host.complete_updates().into_iter().next()
        })
        .await;
        let took = started.elapsed();
        let short = done
            .evals
            .iter()
            .flatten()
            .filter(|eval| eval.depth.is_some_and(|depth| depth < 18.0))
            .count();
        println!(
            "{engines} engine(s): {:.1} s for {} positions, {short} short of depth 18",
            took.as_secs_f64(),
            done.evals.len()
        );
        reviews.stop_reviews().await;
    }
}
