//! The core's wiring of the engine services (`crates/kchess-core/src/core.rs`): driven through
//! `Core::call` with a recording host and scripted engines. Covers the
//! battery reading (never waiting on the host), the managed engine handoff, the exact event
//! payloads, the busy state of automatic reviews, and closing the core.

use crate::support;

use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kchess_core::engine::scheduler::default_budget;
use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};
use serde_json::{Value, json};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const WAIT: Duration = Duration::from_secs(20);

fn profile() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary profile")
}

fn core(host: &Arc<TestHost>, dir: &Path) -> Core {
    Core::open(Config::new(dir.to_path_buf()), host.clone()).expect("the profile opens")
}

/// `script` as the configured engine, with no live game blocking it.
async fn with_engine(core: &Core, script: &str) {
    support::save_engine_path(core, script);
    core.call("resumeOnline", vec![])
        .await
        .expect("no game to resume");
}

fn expected_threads_on_battery() -> usize {
    let cpus = std::thread::available_parallelism().map_or(1, |count| count.get());
    default_budget(cpus).min(2)
}

#[tokio::test]
async fn a_lease_never_waits_on_an_unanswered_battery_request() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    let script = support::engine_copy(dir.path(), "search");
    with_engine(&core, &script).await;
    // The host never answers `onBattery`: the search must still start and finish.
    let answer = tokio::time::timeout(
        WAIT,
        core.call(
            "bestMove",
            vec![json!(["e2e4"]), json!("max"), json!({ "movetime": 100 })],
        ),
    )
    .await
    .expect("the search does not wait for the battery")
    .expect("a move");
    assert!(answer.is_string(), "{answer}");
    assert!(
        !host.requests("onBattery").is_empty(),
        "the battery was asked"
    );
    core.close().await;
}

#[tokio::test]
async fn the_hosts_battery_reading_reaches_the_thread_budget() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    // The first call starts the battery refresh; the reading is still the default (off battery).
    core.call("loadData", vec![]).await.expect("loadData");
    let off = core.engine_threads_for_tests();
    let request = support::wait_until(WAIT, "the battery request", || {
        host.requests("onBattery").pop()
    })
    .await;
    core.call(
        "host.reply",
        vec![request["id"].clone(), json!({ "value": true })],
    )
    .await
    .expect("reply");
    let on = support::wait_until(WAIT, "the battery reading", || {
        let threads = core.engine_threads_for_tests();
        (threads == expected_threads_on_battery()).then_some(threads)
    })
    .await;
    assert_eq!(on, expected_threads_on_battery());
    assert!(
        on <= off,
        "battery power never grants more threads ({on} > {off})"
    );
    core.close().await;
}

#[tokio::test]
async fn managed_replacement_stops_running_engines_before_it_publishes() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    let script = support::engine_copy(dir.path(), "search");
    with_engine(&core, &script).await;
    let id = core
        .call(
            "startAnalysis",
            vec![json!({ "fen": START, "lines": 1, "infinite": true })],
        )
        .await
        .expect("analysis started");
    support::wait_until(WAIT, "the analysis to run", || {
        core.analysis_running_for_tests().then_some(())
    })
    .await;
    // The engine dir the replacement removes; the owners must be stopped before it goes.
    let engine_dir = dir.path().join("managed");
    std::fs::create_dir_all(&engine_dir).expect("engine dir");
    std::fs::write(engine_dir.join("stockfish"), b"binary").expect("engine file");
    core.managed_engine_for_tests(false, 1, Some(engine_dir.clone()), None)
        .await
        .expect("delete");
    assert!(!engine_dir.exists(), "the executable was replaced");
    assert!(!core.analysis_running_for_tests());
    let finals: Vec<Value> = host
        .named("engine:analysis")
        .into_iter()
        .filter(|update| update["done"] == json!(true))
        .collect();
    assert_eq!(finals.len(), 1, "the analysis ended once");
    assert_eq!(finals[0]["id"], id, "the ended analysis is the one started");
    assert_eq!(finals[0]["reason"], json!("interrupted"));
    core.close().await;
}

