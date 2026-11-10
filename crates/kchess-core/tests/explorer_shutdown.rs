//! Core close cancels explorer HTTP instead of waiting out its response deadline.
#[path = "support/lichess_http.rs"]
mod fixtures;

use fixtures::{Recording, Reply, serve};
use kchess_core::{Config, Core};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// Drops the test-only Lichess origin (and debug switch) with the guard, so the other tests in
/// this binary never inherit one test's fixture origin: the overrides are process-wide.
struct LichessOriginGuard;

impl LichessOriginGuard {
    fn set(base: &str) -> Self {
        kchess_core::env::set("KCHESS_STORE_DEBUG", "1");
        kchess_core::env::set("KCHESS_TEST_LICHESS_BASE", base);
        LichessOriginGuard
    }
}

impl Drop for LichessOriginGuard {
    fn drop(&mut self) {
        kchess_core::env::reset("KCHESS_STORE_DEBUG");
        kchess_core::env::reset("KCHESS_TEST_LICHESS_BASE");
    }
}

#[tokio::test]
async fn close_cancels_a_stalled_explorer_and_releases_the_profile() {
    let fixture = serve(|_| Reply::Hang).await;
    let _origin = LichessOriginGuard::set(&fixture.base);
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let core = Core::open(Config::new(dir.path().to_path_buf()), host.clone()).unwrap();
    core.call("resumeOnline", vec![]).await.unwrap();
    let lookup = core.call(
        "positionLookup",
        vec![json!("tablebase"), json!("6k1/8/8/8/8/8/8/R5K1 w - - 0 1")],
    );
    tokio::pin!(lookup);
    tokio::select! {
        biased;
        _ = &mut lookup => panic!("the fixture never responds"),
        _ = async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while fixture.requests().is_empty() { tokio::time::sleep(Duration::from_millis(5)).await; }
            }).await.expect("the lookup reaches the fixture");
        } => {}
    }
    // The lookup is bounded well inside its own 15 s response deadline, and close only as a whole:
    // closing also flushes and closes the profile's databases, which took over 2 s on a Windows
    // runner busy with sixteen test processes (2026-11-10).
    let (closed, result) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(30), core.close()),
        tokio::time::timeout(Duration::from_secs(5), lookup),
    );
    closed.expect("close finishes");
    let result = result.expect("close cancels the HTTP request, not its deadline");
    assert!(result.unwrap_err().aborted);
    let next = Core::open(Config::new(dir.path().to_path_buf()), host).unwrap();
    next.close().await;
}
