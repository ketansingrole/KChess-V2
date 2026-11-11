//! What the core answers for online play, watching and presence, byte for byte as recorded
//! (`support::wire`): the games the account is playing, who is online, the TV channels and the
//! broadcasts, over a local Lichess with `Alice` signed in.
use crate::fixtures;
use crate::signed_in;
use crate::support::wire::assert_wire;

use fixtures::{Reply, serve};
use serde_json::{Value, json};

fn lichess(path: &str) -> Reply {
    let json = |value: Value| Reply::Json(200, value.to_string());
    match path {
        "/api/account/playing" => json(json!({ "nowPlaying": [
            {
                "gameId": "Corr1234", "color": "white", "fen": "8/8/8/8/8/8/8/K6k w - - 0 1",
                "lastMove": "e2e4", "isMyTurn": false, "secondsLeft": 86400,
                "variant": { "key": "standard" }, "speed": "correspondence", "rated": true,
                "opponent": { "username": "Bob", "rating": 1650 },
                "tournamentId": "Abcd1234"
            },
            {
                "gameId": "Corr5678", "color": "black", "fen": "8/8/8/8/8/8/8/K6k b - - 0 1",
                "lastMove": "", "isMyTurn": true, "secondsLeft": 12.5,
                "variant": { "key": "chess960" }, "speed": "correspondence", "rated": false,
                "opponent": { "ai": 3 }, "compat": { "board": false }
            }
        ] })),
        "/api/users/status" => json(json!([
            { "id": "bob", "name": "Bob", "online": true, "playing": true, "signal": 4,
              "playingId": "Game1234" },
            { "id": "carol", "name": "Carol", "online": false },
            { "name": "nobody" }
        ])),
        "/api/tv/channels" => json(json!({
            "best": { "user": { "name": "Magnus", "title": "GM" }, "rating": 2900, "gameId": "Best1234" },
            "bullet": { "user": { "name": "Fast" }, "rating": 2500.5, "gameId": "Bull1234" },
            "blitz": { "user": "not a user" }
        })),
        "/api/broadcast/top" => json(json!({
            "active": [{
                "tour": { "id": "tourAAAA", "name": "Live Open", "image": "https://image.lichess1.org/x.png",
                          "info": { "players": "Carlsen, Nakamura" } },
                "round": { "id": "roundAAA", "name": "Round 3", "ongoing": true, "startsAt": 1800000000000i64 }
            }],
            "upcoming": [{
                "tour": { "id": "tourBBBB", "name": "Next Cup", "description": "Soon",
                          "image": "https://elsewhere.org/x.png" },
                "round": { "id": "roundBBB", "name": "Round 1", "finished": false }
            }],
            "past": { "currentPageResults": [
                { "tour": { "id": "tourCCCC", "name": "Old Masters" }, "round": null }
            ] }
        })),
        "/api/broadcast/tourAAAA" => json(json!({
            "tour": { "id": "tourAAAA", "name": "Live Open", "description": "Nine rounds" },
            "rounds": [
                { "id": "roundAAA", "name": "Round 3", "ongoing": true, "startsAt": 1800000000000i64 },
                { "id": "roundZZZ", "name": "Round 4", "finished": null },
                { "name": "unreadable" }
            ],
            "defaultRoundId": "roundAAA"
        })),
        "/api/broadcast/tourBBBB" => json(json!({
            "tour": { "id": "tourBBBB", "name": "Next Cup" },
            "rounds": []
        })),
        "/api/broadcast/-/-/roundAAA" => json(json!({ "tour": { "id": "tourAAAA" } })),
        _ => Reply::Hang,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_play_and_watching_answer_as_recorded() {
    let fixture = serve(|seen| lichess(seen.path())).await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;

    let games = core.call("ongoingGames", vec![]).await.unwrap();
    assert_eq!(games.as_array().map(Vec::len), Some(2));
    assert_wire("ongoingGames.correspondence", &games);
    // Only correspondence games: nothing goes on the board.
    let resumed = core.call("resumeOnline", vec![]).await.unwrap();
    assert_wire("resumeOnline.none", &resumed);

    // The round trip is the clock's: a whole number of milliseconds, recorded as 0.
    let mut presence = core
        .call("presence", vec![json!(["Bob", "carol"])])
        .await
        .unwrap();
    assert!(presence["latencyMs"].is_u64());
    presence["latencyMs"] = json!(0);
    assert_wire("presence.users", &presence);

    let channels = core.call("tvChannels", vec![]).await.unwrap();
    assert_wire("tvChannels.listed", &channels);
    let featured = core.call("broadcasts", vec![]).await.unwrap();
    assert_wire("broadcasts.top", &featured);
    let tour = core
        .call("broadcastTour", vec![json!("tourAAAA")])
        .await
        .unwrap();
    assert_wire("broadcastTour.rounds", &tour);
    let empty = core
        .call("broadcastTour", vec![json!("tourBBBB")])
        .await
        .unwrap();
    assert_wire("broadcastTour.empty", &empty);
    let round = core
        .call("broadcastRound", vec![json!("roundAAA")])
        .await
        .unwrap();
    assert_wire("broadcastRound.found", &round);

    // A watch answers its session, which stops it.
    let session = core
        .call("watch", vec![json!({ "gameId": "Game1234" })])
        .await
        .unwrap();
    assert_wire("watch.session", &session);
    let stopped = core.call("stopWatching", vec![session]).await.unwrap();
    assert_wire("stopWatching.done", &stopped);
    let round_session = core
        .call("watchBroadcast", vec![json!("roundAAA")])
        .await
        .unwrap();
    assert_wire("watchBroadcast.session", &round_session);
    core.call("stopWatching", vec![]).await.unwrap();
    core.close().await;
}
