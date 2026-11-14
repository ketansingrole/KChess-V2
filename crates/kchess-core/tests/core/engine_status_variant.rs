//! `engineStatus` names the engine that plays a variant: Stockfish for standard chess and
//! Chess960, the bundled Fairy-Stockfish for the others. The computer page checks it before a
//! variant game, so a missing Fairy-Stockfish shows "not found" rather than a failed move.

use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_status_reports_the_engine_that_plays_the_variant() {
    let dir = tempfile::tempdir().expect("profile");
    let fairy = dir.path().join("no-fairy-stockfish");
    let mut config = Config::new(dir.path().to_path_buf());
    config.variant_engine_path = Some(fairy.clone());
    let core = Core::open(config, TestHost::quiet()).expect("opens");

    // Fairy-Stockfish plays Crazyhouse: missing here, so not ready.
    let status = core
        .call("engineStatus", vec![json!("crazyhouse")])
        .await
        .expect("a status");
    assert_eq!(status["ready"], false);
    assert_eq!(status["path"], json!(fairy));
    crate::support::wire::assert_wire_steady(
        "engineStatus.variant-missing",
        &status,
        &["path", "managed/path"],
    );

    // Stockfish plays standard chess and Chess960, with or without the argument.
    for args in [vec![], vec![json!("standard")], vec![json!("chess960")]] {
        let status = core.call("engineStatus", args).await.expect("a status");
        assert_ne!(status["path"], json!(fairy));
    }

    let refused = core.call("engineStatus", vec![json!("bughouse")]).await;
    assert!(refused.is_err(), "an unknown variant is refused");
    core.close().await;
}
