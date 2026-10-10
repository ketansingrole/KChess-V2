//! The Lichess read endpoints the TypeScript services answered and the Rust core now answers
//! through `Lichess`: TV channels, a broadcast tournament, the daily puzzle, the puzzle dashboard
//! and activity, and the Storm dashboard. Each test pins the request the endpoint sends and the
//! shape it returns (`core/src/services/{spectate,lichess}.ts`).

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;

use fixtures::{MemoryStore, MemoryTokens, Recording, Reply, lichess_service, serve};
use kchess_core::lichess::accounts::Reply as Outcome;
use kchess_core::lichess::puzzles::{
    puzzle_activity, puzzle_daily, puzzle_dashboard, storm_dashboard,
};
use kchess_core::lichess::watch::{broadcast_tour, tv_channels};
use serde_json::{Value, json};

fn service(base: &str) -> Arc<kchess_core::lichess::accounts::Lichess> {
    lichess_service(
        base,
        &Arc::new(Recording::default()),
        MemoryStore::with_accounts(&[("Alice", true)]),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    )
}

/// A puzzle the TypeScript `puzzleFromApi` reads: a solution and a start ply.
fn readable_puzzle(id: &str) -> Value {
    json!({
        "game": {},
        "puzzle": { "id": id, "rating": 1500, "solution": ["f2f4"], "themes": [], "initialPly": 0 }
    })
}

#[tokio::test]
async fn tv_channels_name_the_game_and_player_of_each_channel() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({
                "best": { "gameId": "abcdefgh", "rating": 2000, "user": { "name": "Bob", "title": "GM" } },
                "blitz": { "gameId": "ijklmnop" }
            })
            .to_string(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let channels = tv_channels(&lichess).await.unwrap();

    assert_eq!(channels.len(), 16);
    assert_eq!(fixture.requests()[0].path(), "/api/tv/channels");
    let best = &channels[0];
    assert_eq!(best.key, "best");
    assert_eq!(best.game_id.as_deref(), Some("abcdefgh"));
    let player = best.player.as_ref().expect("the top channel has a player");
    assert_eq!(player.name, "Bob");
    assert_eq!(player.title.as_deref(), Some("GM"));
    assert_eq!(player.rating, Some(2000.0));
    // A channel without a player has a game id at most, never an invented player.
    let blitz = channels
        .iter()
        .find(|channel| channel.key == "blitz")
        .unwrap();
    assert_eq!(blitz.game_id.as_deref(), Some("ijklmnop"));
    assert!(blitz.player.is_none());
    let bullet = channels
        .iter()
        .find(|channel| channel.key == "bullet")
        .unwrap();
    assert!(bullet.game_id.is_none() && bullet.player.is_none());
}

#[tokio::test]
async fn broadcast_tour_lists_its_rounds_and_skips_unreadable_ones() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({
                "tour": { "id": "AbCdEfGh", "name": "Grand Prix", "description": "Spring" },
                "rounds": [
                    { "id": "Round001", "name": "Round 1", "ongoing": true, "startsAt": 5 },
                    { "id": "Round002", "name": "Round 2", "finished": true },
                    { "name": "No id" }
                ],
                "defaultRoundId": "Round001"
            })
            .to_string(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let tour = broadcast_tour(&lichess, "AbCdEfGh").await.unwrap();

    assert_eq!(fixture.requests()[0].path(), "/api/broadcast/AbCdEfGh");
    assert_eq!(tour.id, "AbCdEfGh");
    assert_eq!(tour.name, "Grand Prix");
    assert_eq!(tour.description.as_deref(), Some("Spring"));
    assert_eq!(tour.rounds.len(), 2);
    assert_eq!(tour.rounds[0].id, "Round001");
    assert!(tour.rounds[0].ongoing && !tour.rounds[0].finished);
    assert_eq!(tour.rounds[0].starts_at, Some(5));
    assert!(tour.rounds[1].finished && !tour.rounds[1].ongoing);
    assert_eq!(tour.default_round_id.as_deref(), Some("Round001"));
}

