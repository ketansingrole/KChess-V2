//! A sync saves the player's Lichess play time over a local Lichess. On its own: the signed-in
//! profile reads its Lichess from the process environment, one per test binary.
use crate::fixtures;
use crate::signed_in;

use fixtures::{Reply, serve};
use serde_json::{Value, json};

/// A sync saves the player's Lichess play time, which the Following page compares with a later
/// profile to tell there are new games without downloading them; clearing the data forgets it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sync_saves_the_play_time_it_reached() {
    let fixture = serve(|seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/users" => Reply::Json(
            200,
            json!([{ "username": "Bob", "playTime": { "total": 5000, "tv": 0 } }]).to_string(),
        ),
        "/api/games/user/Bob" => Reply::Json(200, String::new()),
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    let bob = |data: Value| {
        data["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|account| account["username"] == "Bob")
            .cloned()
            .unwrap()
    };
    core.call("addFriends", vec![json!(["Bob"])]).await.unwrap();
    assert_eq!(
        bob(core.call("loadData", vec![]).await.unwrap())["syncedPlayTime"],
        Value::Null
    );

    core.call("syncGames", vec![json!("Bob")]).await.unwrap();
    // Read before the games: one that ends during the sync shows as new afterwards.
    let asked: Vec<String> = fixture
        .requests()
        .iter()
        .filter(|seen| seen.path().ends_with("Bob") || seen.path() == "/api/users")
        .map(|seen| format!("{} {}", seen.method, seen.path()))
        .collect();
    assert_eq!(asked, ["POST /api/users", "GET /api/games/user/Bob"]);
    assert_eq!(fixture.to("/api/users")[0].body, "Bob");
    assert_eq!(
        bob(core.call("loadData", vec![]).await.unwrap())["syncedPlayTime"],
        json!(5000)
    );

    core.call("clearAccountData", vec![json!("Bob")])
        .await
        .unwrap();
    assert_eq!(
        bob(core.call("loadData", vec![]).await.unwrap())["syncedPlayTime"],
        Value::Null
    );
    core.close().await;
}
