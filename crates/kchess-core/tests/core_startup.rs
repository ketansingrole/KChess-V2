//! Regression: a packaged app hung at startup because the first store read asked the OS secret
//! store whether encryption is available (on macOS a keychain read that can wait on a prompt).
//! Only an import of an earlier release's tokens may ask, as `ensureMigrated` did.

use std::sync::{Arc, Mutex};

use kchess_core::{Config, Core, Host, Level};
use serde_json::Value;

#[derive(Default)]
struct Wire {
    requests: Mutex<Vec<String>>,
}

impl Host for Wire {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, event: &str, payload: Value) {
        if event == "host:request" {
            let kind = payload["kind"].as_str().unwrap_or_default().to_string();
            self.requests.lock().expect("requests lock").push(kind);
        }
    }
}

#[tokio::test]
async fn a_fresh_profile_starts_without_asking_the_secret_store() {
    let dir = tempfile::tempdir().expect("profile");
    let host = Arc::new(Wire::default());
    let core = Core::open(Config::new(dir.path().to_path_buf()), host.clone()).expect("opens");
    // An unanswered request would make this wait for the capability deadline; it must not ask.
    let data = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        core.call("loadData", vec![]),
    )
    .await
    .expect("loadData answers without waiting on the host")
    .expect("loadData");
    assert!(data.is_object());
    let asked = host.requests.lock().expect("requests lock").clone();
    assert!(
        asked.iter().all(|kind| !kind.starts_with("secrets.")),
        "startup asked the secret store: {asked:?}"
    );
    core.close().await;
}
