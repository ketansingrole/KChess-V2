//! The core reads arguments as their checks leave them.

use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core};
use serde_json::{Value, json};

/// A page resuming its session reads it, and the drills, alone: through the facade each answers
/// what the whole library holds for it, and a kind no session has is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_and_the_drills_are_read_alone() {
    let dir = tempfile::tempdir().expect("profile");
    let core = Core::open(Config::new(dir.path().to_path_buf()), TestHost::quiet()).expect("opens");
    let kept = json!({ "pgn": "1. e4 e5 *", "path": "e2e4", "orientation": "black" });
    core.call("saveSession", vec![json!("analysis"), kept])
        .await
        .expect("save");
    let library = core.call("library", vec![]).await.expect("library");
    let session = core
        .call("session", vec![json!("analysis")])
        .await
        .expect("session");
    assert!(!session.is_null(), "the session is kept");
    assert_eq!(session, library["sessions"]["analysis"]);
    assert_eq!(
        core.call("session", vec![json!("local")])
            .await
            .expect("local"),
        Value::Null
    );
    assert!(core.call("session", vec![json!("nonsense")]).await.is_err());
    assert_eq!(
        core.call("mistakes", vec![]).await.expect("mistakes"),
        library["mistakes"]
    );
    core.close().await;
}

/// `reviewGet` finds a standard game's review by its two arguments, and another variant's by a
/// third: a game with no variant is not refused as an "Invalid variant." (the app's lookup after a
/// restart once was).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_get_takes_an_optional_variant() {
    let dir = tempfile::tempdir().expect("profile");
    let core = Core::open(Config::new(dir.path().to_path_buf()), TestHost::quiet()).expect("opens");
    // Assistance waits for the check that no Lichess game is live.
    core.call("resumeOnline", vec![]).await.expect("no game");
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let zh = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
    let moves = json!(["e2e4", "d7d5", "e4d5", "d8d5"]);
    for args in [
        vec![json!(start), moves.clone()],
        vec![json!(start), moves.clone(), Value::Null],
        vec![
            json!(zh),
            json!(["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"]),
            json!("crazyhouse"),
        ],
    ] {
        let found = core.call("reviewGet", args.clone()).await;
        assert_eq!(
            found.map_err(|cause| cause.message),
            Ok(Value::Null),
            "{args:?}"
        );
    }
    // A drop is not a standard move.
    let refused = core
        .call("reviewGet", vec![json!(start), json!(["P@e3"])])
        .await;
    assert!(refused.is_err());
    core.close().await;
}
