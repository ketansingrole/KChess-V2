//! Closing the core (`Core::close`, the facade's close sequence): an engine search, a Lichess
//! request and a puzzle download that are running when the core closes end, `close` returns, and
//! later calls fail with `The KChess core is closed.`. The Lichess case is `core_close_lichess.rs`.

use crate::support;

use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kchess_core::engine::search;
use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};
use serde_json::json;

/// What a closed core answers.
const CLOSED: &str = "The KChess core is closed.";
/// How long closing may take before the test calls it stuck.
const LIMIT: Duration = Duration::from_secs(20);

fn open(dir: &Path, host: &Arc<TestHost>) -> Arc<Core> {
    Arc::new(Core::open(Config::new(dir.to_path_buf()), host.clone()).expect("the profile opens"))
}

#[tokio::test]
async fn closing_ends_an_engine_search_in_flight() {
    let dir = tempfile::tempdir().expect("profile");
    let host = TestHost::new();
    let core = open(dir.path(), &host);
    let engine = support::engine_copy(dir.path(), "search");
    support::save_engine_path(&core, &engine);
    // Assistance waits until the core has confirmed there is no live game.
    core.call("resumeOnline", vec![]).await.expect("resume");

    let search = {
        let core = Arc::clone(&core);
        tokio::spawn(async move {
            core.call(
                "bestMove",
                vec![
                    json!(["e2e4"]),
                    // A level that never plays a random move instead of searching.
                    json!("max"),
                    // The longest think time: the search is still running when the core closes.
                    json!({ "movetime": 5_000 }),
                ],
            )
            .await
        })
    };
    // The engine is searching when the core closes.
    support::until("the engine's search", || {
        support::received(&support::copy_log(&engine))
            .iter()
            .any(|line| line.starts_with("go movetime"))
    })
    .await;

    tokio::time::timeout(LIMIT, core.close())
        .await
        .expect("closing returns while the search runs");
    let answer = tokio::time::timeout(LIMIT, search)
        .await
        .expect("the search ends when the core closes")
        .expect("the search task");
    let error = answer.expect_err("the search was stopped, not answered");
    assert_eq!(error.message, search::SUPERSEDED);

    let later = core
        .call("loadData", vec![])
        .await
        .expect_err("a closed core refuses");
    assert_eq!(later.message, CLOSED);
}

#[tokio::test]
async fn closing_ends_a_puzzle_download_in_flight() {
    let silent = TcpListener::bind("127.0.0.1:0").expect("silent origin");
    let url = format!(
        "http://127.0.0.1:{}/puzzles.csv.zst",
        silent.local_addr().expect("address").port()
    );
    let dir = tempfile::tempdir().expect("profile");
    let host = TestHost::new();
    let core = open(dir.path(), &host);

    let install = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.install_puzzles_for_tests(&url).await })
    };
    let _download = support::accepted(&silent).await;

    tokio::time::timeout(LIMIT, core.close())
        .await
        .expect("closing returns while the download runs");
    let answer = tokio::time::timeout(LIMIT, install)
        .await
        .expect("the download ends when the core closes")
        .expect("the install task");
    // A cancelled download ends without a database: as an error, or as an uninstalled status.
    assert!(
        answer.is_err()
            || answer
                .as_ref()
                .is_ok_and(|status| status["installed"] == json!(false)),
        "the download ended without installing: {answer:?}"
    );

    let later = core
        .call("puzzleDbStatus", vec![])
        .await
        .expect_err("a closed core refuses");
    assert_eq!(later.message, CLOSED);
    drop(silent);
}

#[tokio::test]
async fn closing_twice_is_harmless_and_frees_the_profile() {
    let dir = tempfile::tempdir().expect("profile");
    let host = TestHost::new();
    let core = open(dir.path(), &host);
    // A second core on the same profile is refused while this one runs.
    let refused = Core::open(Config::new(dir.path().to_path_buf()), host.clone());
    assert!(refused.is_err(), "a live profile is not shared");
    core.close().await;
    core.close().await;
    // Once closed, the profile opens again.
    let reopened = Core::open(Config::new(dir.path().to_path_buf()), host.clone());
    assert!(reopened.is_ok(), "a closed profile reopens");
}

#[tokio::test]
async fn a_closed_core_refuses_later_calls() {
    // A late call must not start the battery and review services again, or reopen the database.
    let dir = tempfile::tempdir().expect("profile");
    let host = TestHost::new();
    let core = open(dir.path(), &host);
    core.call("loadData", vec![]).await.expect("opened");
    core.close().await;
    let db = dir.path().join("kchess.db");
    std::fs::remove_file(&db).expect("database removed");
    let later = core
        .call("loadData", vec![])
        .await
        .expect_err("a closed core refuses");
    assert_eq!(later.message, CLOSED);
    assert!(!core.analysis_running_for_tests());
    assert!(!db.exists(), "the database was not opened again");
}
