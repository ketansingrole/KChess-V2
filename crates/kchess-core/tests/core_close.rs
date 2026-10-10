//! Closing the core (`Core::close`, the facade's close sequence): an engine search, a Lichess
//! request and a puzzle download that are running when the core closes end, `close` returns, and
//! later calls fail with the TypeScript message `The KChess core is closed.`.

mod support;

use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

/// What a closed core answers (`createKChessCore`'s closed check).
const CLOSED: &str = "The KChess core is closed.";
/// How long closing may take before the test calls it stuck.
const LIMIT: Duration = Duration::from_secs(20);

/// Records the events; answers no host request (none of these calls needs one).
#[derive(Default)]
struct Wire {
    events: Mutex<Vec<(String, Value)>>,
}

impl Host for Wire {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, event: &str, payload: Value) {
        self.events
            .lock()
            .expect("events lock")
            .push((event.to_string(), payload));
    }
}

fn open(dir: &Path, host: &Arc<Wire>) -> Arc<Core> {
    Arc::new(Core::open(Config::new(dir.to_path_buf()), host.clone()).expect("the profile opens"))
}

/// An executable that runs the scripted fake engine in `mode`.
fn engine_script(dir: &Path, mode: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!("engine-{mode}.sh"));
    std::fs::write(
        &path,
        format!("#!/bin/sh\nexec '{}' {mode}\n", support::fake_path()),
    )
    .expect("engine script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("engine mode");
    path.display().to_string()
}

/// Trusts `path` as the engine and saves it as the configured executable.
fn configure_engine(core: &Core, path: &str) {
    core.call_sync("engine.trust", vec![json!(path)])
        .expect("trusted engine");
    let mut settings = core
        .call_sync("store.games.getSettings", vec![])
        .expect("settings");
    settings["enginePath"] = json!(path);
    core.call_sync("store.games.saveSettings", vec![settings])
        .expect("save settings");
}

/// Waits until the spawned call has started its work.
async fn let_work_start() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn closing_ends_an_engine_search_in_flight() {
    let dir = tempfile::tempdir().expect("profile");
    let host = Arc::new(Wire::default());
    let core = open(dir.path(), &host);
    configure_engine(&core, &engine_script(dir.path(), "search"));
    // Assistance waits until the core has confirmed there is no live game.
    core.call("resumeOnline", vec![]).await.expect("resume");

    let search = {
        let core = Arc::clone(&core);
        tokio::spawn(async move {
            core.call(
                "bestMove",
                vec![
                    json!(["e2e4"]),
                    json!("beginner"),
                    json!({ "movetime": 60_000 }),
                ],
            )
            .await
        })
    };
    let_work_start().await;

    tokio::time::timeout(LIMIT, core.close())
        .await
        .expect("closing returns while the search runs");
    let answer = tokio::time::timeout(LIMIT, search)
        .await
        .expect("the search ends when the core closes")
        .expect("the search task");
    assert!(answer.is_err(), "the search was cancelled, not answered");

    let later = core
        .call("loadData", vec![])
        .await
        .expect_err("a closed core refuses");
    assert_eq!(later.message, CLOSED);
}

#[tokio::test]
async fn closing_ends_a_lichess_request_in_flight() {
    // The test origin serves Lichess from a socket that never answers.
    let silent = TcpListener::bind("127.0.0.1:0").expect("silent origin");
    let origin = format!(
        "http://127.0.0.1:{}",
        silent.local_addr().expect("address").port()
    );
    // SAFETY: only this test in the binary reads the Lichess origin, before it opens its core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", &origin);
    }
    let dir = tempfile::tempdir().expect("profile");
    let host = Arc::new(Wire::default());
    let core = open(dir.path(), &host);

    let request = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("profile", vec![json!("someone")]).await })
    };
    let_work_start().await;

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

#[tokio::test]
async fn closing_ends_a_puzzle_download_in_flight() {
    let silent = TcpListener::bind("127.0.0.1:0").expect("silent origin");
    let url = format!(
        "http://127.0.0.1:{}/puzzles.csv.zst",
        silent.local_addr().expect("address").port()
    );
    let dir = tempfile::tempdir().expect("profile");
    let host = Arc::new(Wire::default());
    let core = open(dir.path(), &host);

    let install = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("puzzles.install", vec![json!(url)]).await })
    };
    let_work_start().await;

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
    let host = Arc::new(Wire::default());
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
