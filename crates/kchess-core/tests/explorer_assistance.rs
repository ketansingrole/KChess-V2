//! A game starting mid-lookup still blocks: the explorer answer must not reach a live game.
#[path = "support/lichess_http.rs"]
mod fixtures;
#[path = "support/signed_in.rs"]
mod signed_in;

use fixtures::{Reply, serve};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{sync::Arc, time::Duration};

const PLAYING: &str = "/api/account/playing";

/// `positionLookup` checks assistance before its fetch and again after it resolves, so a game
/// starting while the lookup is in flight discards the answer instead of delivering explorer
/// data into the live game (as `bestMove`/`startAnalysis` already recheck after settings load).
///
/// Recovery stays unverified, and assistance blocked, only while it is checking Lichess, so the
/// test holds that check open at the fixture. It used to rely on the first poll of
/// `resumeOnline` being pending; with no account its only wait is one database read, and a
/// preempted test thread (a loaded coverage run) let it finish first, verifying no game before
/// the lookup answered (CI runs 42173832416 and 42180154667).
#[tokio::test]
async fn a_game_starting_mid_lookup_blocks_the_explorer_answer() {
    let (lookup_open, lookup_gate) = tokio::sync::watch::channel(false);
    let (recovery_open, recovery_gate) = tokio::sync::watch::channel(false);
    let checks = Arc::new(AtomicUsize::new(0));
    let fixture = serve(move |seen| {
        if seen.path() == PLAYING {
            let reply = Box::new(Reply::Json(200, json!({ "nowPlaying": [] }).to_string()));
            // Startup recovery answers at once; the mid-lookup one waits for its gate.
            return match checks.fetch_add(1, Ordering::SeqCst) {
                0 => *reply,
                _ => Reply::Gated(recovery_gate.clone(), reply),
            };
        }
        Reply::Gated(
            lookup_gate.clone(),
            Box::new(Reply::Json(
                200,
                json!({ "category": "win", "dtz": 1, "moves": [] }).to_string(),
            )),
        )
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    // Startup leaves assistance blocked until recovery verifies no game is in progress.
    core.call("resumeOnline", vec![]).await.unwrap();
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(false)
    );
    let lookup = tokio::spawn({
        let core = Arc::clone(core);
        async move {
            core.call(
                "positionLookup",
                vec![json!("tablebase"), json!("6k1/8/8/8/8/8/8/R5K1 w - - 0 1")],
            )
            .await
        }
    });
    fixture
        .wait_for("the lookup", |requests| {
            requests.iter().any(|seen| seen.path() != PLAYING)
        })
        .await;
    assert!(!lookup.is_finished(), "the gated fixture holds the lookup");
    // A game starts while the lookup is in flight: recovery re-blocks assistance and checks
    // Lichess, which the fixture holds until the lookup has answered.
    let recovery = tokio::spawn({
        let core = Arc::clone(core);
        async move { core.call("resumeOnline", vec![]).await }
    });
    fixture.requested(PLAYING, 2).await;
    assert!(!recovery.is_finished(), "the gated fixture holds recovery");
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(true),
        "recovery is verifying and assistance is blocked mid-lookup"
    );
    lookup_open.send(true).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(30), lookup)
        .await
        .expect("the gated lookup resolves")
        .unwrap()
        .unwrap_err();
    assert!(
        error.message.contains("live Lichess game"),
        "the mid-lookup game blocks the answer: {error:?}"
    );
    recovery_open.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(30), recovery)
        .await
        .expect("the gated recovery resolves")
        .unwrap()
        .unwrap();
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(false),
        "recovery that finds no game unblocks assistance again"
    );
    core.close().await;
}
