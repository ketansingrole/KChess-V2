//! One real Stockfish move through the search service: the bundled engine
//! (`node_modules/stockfish`, the script the TypeScript core runs under Node). Skipped with a
//! message when node or the script is missing.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use kchess_core::engine::EngineContext;
use kchess_core::engine::managed::Target;
use kchess_core::engine::search::Search;
use kchess_core::engine::status::EngineLocations;
use serde_json::json;

#[tokio::test]
async fn bundled_stockfish_answers_a_move_for_the_position() {
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
    let locations = EngineLocations {
        managed_dir: std::env::temp_dir().join("kchess-no-managed-engine"),
        target: Target::current(),
        bundled_script: script,
        node_path: PathBuf::from("node"),
        node_env: BTreeMap::new(),
    };
    let ctx = EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        support::scheduler(2, false),
        locations,
    );
    let search = Search::new(ctx);
    // After 1. e4 it is Black's move: Stockfish answers with a Black move, never White's e2e4.
    let mv = search
        .best_move(json!(["e2e4"]), "max", "", json!({ "movetime": 200 }))
        .await
        .expect("Stockfish answers");
    assert!(mv.len() == 4 || mv.len() == 5, "a UCI move, got {mv:?}");
    assert!(mv.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert_ne!(mv, "e2e4");
    assert!(search.computer_playing(60_000));
    search.stop_engine(true);
}
