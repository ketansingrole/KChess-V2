//! Regression: requests waiting for settings belong to stop, recovery and shutdown.
//! Runs on every platform, against a copy of the fake engine (`support::engine_copy`).
use crate::support;
use futures_util::poll;
use kchess_contracts::core_api::CoreApi;
use kchess_contracts::types::Assistance;
use kchess_core::engine::search;
use kchess_core::{Config, Core};
use serde_json::Value;
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

async fn prepared(dir: &Path, host: &Arc<support::Recording>) -> Core {
    let core = Core::open(Config::new(dir.to_path_buf()), host.clone()).unwrap();
    core.call("loadData", vec![]).await.unwrap();
    core.call("resumeOnline", vec![]).await.unwrap();
    assert_eq!(core.assistance(), Assistance::Allowed);
    let engine = support::engine_copy(dir, "search");
    support::save_engine_path(&core, &engine);
    core
}

/// The engine refuses on its own, not only at the method table: a call that reaches it without
/// the table's check (a game started while it waited, or a caller inside the core) is refused
/// while Lichess has not confirmed that no game is in progress, and says so.
#[tokio::test]
async fn the_engine_refuses_until_the_check_for_a_game_answers() {
    use kchess_contracts::check::Moves;
    use kchess_contracts::core_api::EngineApi;
    use kchess_contracts::types::EngineLevel;
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let core = Core::open(Config::new(dir.path().to_path_buf()), host.clone()).unwrap();
    core.call("loadData", vec![]).await.unwrap();
    assert_eq!(core.assistance(), Assistance::Unverified);
    let refused = EngineApi::best_move(&core, Moves::default(), EngineLevel::Club, None)
        .await
        .unwrap_err();
    assert_eq!(
        refused.message,
        "Engine assistance is unavailable until your Lichess game status is verified."
    );
    core.close().await;
}

async fn scored(host: &support::Recording) -> Value {
    support::wait_until(Duration::from_secs(5), "a scored update", || {
        host.named("engine:analysis").into_iter().find(|value| {
            value["done"] == json!(false)
                && value["lines"]
                    .as_array()
                    .is_some_and(|lines| !lines.is_empty())
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
    assert!(!core.analysis_running_for_tests());
    assert!(host.named("engine:analysis").is_empty());
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
    let __t = std::time::Instant::now();
    let core = prepared(dir.path(), &host).await;
    let __prepared = __t.elapsed();
    // TEMPORARY: the longest the test's runtime thread went without running a 5 ms ticker.
    let __gap = Arc::new(std::sync::Mutex::new(Duration::ZERO));
    let __ticker = tokio::spawn({
        let gap = Arc::clone(&__gap);
        async move {
            let mut last = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let now = std::time::Instant::now();
                let mut gap = gap.lock().unwrap();
                *gap = (*gap).max(now - last);
                last = now;
            }
        }
    });
    let storage = core.hold_storage_for_tests().await.unwrap();
    let start = core.call(
        "startAnalysis",
        vec![json!({"fen": START, "lines": 1, "infinite": true})],
    );
    tokio::pin!(start);
    assert!(poll!(&mut start).is_pending(), "start is awaiting settings");
    let recovery = core.call("resumeOnline", vec![]);
    tokio::pin!(recovery);
    assert!(
        poll!(&mut recovery).is_pending(),
        "recovery has reported checking and is awaiting accounts"
    );
    assert_eq!(core.assistance(), Assistance::Unverified);
    drop(storage);
    assert!(start.await.unwrap_err().aborted);
    assert!(host.named("engine:analysis").is_empty());
    assert_eq!(core.assistance(), Assistance::Unverified);
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
    assert_eq!(search.await.unwrap_err().message, search::SUPERSEDED);
    core.close().await;
}

#[tokio::test]
async fn close_drains_and_cancels_a_pending_analysis_start() {
    let dir = tempfile::tempdir().unwrap();
    let host = support::host();
    let __t = std::time::Instant::now();
    let core = prepared(dir.path(), &host).await;
    let __prepared = __t.elapsed();
    // TEMPORARY: the longest the test's runtime thread went without running a 5 ms ticker.
    let __gap = Arc::new(std::sync::Mutex::new(Duration::ZERO));
    let __ticker = tokio::spawn({
        let gap = Arc::clone(&__gap);
        async move {
            let mut last = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let now = std::time::Instant::now();
                let mut gap = gap.lock().unwrap();
                *gap = (*gap).max(now - last);
                last = now;
            }
        }
    });
    let storage = core.hold_storage_for_tests().await.unwrap();
    let start = core.call(
        "startAnalysis",
        vec![json!({"fen": START, "lines": 1, "infinite": true})],
    );
    tokio::pin!(start);
    assert!(poll!(&mut start).is_pending());
    drop(storage);
    let __closing = std::time::Instant::now();
    let timed = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(core.close(), start)
    })
    .await;
    let report = format!(
        "TEST-TIMING close_drains: prepared {:?}, close+start {:?} ({}), longest runtime stall {:?}\n",
        __prepared,
        __closing.elapsed(),
        if timed.is_ok() { "ok" } else { "timed out" },
        *__gap.lock().unwrap()
    );
    __ticker.abort();
    eprint!("{report}");
    {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("kchess-close-timing.log"))
        {
            let _ = file.write_all(report.as_bytes());
        }
    }
    let (_, result) = timed.expect("close must cancel a pending start");
    assert!(result.unwrap_err().aborted);
    assert!(host.named("engine:analysis").is_empty());
}