#[tokio::test]
async fn analysis_and_review_events_carry_the_contract_payloads() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    let script = support::engine_copy(dir.path(), "search");
    with_engine(&core, &script).await;
    let id = core
        .call(
            "startAnalysis",
            vec![json!({ "fen": START, "lines": 1, "infinite": true })],
        )
        .await
        .expect("analysis started");
    let live = support::wait_until(WAIT, "a live analysis update", || {
        host.named("engine:analysis").into_iter().find(|update| {
            update["done"] == json!(false) && !update["lines"].as_array().unwrap().is_empty()
        })
    })
    .await;
    assert_eq!(live["id"], id);
    assert_eq!(live["fen"], json!(START));
    assert!(live["depth"].is_number(), "{live}");
    assert!(live["context"].is_string(), "{live}");
    assert!(live["engine"].is_string(), "{live}");
    let line = &live["lines"][0];
    assert_eq!(line["rank"], json!(1));
    assert!(line["pv"].is_array(), "{line}");
    core.call("stopAnalysis", vec![json!(false)])
        .await
        .expect("stop");
    let last = support::wait_until(WAIT, "the final analysis update", || {
        host.named("engine:analysis")
            .into_iter()
            .find(|update| update["done"] == json!(true))
    })
    .await;
    assert_eq!(last["reason"], json!("interrupted"));
    assert_eq!(last["id"], id);

    // Review: the default engine is missing, so the requested review fails with a status.
    support::save_engine_path(&core, "");
    core.call(
        "reviewRequest",
        vec![json!({ "fen": START, "moves": ["e2e4", "e7e5"] })],
    )
    .await
    .expect("request");
    let failed = support::wait_until(WAIT, "a failed review status", || {
        host.named("review:status")
            .into_iter()
            .find(|status| status.get("failed").is_some())
    })
    .await;
    assert!(failed["waiting"].is_number(), "{failed}");
    assert_eq!(
        failed["failed"]["message"],
        json!("Stockfish could not be found. Choose an engine in Settings.")
    );
    assert!(failed["failed"]["key"].is_string(), "{failed}");
    support::wire::assert_wire("review-status.failed", &failed);
    core.close().await;
}

/// A frontend reads events as their contract types: each recorded payload of an event's shape
/// is read as its event and crosses again with the same bytes.
#[test]
fn typed_events_cross_as_recorded() {
    use kchess_contracts::CoreEvent;
    for (name, recording) in [
        ("engine:analysis", "engine-analysis.completed"),
        ("review:update", "review-update.complete"),
        ("review:status", "review-status.failed"),
        ("challenges:update", "challenges.listed"),
        ("settings:saved", "settings"),
        ("settings:saved", "saveSettings"),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wire")
            .join(format!("{recording}.json"));
        let recorded = std::fs::read_to_string(&path).expect("the recording");
        let payload: Value = serde_json::from_str(&recorded).expect("JSON");
        let event = CoreEvent::from_wire(name, payload).expect("the event's shape");
        support::wire::assert_wire(recording, &event.into_payload().unwrap());
    }
}

#[tokio::test]
async fn a_review_update_carries_the_stored_review_and_its_summary() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    let script = support::engine_copy(dir.path(), "review");
    // Point the review at the scripted engine through the settings the store keeps.
    with_engine(&core, &script).await;
    core.call(
        "reviewRequest",
        vec![json!({ "fen": START, "moves": ["e2e4", "e7e5"] })],
    )
    .await
    .expect("request");
    let update = support::wait_until(WAIT, "a review update", || {
        host.named("review:update").into_iter().next()
    })
    .await;
    assert_eq!(update["review"]["fen"], json!(START));
    assert_eq!(update["review"]["moves"], json!(["e2e4", "e7e5"]));
    assert_eq!(update["review"]["source"], json!("local"));
    assert!(update["review"]["evals"].is_array(), "{update}");
    assert!(update["review"]["key"].is_string(), "{update}");
    assert!(update["summary"].is_object(), "{update}");
    core.close().await;
}

