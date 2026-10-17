//! Core close cancels explorer HTTP instead of waiting out its response deadline.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]
#[path = "support/lichess_http.rs"]
mod fixtures;

use fixtures::{Recording, Reply, serve};
use kchess_core::{Config, Core};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// Restores the test-only Lichess origin (and debug switch) when dropped, so a
/// later test in this binary never inherits this test's fixture origin.
/// Environment variables are process-global and same-binary tests share them.
struct LichessOriginGuard {
    debug: Option<String>,
    base: Option<String>,
}

impl LichessOriginGuard {
    fn set(base: &str) -> Self {
        let previous = Self {
            debug: std::env::var("KCHESS_STORE_DEBUG").ok(),
            base: std::env::var("KCHESS_TEST_LICHESS_BASE").ok(),
        };
        // SAFETY: the guard below restores the previous values when the test ends.
        unsafe {
            std::env::set_var("KCHESS_STORE_DEBUG", "1");
            std::env::set_var("KCHESS_TEST_LICHESS_BASE", base);
        }
        previous
    }
}

impl Drop for LichessOriginGuard {
    fn drop(&mut self) {
        // SAFETY: restores what `set` replaced; no other test in this binary
        // touches these variables while the guard is alive.
        unsafe {
            match self.debug.take() {
                Some(value) => std::env::set_var("KCHESS_STORE_DEBUG", value),
                None => std::env::remove_var("KCHESS_STORE_DEBUG"),
            }
            match self.base.take() {
                Some(value) => std::env::set_var("KCHESS_TEST_LICHESS_BASE", value),
                None => std::env::remove_var("KCHESS_TEST_LICHESS_BASE"),
            }
        }
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
    let (_, result) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(core.close(), lookup)
    })
    .await
    .expect("close cancels the HTTP request immediately");
    assert!(result.unwrap_err().aborted);
    let next = Core::open(Config::new(dir.path().to_path_buf()), host).unwrap();
    next.close().await;
}
