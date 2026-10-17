//! A game starting mid-lookup still blocks: the explorer answer must not reach a live game.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]
#[path = "support/lichess_http.rs"]
mod fixtures;

use fixtures::{Recording, Reply, serve};
use futures_util::poll;
use kchess_core::{Config, Core};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// `positionLookup` checks assistance before its fetch and again after it resolves, so a game
/// starting while the lookup is in flight discards the answer instead of delivering explorer
/// data into the live game (as `bestMove`/`startAnalysis` already recheck after settings load).
#[tokio::test]
async fn a_game_starting_mid_lookup_blocks_the_explorer_answer() {
    let (gate_open, gate) = tokio::sync::watch::channel(false);
    let fixture = serve(move |_| {
        Reply::Gated(
            gate.clone(),
            Box::new(Reply::Json(
                200,
                json!({ "category": "win", "dtz": 1, "moves": [] }).to_string(),
            )),
        )
    })
    .await;
    // SAFETY: this test binary has one test; the fixture origin is set before opening the core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", &fixture.base);
    }
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let core = Core::open(Config::new(dir.path().to_path_buf()), host).unwrap();
    // Startup leaves assistance blocked until recovery verifies no game is in progress.
    core.call("resumeOnline", vec![]).await.unwrap();
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(false)
    );
    let lookup = core.call(
        "positionLookup",
        vec![json!("tablebase"), json!("6k1/8/8/8/8/8/8/R5K1 w - - 0 1")],
    );
    tokio::pin!(lookup);
    tokio::select! {
        biased;
        _ = &mut lookup => panic!("the gated fixture holds the lookup"),
        _ = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while fixture.requests().is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("the lookup reaches the fixture");
        } => {}
    }
    // A game starts while the lookup is in flight: recovery re-blocks assistance.
    let recovery = core.call("resumeOnline", vec![]);
    tokio::pin!(recovery);
    assert!(
        poll!(&mut recovery).is_pending(),
        "recovery is verifying and assistance is blocked mid-lookup"
    );
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(true)
    );
    gate_open.send(true).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .expect("the gated lookup resolves")
        .unwrap_err();
    assert!(
        error.message.contains("live Lichess game"),
        "the mid-lookup game blocks the answer: {error:?}"
    );
    recovery.await.unwrap();
    core.close().await;
}