#[tokio::test]
async fn online_play_pauses_automatic_reviews() {
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    core.call("loadData", vec![]).await.expect("loadData");
    core.set_engines_busy_for_tests(true);
    core.reviews_changed_for_tests();
    let status = support::wait_until(WAIT, "a paused status", || {
        host.named("review:status")
            .into_iter()
            .find(|status| status["paused"] == json!("online"))
    })
    .await;
    assert_eq!(status["paused"], json!("online"));
    core.set_engines_busy_for_tests(false);
    core.close().await;
}

#[tokio::test]
async fn closing_the_core_cancels_an_install_and_stops_engines() {
    let dir = profile();
    let host = TestHost::new();
    let core = Arc::new(core(&host, dir.path()));
    let script = support::engine_copy(dir.path(), "search");
    with_engine(&core, &script).await;
    core.call(
        "startAnalysis",
        vec![json!({ "fen": START, "lines": 1, "infinite": true })],
    )
    .await
    .expect("analysis started");
    support::wait_until(WAIT, "the analysis to run", || {
        core.analysis_running_for_tests().then_some(())
    })
    .await;
    // A release endpoint that accepts connections and never answers: the install waits on it.
    let silent = TcpListener::bind("127.0.0.1:0").expect("listener");
    let release = format!("http://{}/release", silent.local_addr().expect("address"));
    let engine_dir = dir.path().join("managed-install");
    let installing = {
        let core = Arc::clone(&core);
        let engine_dir = engine_dir.clone();
        tokio::spawn(async move {
            core.managed_engine_for_tests(true, 7, Some(engine_dir), Some(release))
                .await
        })
    };
    // The install is waiting on its download when the core closes.
    let _download = support::accepted(&silent).await;
    core.close().await;
    let outcome = tokio::time::timeout(WAIT, installing)
        .await
        .expect("the install ends when the core closes")
        .expect("the install task");
    let error = outcome.expect_err("a closed core does not install");
    assert!(
        error.aborted,
        "the install was cancelled, not failed: {}",
        error.message
    );
    assert!(!core.analysis_running_for_tests());
    drop(silent);
}

#[tokio::test]
async fn account_calls_use_the_username_as_validated() {
    // `" Alice "` passed validation trimmed but reached the store untrimmed, matching nothing.
    let dir = profile();
    let host = TestHost::new();
    let core = core(&host, dir.path());
    core.call("addAccount", vec![json!("Alice")])
        .await
        .expect("added");
    let data = core
        .call("removeAccount", vec![json!(" Alice ")])
        .await
        .expect("removed");
    assert_eq!(data["accounts"], json!([]));
    core.close().await;
}

/// The embedding frontend says who it is: a command line has no window, so no connection alerts
/// until a window connects, and each change of owner is reported once as `alerts:owner`.
#[tokio::test]
async fn the_embedding_frontend_and_each_window_move_the_alert_owner() {
    use kchess_core::connection::{APP, Capability, ClientInfo};
    let host = TestHost::new();
    let dir = profile();
    let core = core(&host, dir.path());
    let client = |kind: &str, capabilities: Vec<Capability>| ClientInfo {
        kind: kind.into(),
        release: String::new(),
        capabilities,
    };
    core.embedded_by(client("cli", vec![]));
    assert_eq!(core.connections().info(APP).expect("the app").kind, "cli");
    let window = core.connect(client("gpui", vec![Capability::Focus]));
    core.focus(window, true);
    core.disconnect(window);
    assert_eq!(
        host.named("alerts:owner"),
        vec![
            json!({ "connection": null }),
            json!({ "connection": window }),
            json!({ "connection": null }),
        ]
    );
    core.close().await;
}
