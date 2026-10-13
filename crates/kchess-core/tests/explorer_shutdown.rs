//! Core close cancels explorer HTTP instead of waiting out its response deadline.
#[path = "support/lichess_http.rs"]
mod fixtures;

use fixtures::{Recording, Reply, serve};
use kchess_core::{Config, Core};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn close_cancels_a_stalled_explorer_and_releases_the_profile() {
    let fixture = serve(|_| Reply::Hang).await;
    // SAFETY: this test binary has one test; the fixture origin is set before opening the core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", &fixture.base);
    }
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
