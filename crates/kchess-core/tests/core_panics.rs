//! A core method that panics is answered with an error. The shared service runs each socket call
//! as its own task and `Client::call` has no deadline, so a panic used to leave the caller (a
//! page's spinner, a command-line command) waiting for an answer that never came.

use std::sync::Arc;
use std::time::Duration;

use kchess_core::{Config, Core, Host, Level};
use serde_json::Value;

struct Quiet;

impl Host for Quiet {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}
    fn emit(&self, _event: &str, _payload: Value) {}
}

#[tokio::test]
async fn a_method_that_panics_is_answered_with_an_error() {
    kchess_core::env::set("KCHESS_STORE_DEBUG", "1");
    let dir = tempfile::tempdir().expect("profile");
    let core = Core::open(Config::new(dir.path().to_path_buf()), Arc::new(Quiet)).expect("opens");
    let answer = tokio::time::timeout(
        Duration::from_secs(5),
        core.call_as(7, "debug.panic", vec![]),
    )
    .await
    .expect("the call is answered");
    let error = answer.expect_err("a panic is a failure");
    assert!(error.message.contains("debug.panic"), "{}", error.message);
    // The core keeps serving.
    let data = core.call("loadData", vec![]).await.expect("loadData");
    assert!(data.is_object());
    core.close().await;
}
