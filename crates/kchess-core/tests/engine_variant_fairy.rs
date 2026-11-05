//! Variant analysis with the real Fairy-Stockfish (`support::fairy_stockfish`): a Crazyhouse
//! position analysed with drops in its lines, and an Atomic one under Atomic's rules.

mod support;

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
async fn crazyhouse_lines_drop_pieces_from_the_pockets() {
    let Some(engine) = support::fairy_stockfish() else {
        return;
    };
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
async fn an_atomic_capture_that_explodes_the_king_is_mate() {
    let Some(engine) = support::fairy_stockfish() else {
        return;
    };
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
