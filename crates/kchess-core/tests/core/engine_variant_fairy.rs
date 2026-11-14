//! Variant analysis with the real Fairy-Stockfish (`support::fairy_stockfish`): a Crazyhouse
//! position analysed with drops in its lines, an Atomic one under Atomic's rules, and a
//! Crazyhouse game reviewed, and the computer playing Crazyhouse and Horde.

use crate::support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::engine::EngineContext;
use kchess_core::engine::analysis::{Analysis, AnalysisSink, AnalysisUpdate};
use serde_json::json;

/// The first update of the search that `done` accepts.
async fn analyse(
    engine: std::path::PathBuf,
    fen: &str,
    variant: &str,
    done: impl Fn(&AnalysisUpdate) -> bool,
) -> AnalysisUpdate {
    let host = support::host();
    let mut locations = support::locations();
    locations.variant = engine;
    let ctx = EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        support::scheduler(2, false),
        locations,
    );
    let analysis = Analysis::new(ctx);
    let updates: Arc<Mutex<Vec<AnalysisUpdate>>> = Arc::default();
    let recorded = Arc::clone(&updates);
    let sink: AnalysisSink = Arc::new(move |update| recorded.lock().expect("updates").push(update));
    let id = analysis
        .start_analysis(
            json!({ "fen": fen, "lines": 3, "infinite": true, "variant": variant }),
            "",
            sink,
        )
        .expect("a valid request");
    let update = support::wait_until(Duration::from_secs(30), "the lines wanted", || {
        updates
            .lock()
            .expect("updates")
            .iter()
            .find(|u| u.id == id && done(u))
            .cloned()
    })
    .await;
    analysis.stop_analysis(true);
    update
}

#[tokio::test]
#[cfg_attr(
    not(kchess_fairy_stockfish),
    ignore = "needs Fairy-Stockfish: set KCHESS_VARIANT_ENGINE_PATH"
)]
async fn crazyhouse_lines_drop_pieces_from_the_pockets() {
    let engine = support::engine("KCHESS_VARIANT_ENGINE_PATH");
    // 1. e4 d5 2. exd5 Qxd5: a pawn each in hand.
    let update = analyse(
        engine,
        "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3",
        "crazyhouse",
        |update| update.lines.len() == 3 && update.depth >= 10.0,
    )
    .await;
    assert!(
        update.engine.contains("fairy-stockfish"),
        "{}",
        update.engine
    );
    assert!(
        update
            .lines
            .iter()
            .any(|line| line.pv.iter().any(|mv| mv.contains('@'))),
        "a drop in some line: {:?}",
        update.lines
    );
}

#[tokio::test]
#[cfg_attr(
    not(kchess_fairy_stockfish),
    ignore = "needs Fairy-Stockfish: set KCHESS_VARIANT_ENGINE_PATH"
)]
async fn an_atomic_capture_that_explodes_the_king_is_mate() {
    let engine = support::engine("KCHESS_VARIANT_ENGINE_PATH");
    // Qxf7 explodes the king beside f7: White wins at once, which standard rules would not see.
    let update = analyse(
        engine,
        "rnbqkbnr/pppp1ppp/8/4p2Q/8/4P3/PPPP1PPP/RNB1KBNR w KQkq - 0 3",
        "atomic",
        |update| update.lines.first().is_some_and(|line| line.mate.is_some()),
    )
    .await;
    let best = &update.lines[0];
    assert_eq!(
        best.pv.first().map(String::as_str),
        Some("h5f7"),
        "{best:?}"
    );
    assert!(best.mate.is_some_and(|mate| mate > 0.0), "{best:?}");
}

