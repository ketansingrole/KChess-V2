//! Regression: requests waiting for settings belong to stop, recovery and shutdown.
//! Runs on every platform, against a copy of the fake engine (`support::engine_copy`).
mod support;
use futures_util::poll;
use kchess_core::{Config, Core};
use serde_json::Value;
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

async fn prepared(dir: &Path, host: &Arc<support::Recording>) -> Core {
    let core = Core::open(Config::new(dir.to_path_buf()), host.clone()).unwrap();
    core.call("loadData", vec![]).await.unwrap();
    core.call("online.resume", vec![]).await.unwrap();
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(false)
    );
    let engine = support::engine_copy(dir, "search");
    let mut settings = core.call_sync("store.games.getSettings", vec![]).unwrap();
    settings["enginePath"] = json!(engine);
    core.call_sync("store.games.saveSettings", vec![settings])
        .unwrap();
    core
}

async fn scored(host: &support::Recording) -> Value {
    support::wait_until(Duration::from_secs(5), "a scored update", || {
        host.events
            .lock()
            .unwrap()
            .iter()
            .find_map(|(name, value)| {
                (name == "engine:analysis"
                    && value["done"] == json!(false)
                    && value["lines"]
                        .as_array()
                        .is_some_and(|lines| !lines.is_empty()))
                .then(|| value.clone())
            })
    })
    .await
}

#[tokio::test]
async fn stopping_cancels_analysis_waiting_for_settings() {
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let core = prepared(dir.path(), &host).await;
    // Held storage keeps the settings read queued: without it, a loaded machine can finish
    // the read inside the first poll (seen on Linux) and the start never waits.
    let storage = core.hold_storage_for_tests().await.unwrap();
    let start = core.call(
        "startAnalysis",
        vec![json!({"fen": START, "lines": 1, "infinite": true})],
    );
    tokio::pin!(start);
    assert!(poll!(&mut start).is_pending(), "start is awaiting settings");
    core.call("stopAnalysis", vec![json!(true)]).await.unwrap();
    drop(storage);
    assert!(start.await.unwrap_err().aborted);
    assert_eq!(
        core.call_sync("analysis.running", vec![]).unwrap(),
        json!(false)
    );
    assert!(
        host.events
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "engine:analysis")
    );
    // A later explicit start is still allowed and produces usable output.
    let id = core
        .call(
            "startAnalysis",
            vec![json!({"fen": START, "lines": 1, "infinite": true})],
        )
        .await
        .unwrap();
    assert_eq!(scored(&host).await["id"], id);
    core.close().await;
}

#[tokio::test]
async fn recovery_cancels_analysis_waiting_for_settings() {
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let core = prepared(dir.path(), &host).await;
    let storage = core.hold_storage_for_tests().await.unwrap();
    let start = core.call(
        "startAnalysis",
        vec![json!({"fen": START, "lines": 1, "infinite": true})],
    );
    tokio::pin!(start);
    assert!(poll!(&mut start).is_pending(), "start is awaiting settings");
    let recovery = core.call("online.resume", vec![]);
    tokio::pin!(recovery);
    assert!(
        poll!(&mut recovery).is_pending(),
        "recovery has reported checking and is awaiting accounts"
    );
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(true)
    );
    drop(storage);
    assert!(start.await.unwrap_err().aborted);
    assert!(
        host.events
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "engine:analysis")
    );
    assert_eq!(
        core.call_sync("online.assistanceBlocked", vec![]).unwrap(),
        json!(true)
    );
    recovery.await.unwrap();
    core.close().await;
}

#[tokio::test]
async fn stopping_cancels_computer_search_waiting_for_settings() {
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let core = prepared(dir.path(), &host).await;
    let storage = core.hold_storage_for_tests().await.unwrap();
    let search = core.call(
        "bestMove",
        vec![json!([]), json!("max"), json!({"movetime": 50})],
    );
    tokio::pin!(search);
    assert!(
        poll!(&mut search).is_pending(),
        "search is awaiting settings"
    );
    core.call("stopEngine", vec![]).await.unwrap();
    drop(storage);
    assert_eq!(
        search.await.unwrap_err().message,
        "Engine search superseded."
    );
    core.close().await;
}

#[tokio::test]
async fn close_drains_and_cancels_a_pending_analysis_start() {
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let core = prepared(dir.path(), &host).await;
    let storage = core.hold_storage_for_tests().await.unwrap();
    let start = core.call(
        "startAnalysis",
        vec![json!({"fen": START, "lines": 1, "infinite": true})],
    );
    tokio::pin!(start);
    assert!(poll!(&mut start).is_pending());
    drop(storage);
    let (_, result) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(core.close(), start)
    })
    .await
    .expect("close must cancel a pending start");
    assert!(result.unwrap_err().aborted);
    assert!(
        host.events
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "engine:analysis")
    );
}