#[tokio::test]
async fn the_daily_puzzle_needs_no_login_and_is_read_as_a_puzzle() {
    let fixture = serve(|_| Reply::Json(200, readable_puzzle("daily1").to_string())).await;
    let lichess = service(&fixture.base);
    let puzzle = puzzle_daily(&lichess).await.unwrap();

    let sent = &fixture.requests()[0];
    assert_eq!(sent.path(), "/api/puzzle/daily");
    assert_eq!(sent.header("authorization"), None);
    assert_eq!(puzzle["id"], "daily1");
}

#[tokio::test]
async fn the_daily_puzzle_kchess_cannot_read_is_an_error() {
    let fixture =
        serve(|_| Reply::Json(200, json!({ "game": {}, "puzzle": {} }).to_string())).await;
    let lichess = service(&fixture.base);
    let error = puzzle_daily(&lichess).await.unwrap_err();
    assert_eq!(
        error.message,
        "Lichess sent a puzzle KChess could not read."
    );
}

#[tokio::test]
async fn the_puzzle_dashboard_is_the_accounts_own_results_for_the_days_asked() {
    let fixture = serve(|_| Reply::Json(200, json!({ "solved": 7, "days": 30 }).to_string())).await;
    let lichess = service(&fixture.base);
    let Outcome::Done(dashboard) = puzzle_dashboard(&lichess, "Alice", 30).await.unwrap() else {
        panic!("the login is connected");
    };

    assert_eq!(dashboard, json!({ "solved": 7, "days": 30 }));
    let sent = &fixture.requests()[0];
    assert_eq!(sent.path(), "/api/puzzle/dashboard/30");
    assert_eq!(sent.header("authorization"), Some("Bearer lip_alice"));
}

#[tokio::test]
async fn a_refused_puzzle_dashboard_asks_the_user_to_connect_again() {
    let fixture = serve(|_| Reply::Json(401, "{}".into())).await;
    let lichess = service(&fixture.base);
    assert_eq!(
        puzzle_dashboard(&lichess, "Alice", 7).await.unwrap(),
        Outcome::NeedsReconnect
    );
}

#[tokio::test]
async fn puzzle_activity_keeps_the_readable_results_newest_first_and_skips_the_rest() {
    let body = [
        json!({ "date": 2, "win": true, "puzzle": { "id": "a", "rating": 1500, "solution": ["f2f4"], "themes": [] } }),
        json!({ "date": 1, "win": false, "puzzle": { "id": "b" } }),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let fixture = serve(move |_| Reply::Json(200, body.clone())).await;
    let lichess = service(&fixture.base);
    let Outcome::Done(entries) = puzzle_activity(&lichess, "Alice", 2).await.unwrap() else {
        panic!("the login is connected");
    };

    let sent = &fixture.requests()[0];
    assert_eq!(sent.path(), "/api/puzzle/activity");
    assert!(sent.target.ends_with("max=2"), "target was {}", sent.target);
    assert_eq!(sent.header("authorization"), Some("Bearer lip_alice"));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].date, json!(2));
    assert!(entries[0].win);
    assert_eq!(entries[0].puzzle["id"], "a");
}

#[tokio::test]
async fn storm_dashboard_needs_no_login_and_names_an_unknown_user() {
    let fixture = serve(|seen| {
        if seen.path().ends_with("/Bob") {
            Reply::Json(200, json!({ "high": 42, "games": 3 }).to_string())
        } else {
            Reply::Json(404, "{}".into())
        }
    })
    .await;
    let lichess = service(&fixture.base);

    let dashboard = storm_dashboard(&lichess, "Bob", 30).await.unwrap();
    assert_eq!(dashboard, json!({ "high": 42, "games": 3 }));
    let sent = &fixture.requests()[0];
    assert_eq!(sent.path(), "/api/storm/dashboard/Bob");
    assert!(
        sent.target.ends_with("days=30"),
        "target was {}",
        sent.target
    );
    assert_eq!(sent.header("authorization"), None);

    let error = storm_dashboard(&lichess, "Nobody", 30).await.unwrap_err();
    assert!(
        error.message.ends_with("no such Lichess user \"Nobody\""),
        "message was {}",
        error.message
    );
}