/// A Crazyhouse game with a drop is reviewed by Fairy-Stockfish to the end (standard rules stop
/// at the drop), under a key of its own: the same moves in standard chess are another game.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(
    not(kchess_fairy_stockfish),
    ignore = "needs Fairy-Stockfish: set KCHESS_VARIANT_ENGINE_PATH"
)]
async fn a_crazyhouse_game_is_reviewed_with_its_drops() {
    use kchess_core::engine::review::{ReviewRequest, ReviewSettings, Reviews};
    use support::reviews::{ReviewDb, ScriptedHost};
    let engine = support::engine("KCHESS_VARIANT_ENGINE_PATH");
    let host = support::host();
    let mut locations = support::locations();
    locations.variant = engine;
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
            review_auto: "off".into(),
            review_on_battery: true,
            engine_path: String::new(),
        },
    );
    reviews.setup_reviews(review_host.clone());
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
    let moves: Vec<String> = ["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"]
        .map(String::from)
        .to_vec();
    reviews
        .request_review(ReviewRequest {
            fen: fen.into(),
            moves: moves.clone(),
            game_id: None,
            account: None,
            variant: Some("crazyhouse".into()),
        })
        .await
        .expect("the request");
    let done = wait_until_review(&review_host).await;
    assert_eq!(done.evals.len(), moves.len() + 1);
    assert!(done.evals.iter().all(Option::is_some), "{:?}", done.evals);
    assert_eq!(done.variant.as_deref(), Some("crazyhouse"));
    assert!(
        done.engine
            .as_deref()
            .is_some_and(|engine| engine.contains("fairy-stockfish")),
        "{:?}",
        done.engine
    );
    let standard = kchess_domain::records::review::review_key(fen, &moves, None);
    assert_ne!(done.key, standard);
    // The stored review is found again by its variant.
    let found = reviews
        .get_review(fen, &moves, Some("crazyhouse"))
        .await
        .expect("the lookup");
    assert_eq!(found.map(|review| review.key), Some(done.key));
    reviews.stop_reviews().await;
}

async fn wait_until_review(
    host: &support::reviews::ScriptedHost,
) -> kchess_core::engine::review::StoredReview {
    support::wait_until(Duration::from_secs(60), "the complete review", || {
        host.complete_updates().into_iter().next()
    })
    .await
}

/// The computer's search over the real Fairy-Stockfish, which `random` makes play its random
/// move (the weakest levels) or its best.
fn variant_search(engine: std::path::PathBuf, random: f64) -> kchess_core::engine::search::Search {
    let host = support::host();
    let mut locations = support::locations();
    locations.variant = engine;
    let ctx = EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        support::scheduler(2, false),
        locations,
    );
    kchess_core::engine::search::Search::with_random(ctx, Arc::new(move || random))
}

/// The computer plays Crazyhouse and Horde by their rules, from their own starts: its moves
/// (best or random) are legal there, and a drop is among the moves it is given.
#[tokio::test]
#[cfg_attr(
    not(kchess_fairy_stockfish),
    ignore = "needs Fairy-Stockfish: set KCHESS_VARIANT_ENGINE_PATH"
)]
async fn the_computer_plays_a_variant_by_its_rules() {
    let engine = support::engine("KCHESS_VARIANT_ENGINE_PATH");
    let zh = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
    let horde = "rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/PPPPPPPP/PPPPPPPP/PPPPPPPP w kq - 0 1";
    let games = [
        (
            "crazyhouse",
            zh,
            vec!["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"],
        ),
        ("horde", horde, vec!["d4d5"]),
    ];
    for random in [1.0, 0.0] {
        let search = variant_search(engine.clone(), random);
        for (variant, fen, moves) in &games {
            let level = if random == 0.0 { "beginner" } else { "max" };
            let reply = search
                .best_move(
                    json!(moves),
                    level,
                    "",
                    json!({ "fen": fen, "variant": variant, "movetime": 100 }),
                )
                .await
                .unwrap_or_else(|cause| panic!("{variant} {level}: {}", cause.message));
            let mut line: Vec<String> = moves.iter().map(|mv| mv.to_string()).collect();
            line.push(reply.clone());
            assert_eq!(
                kchess_domain::replay::replay_setup_count(
                    kchess_domain::variant::Variant::from_key(variant).unwrap(),
                    fen,
                    &line
                ),
                Some(line.len()),
                "{variant} {level} played {reply}"
            );
        }
        search.stop_engine(true);
    }
}

/// Without Fairy-Stockfish a variant game's move fails at once, saying which engine is missing.
#[tokio::test]
async fn a_variant_move_without_fairy_stockfish_says_so() {
    let search = variant_search(
        std::path::PathBuf::from("/nonexistent/fairy-stockfish"),
        1.0,
    );
    let failed = search
        .best_move(
            json!(["e2e4"]),
            "max",
            "",
            json!({
                "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1",
                "variant": "crazyhouse",
            }),
        )
        .await
        .expect_err("no engine");
    assert_eq!(
        failed.message,
        "Fairy-Stockfish, which plays this variant, is missing."
    );
}
