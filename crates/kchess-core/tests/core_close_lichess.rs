//! Closing the core while a Lichess request waits on an origin that never answers: the request
//! ends, `close` returns, and later calls fail with `The KChess core is closed.`.
//!
//! It sets the test Lichess origin for the whole process (`kchess_core::env`), so it is the only
//! test in this binary: no other test can inherit the origin.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

/// What a closed core answers (`createKChessCore`'s closed check).
const CLOSED: &str = "The KChess core is closed.";
/// How long closing may take before the test calls it stuck.
const LIMIT: Duration = Duration::from_secs(20);

/// Answers no host request (none of these calls needs one).
struct Quiet;

impl Host for Quiet {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}
    fn emit(&self, _event: &str, _payload: Value) {}
}

#[tokio::test]
async fn closing_ends_a_lichess_request_in_flight() {
    // The test origin serves Lichess from a socket that never answers.
    let silent = TcpListener::bind("127.0.0.1:0").expect("silent origin");
    let origin = format!(
        "http://127.0.0.1:{}",
        silent.local_addr().expect("address").port()
    );
    kchess_core::env::set("KCHESS_STORE_DEBUG", "1");
    kchess_core::env::set("KCHESS_TEST_LICHESS_BASE", &origin);
    let dir = tempfile::tempdir().expect("profile");
    let host: Arc<dyn Host> = Arc::new(Quiet);
    let core = Arc::new(
        Core::open(Config::new(dir.path().to_path_buf()), host).expect("the profile opens"),
    );

    let request = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("profile", vec![json!("someone")]).await })
    };
    // Waits until the spawned call has started its work.
    tokio::time::sleep(Duration::from_millis(300)).await;

    tokio::time::timeout(LIMIT, core.close())
        .await
        .expect("closing returns while the request waits");
    let answer = tokio::time::timeout(LIMIT, request)
        .await
        .expect("the request ends when the core closes")
        .expect("the request task");
    assert!(answer.is_err(), "the request was cancelled, not answered");

    let later = core
        .call("profile", vec![json!("someone")])
        .await
        .expect_err("a closed core refuses");
    assert_eq!(later.message, CLOSED);
    drop(silent);
}
