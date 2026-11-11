//! A core method that panics is answered with an error. The shared service runs each socket call
//! as its own task and `Client::call` has no deadline, so a panic used to leave the caller (a
//! page's spinner, a command-line command) waiting for an answer that never came.

use std::time::Duration;

use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};

#[tokio::test]
async fn a_method_that_panics_is_answered_with_an_error() {
    let dir = tempfile::tempdir().expect("profile");
    let core = Core::open(Config::new(dir.path().to_path_buf()), TestHost::quiet()).expect("opens");
    let answer = tokio::time::timeout(Duration::from_secs(5), core.panic_for_tests(7))
        .await
        .expect("the call is answered");
    let error = answer.expect_err("a panic is a failure");
    assert!(
        error.message.contains("a panicking call"),
        "{}",
        error.message
    );
    // The core keeps serving.
    let data = core.call("loadData", vec![]).await.expect("loadData");
    assert!(data.is_object());
    core.close().await;
}
