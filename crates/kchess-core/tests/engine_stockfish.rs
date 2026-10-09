//! The bundled Stockfish (`node_modules/stockfish`, the same script the TypeScript core runs)
//! through the Rust controller. Skipped with a message when node or the script is missing.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use kchess_core::engine::uci::UciController;
use tokio::process::Command;

#[tokio::test]
async fn bundled_stockfish_answers_a_bounded_search() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("node_modules/stockfish/bin/stockfish-19-lite.js");
    if !script.exists() {
        eprintln!(
            "skipping: {} is missing (run pnpm install at the repository root)",
            script.display()
        );
        return;
    }
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: node is not on PATH");
        return;
    }

    let host = support::host();
    let hub = support::hub(&host);
    let mut command = Command::new("node");
    command.arg(&script);
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
