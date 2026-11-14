//! The facade's engine calls: settings take only an engine the person chose and stop what ran on
//! the one they replace, `stopAnalysis` closes the engine only when asked, `deleteEngine` removes
//! the downloaded one, and reviews are cancelled, reported and found by their variant.

use crate::support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kchess_contracts::types::StoredReview;
use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};
use serde_json::{Value, json};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const WAIT: Duration = Duration::from_secs(30);

fn open(host: &Arc<TestHost>, config: Config) -> Core {
    Core::open(config, host.clone()).expect("the profile opens")
}

/// Starts an infinite analysis of the start position and waits for its first line.
async fn analysing(core: &Core, host: &TestHost) -> Value {
    let id = core
        .call(
            "startAnalysis",
            vec![json!({ "fen": START, "lines": 1, "infinite": true })],
        )
        .await
        .expect("analysis started");
    support::wait_until(WAIT, "a live analysis update", || {
        host.named("engine:analysis").into_iter().find(|update| {
            update["id"] == id
                && update["done"] == json!(false)
                && !update["lines"].as_array().unwrap().is_empty()
        })
    })
    .await;
    id
}

/// Waits for analysis `id`'s final update, and returns why it ended.
async fn ended(host: &TestHost, id: &Value) -> Value {
    let last = support::wait_until(WAIT, "the final analysis update", || {
        host.named("engine:analysis")
            .into_iter()
            .find(|update| update["id"] == *id && update["done"] == json!(true))
    })
    .await;
    last["reason"].clone()
}

/// How many times the engine at `engine` has been started (`uci` is its first command).
fn starts(engine: &str) -> usize {
    support::received(&support::copy_log(engine))
        .iter()
        .filter(|line| *line == "uci")
        .count()
}

fn with_engine_path(settings: &Value, path: &str) -> Value {
    let mut settings = settings.clone();
    settings["enginePath"] = json!(path);
    settings
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_take_a_chosen_engine_and_a_new_one_stops_the_analysis() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new();
    let core = open(&host, Config::new(dir.path().to_path_buf()));
    let first = support::engine_copy(dir.path(), "search");
    let second = dir
        .path()
        .join(format!("engine-other{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&first, &second).unwrap();
    let second = second.display().to_string();
    let settings = core.call("settings", vec![]).await.unwrap();

    // A path nobody chose in the file picker is refused.
    let refused = core
        .call("saveSettings", vec![with_engine_path(&settings, &first)])
        .await
        .expect_err("an engine nobody chose");
    assert_eq!(
        refused.message,
        "Choose the Stockfish executable with the file picker."
    );
    core.call("trustEnginePath", vec![json!(first)])
        .await
        .unwrap();
    let saved = core
        .call("saveSettings", vec![with_engine_path(&settings, &first)])
        .await
        .expect("a chosen engine");
    assert_eq!(saved["enginePath"], json!(first));
    core.call("resumeOnline", vec![]).await.expect("no game");

    // Another engine stops the analysis of the one it replaces.
    let id = analysing(&core, &host).await;
    core.call("trustEnginePath", vec![json!(second)])
        .await
        .unwrap();
    let saved = core
        .call("saveSettings", vec![with_engine_path(&saved, &second)])
        .await
        .expect("another chosen engine");
    assert_eq!(ended(&host, &id).await, json!("interrupted"));
    // Saving other settings keeps the saved engine, though nobody picked it again.
    let mut cloud = saved.clone();
    cloud["cloudEval"] = json!(true);
    core.call("saveSettings", vec![cloud])
        .await
        .expect("the saved engine stays");
    core.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_an_analysis_closes_its_engine_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new();
    let core = open(&host, Config::new(dir.path().to_path_buf()));
    let engine = support::engine_copy(dir.path(), "search");
    support::save_engine_path(&core, &engine);
    core.call("resumeOnline", vec![]).await.expect("no game");

    let id = analysing(&core, &host).await;
    core.call("stopAnalysis", vec![json!(false)]).await.unwrap();
    assert_eq!(ended(&host, &id).await, json!("interrupted"));
    let id = analysing(&core, &host).await;
    assert_eq!(starts(&engine), 1, "the engine was kept");
    core.call("stopAnalysis", vec![json!(true)]).await.unwrap();
    assert_eq!(ended(&host, &id).await, json!("interrupted"));
    analysing(&core, &host).await;
    // Waited for rather than read once: on a Windows runner the count read 1 here once. A new
    // process must still start; failing, the message shows what each engine received.
    let started = std::time::Instant::now();
    while starts(&engine) < 2 {
        assert!(
            started.elapsed() < WAIT,
            "the engine was closed, so another starts: {:?}",
            support::received(&support::copy_log(&engine))
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(starts(&engine), 2, "one engine more, not several");
    core.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_the_engine_removes_the_downloaded_one() {
    let dir = tempfile::tempdir().unwrap();
    let managed = dir.path().join("managed");
    std::fs::create_dir_all(&managed).unwrap();
    std::fs::write(managed.join("stockfish"), "an engine").unwrap();
    let host = TestHost::new();
    let config = Config {
        managed_engine_dir: Some(managed.clone()),
        ..Config::new(dir.path().to_path_buf())
    };
    let core = open(&host, config);
    core.call("deleteEngine", vec![]).await.expect("deleted");
    assert!(!managed.exists());
    core.close().await;
}

/// A stored Crazyhouse review, as `review.crazyhouse` recorded it.
fn crazyhouse_review() -> StoredReview {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wire/review.crazyhouse.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviews_are_found_by_variant_reported_and_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new();
    let core = open(&host, Config::new(dir.path().to_path_buf()));
    core.call("resumeOnline", vec![]).await.expect("no game");

    // A Crazyhouse game's review is found by its variant.
    let review = crazyhouse_review();
    let (fen, moves) = (review.fen.clone(), review.moves.clone());
    core.database_for_tests()
        .write(|c| kchess_core::store::reviews::write_review(c.host, c.db, review))
        .unwrap();
    let found = core
        .call(
            "reviewGet",
            vec![json!(fen), json!(moves), json!("crazyhouse")],
        )
        .await
        .unwrap();
    assert_eq!(found["key"], json!("618d40135675018e"));

    // Cancelling a review reports the reviews' state.
    core.call("reviewCancel", vec![json!("0123456789abcdef")])
        .await
        .unwrap();
    assert_eq!(host.named("review:status"), vec![json!({ "waiting": 0 })]);

    // With no engine, a requested review fails, and the status says so.
    core.call(
        "reviewRequest",
        vec![json!({ "fen": START, "moves": ["e2e4", "e7e5"] })],
    )
    .await
    .unwrap();
    support::until("a failed review status", || {
        host.named("review:status")
            .iter()
            .any(|status| status.get("failed").is_some())
    })
    .await;
    let status = core.call("reviewStatus", vec![]).await.unwrap();
    assert_eq!(
        status["failed"]["message"],
        json!("Stockfish could not be found. Choose an engine in Settings.")
    );
    core.close().await;
}
