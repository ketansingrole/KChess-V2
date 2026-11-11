//! Core close cancels explorer HTTP instead of waiting out its response deadline.
use crate::fixtures::{Recording, Reply, serve};
use kchess_core::{Config, Core, TestSwitches};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// A profile at `dir` whose Lichess is `base`.
fn config(dir: &std::path::Path, base: &str) -> Config {
    Config {
        test: Some(TestSwitches {
            lichess_origin: Some(base.to_string()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.to_path_buf())
    }
}

#[tokio::test]
async fn close_cancels_a_stalled_explorer_and_releases_the_profile() {
    let fixture = serve(|_| Reply::Hang).await;
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let core = Core::open(config(dir.path(), &fixture.base), host.clone()).unwrap();
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
            fixture.wait_for("the lookup", |requests| !requests.is_empty()).await;
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
    let next = Core::open(config(dir.path(), &fixture.base), host).unwrap();
    next.close().await;
}
