//! The facade's calls of a live Lichess game, for a signed-in account: each reaches the game's
//! Lichess endpoint, another game cannot start or be watched while it is on the board, and
//! leaving it is reported.

use crate::fixtures::{Reply, serve};
use crate::signed_in;

use serde_json::json;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_games_calls_reach_lichess_until_it_is_left() {
    // The streams stay open (and quiet): a request still waiting for its answer would hold up
    // the account's next one.
    let mut open = Vec::new();
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("GET", "/api/stream/event" | "/api/board/game/stream/AbCd1234") => {
            let (sender, stream) = tokio::sync::mpsc::unbounded_channel();
            open.push(sender);
            Reply::Stream(stream)
        }
        (_, "/api/account/playing") => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234", "speed": "rapid" }] }).to_string(),
        ),
        ("GET", "/api/board/game/AbCd1234/chat") => Reply::Json(
            200,
            json!([{ "user": "Bob", "text": "You too" }]).to_string(),
        ),
        ("POST", _) => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    let resumed = core.call("resumeOnline", vec![]).await.unwrap();
    assert_eq!(resumed["id"], json!("AbCd1234"));

    let game = json!("AbCd1234");
    for (method, args, path) in [
        (
            "playOnline",
            vec![game.clone(), json!("e2e4")],
            "/api/board/game/AbCd1234/move/e2e4",
        ),
        (
            "onlineAction",
            vec![game.clone(), json!("resign")],
            "/api/board/game/AbCd1234/resign",
        ),
        (
            "sendChat",
            vec![game.clone(), json!("player"), json!("Good luck")],
            "/api/board/game/AbCd1234/chat",
        ),
    ] {
        core.call(method, args).await.expect(method);
        assert_eq!(fixture.to(path).len(), 1, "{method}");
    }
    let chat = core.call("onlineChat", vec![game.clone()]).await.unwrap();
    assert_eq!(chat[0]["text"], json!("You too"));
    // A correspondence reply goes only to a game Lichess still lists.
    let gone = core
        .call(
            "playCorrespondence",
            vec![
                json!("Alice"),
                json!("Gone1234"),
                json!(START),
                json!("e2e4"),
            ],
        )
        .await
        .expect_err("a game no longer in progress");
    assert_eq!(gone.message, "This game is no longer in progress.");

    // While the game is on the board, no other starts and none is watched.
    let seek = json!({ "minutes": 10, "increment": 0, "color": "random", "rated": false });
    let refused = core
        .call("startOnline", vec![seek])
        .await
        .expect_err("a second game");
    assert_eq!(
        refused.message,
        "Reconnect to your current game before finding another."
    );
    let refused = core
        .call("watch", vec![json!({ "channel": "best" })])
        .await
        .expect_err("watching while playing");
    assert_eq!(refused.message, "Finish your game before watching another.");
    // Leaving it reports the board as disconnected: the game stays protected until Lichess
    // says it is over.
    let last_phase = || {
        profile
            .events()
            .into_iter()
            .rev()
            .find(|(name, _)| name == "online:state")
            .map(|(_, state)| state["phase"].clone())
    };
    assert_ne!(last_phase(), Some(json!("disconnected")));
    core.call("cancelOnline", vec![]).await.unwrap();
    assert_eq!(last_phase(), Some(json!("disconnected")));
    core.close().await;
}
