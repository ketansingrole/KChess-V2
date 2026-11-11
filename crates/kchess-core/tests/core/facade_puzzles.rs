//! The puzzle database through the core's methods.

use kchess_core::{Config, Core};
use serde_json::json;

use crate::support;

/// 2026-11-11: every failed puzzle operation was logged twice at Warn, once by the puzzle service
/// and again by the facade. The service's line, which names the operation, is the only one.
#[tokio::test]
async fn a_failed_puzzle_operation_is_logged_once() {
    let dir = tempfile::tempdir().expect("profile");
    let host = support::host();
    let core = Core::open(Config::new(dir.path().to_path_buf()), host.clone()).expect("opens");
    let refused = core
        .call("localPuzzles", vec![json!({ "count": 5 })])
        .await
        .expect_err("no puzzle database yet");
    assert!(
        refused
            .message
            .starts_with("Download the puzzle database first")
    );
    let warned: Vec<String> = host
        .lines
        .lock()
        .expect("log lock")
        .iter()
        .filter(|line| line.starts_with("warn [puzzles]"))
        .cloned()
        .collect();
    assert_eq!(
        warned,
        [format!(
            "warn [puzzles] Puzzle operation query failed {}",
            refused.message
        )]
    );
    core.close().await;
}
