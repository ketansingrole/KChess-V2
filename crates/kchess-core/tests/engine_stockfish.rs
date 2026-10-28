//! A native Stockfish through the Rust controller (`support::stockfish`).

mod support;

use std::time::Duration;

use kchess_core::engine::uci::UciController;
use tokio::process::Command;

#[tokio::test]
async fn bundled_stockfish_answers_a_bounded_search() {
    let Some(engine) = support::stockfish() else {
        return;
    };

    let host = support::host();
    let hub = support::hub(&host);
    let command = Command::new(&engine);
    let engine = UciController::start(&hub, command, Duration::from_secs(30))
        .expect("spawn the bundled Stockfish");
    engine.ready().await.expect("Stockfish handshake");
    engine
        .ensure_options(&[("Threads", "1"), ("Hash", "16")])
        .await
        .expect("options");
    engine.write("position startpos").expect("position");
    let best = engine
        .search(
            "go movetime 100",
            |_line: &str| {},
            Some(Duration::from_secs(20)),
            None,
        )
        .await
        .expect("bounded search");
    assert!(best.starts_with("bestmove "), "unexpected reply: {best}");
    engine.close();
    tokio::time::timeout(Duration::from_secs(10), engine.closed())
        .await
        .expect("Stockfish exits after close");
}
