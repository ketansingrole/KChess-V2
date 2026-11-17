//! Regression: a packaged app hung at startup because the first store read asked the OS secret
//! store whether encryption is available (on macOS a keychain read that can wait on a prompt).
//! Only an import of an earlier release's tokens may ask, as `ensureMigrated` did.

use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};

#[tokio::test]
async fn a_fresh_profile_starts_without_asking_the_secret_store() {
    let dir = tempfile::tempdir().expect("profile");
    let host = TestHost::new();
    let core = Core::open(Config::new(dir.path().to_path_buf()), host.clone()).expect("opens");
    // What it asked the host is the test, not how long it took: a five-second limit failed on a
    // loaded Windows runner whose startup asked nothing (2026-11-14). A startup that asked would
    // wait out the capability deadline and is then refused below by what it asked.
    let data = core.call("loadData", vec![]).await.expect("loadData");
    assert!(data.is_object());
    let asked: Vec<_> = host.named("host:request");
    assert!(
        asked.iter().all(|ask| !ask["kind"]
            .as_str()
            .unwrap_or_default()
            .starts_with("secrets.")),
        "startup asked the secret store: {asked:?}"
    );
    core.close().await;
}
