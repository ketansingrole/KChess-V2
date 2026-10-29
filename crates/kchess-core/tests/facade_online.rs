//! The facade's online methods: arguments are checked before anything is sent, account-bound
//! calls without a login ask to reconnect instead of reaching Lichess, and the player lookups
//! reach Lichess as the facade normalised them.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]
#[path = "support/lichess_http.rs"]
mod fixtures;

use fixtures::{Recording, Reply, serve};
use kchess_core::{Config, Core};
use serde_json::{Value, json};
use std::sync::Arc;

#[tokio::test]
async fn online_methods_check_their_arguments_and_reach_lichess() {
    let fixture = serve(|seen| match seen.path() {
        "/api/users" => Reply::Json(200, "[]".into()),
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    // SAFETY: this test binary has one test; the fixture origin is set before opening the core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", &fixture.base);
    }
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(
        Config::new(dir.path().to_path_buf()),
        Arc::new(Recording::default()),
    )
    .unwrap();
    let sent = || fixture.requests().len();

    // Invalid arguments are refused before anything reaches Lichess.
    for (method, args, message) in [
        ("startOnline", vec![json!({})], "Invalid game options."),
        (
            "playOnline",
            vec![json!("abcdefgh"), json!("zz")],
            "Invalid move.",
        ),
        (
            "importGame",
            vec![Value::Null, json!("1. e4 *")],
            "Invalid account.",
        ),
    ] {
        let error = core.call(method, args).await.unwrap_err();
        assert_eq!(error.message, message, "{method}");
    }
    for (method, args) in [
        ("playerPerf", vec![json!("Alice"), json!("bullet-ish")]),
        ("crosstable", vec![json!("Alice"), json!("not a name!")]),
        ("leaderboard", vec![json!(3)]),
        ("onlineAction", vec![json!("abcdefgh"), json!("explode")]),
        (
            "sendChat",
            vec![json!("abcdefgh"), json!("lobby"), json!("hi")],
        ),
        ("declineChallenge", vec![json!("bad id!")]),
        (
            "setRelation",
            vec![json!("Alice"), json!("Bob"), json!("hug")],
        ),
        ("profiles", vec![json!("Alice")]),
    ] {
        assert!(core.call(method, args).await.is_err(), "{method}");
    }
    assert_eq!(sent(), 0);

    // Without a login, the account's own calls ask to reconnect and send nothing.
    for (method, args) in [
        ("timeline", vec![json!("Alice")]),
        ("bookmarkedGames", vec![json!("Alice")]),
        ("relation", vec![json!("Alice"), json!("Bob")]),
        (
            "sendMessage",
            vec![json!("Alice"), json!("Bob"), json!("hi")],
        ),
    ] {
        assert_eq!(
            core.call(method, args).await.unwrap(),
            json!({ "needsReconnect": true }),
            "{method}"
        );
    }
    assert_eq!(core.call("challenges", vec![]).await.unwrap(), json!([]));
    assert_eq!(core.call("ongoingGames", vec![]).await.unwrap(), json!([]));
    assert_eq!(
        core.call("cancelOnline", vec![]).await.unwrap(),
        Value::Null
    );
    // A challenge the core does not know is never answered.
    for (method, args) in [
        ("acceptChallenge", vec![json!("abcdefgh")]),
        (
            "declineChallenge",
            vec![json!("abcdefgh"), json!("generic")],
        ),
        ("cancelChallenge", vec![json!("abcdefgh")]),
    ] {
        let error = core.call(method, args).await.unwrap_err();
        assert_eq!(
            error.message, "That challenge is no longer open.",
            "{method}"
        );
    }
    assert_eq!(sent(), 0);

    // Public lookups reach Lichess with the facade's arguments.
    let mut expected = Vec::new();
    for (method, args, target) in [
        (
            "crosstable",
            vec![json!("Alice"), json!("Bob")],
            "GET /api/crosstable/Alice/Bob?matchup=true",
        ),
        (
            "recentGames",
            vec![json!("Alice"), json!(true)],
            "GET /api/games/user/Alice?max=200&moves=false&opening=false&ongoing=false&rated=true",
        ),
        (
            "leaderboard",
            vec![json!("blitz")],
            "GET /api/player/top/100/blitz",
        ),
        ("profiles", vec![json!(["Alice", "Bob"])], "POST /api/users"),
        (
            "searchPlayers",
            vec![json!("ali")],
            "GET /api/player/autocomplete?term=ali&object=true",
        ),
        (
            "activity",
            vec![json!("Alice")],
            "GET /api/user/Alice/activity",
        ),
        (
            "presence",
            vec![json!(["Alice"])],
            "GET /api/users/status?ids=alice&withSignal=true&withGameIds=true",
        ),
        (
            "playerPerf",
            vec![json!("Alice"), json!("blitz")],
            "GET /api/user/Alice/perf/blitz",
        ),
    ] {
        // The fixture's empty answers are not every endpoint's shape; only the request matters.
        let _ = core.call(method, args).await;
        expected.push(target.to_string());
        let seen: Vec<String> = fixture
            .requests()
            .iter()
            .map(|seen| format!("{} {}", seen.method, seen.target))
            .collect();
        assert_eq!(seen, expected, "{method}");
    }
    assert_eq!(fixture.to("/api/users")[0].body, "Alice,Bob");
    core.close().await;
}
