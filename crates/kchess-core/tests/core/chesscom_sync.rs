//! Chess.com game sync (`crates/kchess-core/src/chesscom.rs`) against a local fixture of its public
//! API: the archive list, months read once they have ended, conditional requests for the current
//! month, a rate limit waited out, a removal that overtakes a sync, and an unknown player.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::Config;
use kchess_core::chesscom::Chesscom;
use kchess_core::store::Database;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::fixtures::{Fixture, Reply, Seen, serve};
use crate::support::Recording;

const PAST: &str = "2020/01";

fn this_month() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        / 86_400;
    let (year, month, _) = kchess_domain::calendar::civil_from_days(days).unwrap();
    format!("{year:04}/{month:02}")
}

fn game(number: u64, white: &str, black: &str, white_result: &str, black_result: &str) -> Value {
    json!({
        "url": format!("https://www.chess.com/game/live/{number}"),
        "pgn": format!("[White \"{white}\"]\n[Black \"{black}\"]\n[UTCDate \"2020.01.0{}\"]\n[UTCTime \"10:00:00\"]\n\n1. e4 e5 2. Nf3 Nc6 *", number % 9 + 1),
        "time_control": "600",
        "time_class": "rapid",
        "end_time": 1_578_000_000 + number as i64,
        "rated": true,
        "rules": "chess",
        "white": { "username": white, "rating": 1500, "result": white_result },
        "black": { "username": black, "rating": 1480, "result": black_result },
    })
}

fn month_body(games: &[Value]) -> String {
    json!({ "games": games }).to_string()
}

struct Profile {
    _dir: tempfile::TempDir,
    database: Arc<Database>,
    chesscom: Chesscom,
}

fn profile(fixture: &Fixture) -> Profile {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let database = Arc::new(Database::new(
        Config::new(dir.path().to_path_buf()),
        host.clone(),
    ));
    let chesscom = Chesscom::new(
        Arc::clone(&database),
        host,
        Some(fixture.base.clone()),
        CancellationToken::new(),
    );
    Profile {
        _dir: dir,
        database,
        chesscom,
    }
}

impl Profile {
    async fn games(&self, account: &str) -> Vec<Value> {
        let page = self
            .database
            .call_async(
                "store.games.gamePage",
                &[json!({ "account": account, "offset": 0, "limit": 100 })],
            )
            .await
            .unwrap();
        page["games"].as_array().cloned().unwrap_or_default()
    }
}

/// The archive list names a past month and this one; the profile says how Chess.com spells the name.
fn archives(base_month: &str) -> String {
    json!({ "archives": [
        format!("https://api.chess.com/pub/player/alice/games/{PAST}"),
        format!("https://api.chess.com/pub/player/alice/games/{base_month}"),
    ]})
    .to_string()
}

fn profile_body() -> String {
    json!({ "username": "alice", "url": "https://www.chess.com/member/Alice" }).to_string()
}

#[tokio::test]
async fn a_player_is_synced_and_a_second_sync_asks_only_for_this_month_if_changed() {
    let now = this_month();
    let month_path = format!("/pub/player/alice/games/{now}");
    let past_path = format!("/pub/player/alice/games/{PAST}");
    let (month, past) = (month_path.clone(), past_path.clone());
    let fixture = serve(move |seen: &Seen| match seen.path() {
        "/pub/player/alice" => Reply::Json(200, profile_body()),
        "/pub/player/alice/games/archives" => Reply::Json(200, archives(&this_month())),
        path if path == past => Reply::WithHeaders(
            200,
            vec![("ETag", "\"past\"".into())],
            month_body(&[
                game(1, "Alice", "bob", "win", "resigned"),
                game(2, "carol", "alice", "agreed", "agreed"),
                json!({ "url": "https://www.chess.com/game/live/3", "rules": "bughouse",
                        "pgn": "1. e4 *", "white": {"username": "alice"}, "black": {"username": "x"} }),
            ]),
        ),
        path if path == month => {
            if seen.header("If-None-Match") == Some("\"now\"") {
                Reply::Text(304, "application/json", String::new())
            } else {
                Reply::WithHeaders(
                    200,
                    vec![("ETag", "\"now\"".into())],
                    month_body(&[game(4, "dave", "Alice", "checkmated", "win")]),
                )
            }
        }
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let profile = profile(&fixture);
    let accounts = profile.chesscom.add("ALICE").await.expect("added");
    assert_eq!(accounts[0]["username"], "Alice", "{accounts}");
    assert!(accounts[0]["lastSyncedAt"].is_i64(), "{accounts}");
    let games = profile.games("Alice@chess.com").await;
    let ids: Vec<&str> = games.iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 3, "bughouse is not kept: {ids:?}");
    assert!(ids.contains(&"cc-l1") && ids.contains(&"cc-l2") && ids.contains(&"cc-l4"));
    let won = games.iter().find(|g| g["id"] == "cc-l4").unwrap();
    assert_eq!(won["color"], "black");
    assert_eq!(won["winner"], "black");
    assert_eq!(won["speed"], "rapid");

    profile.chesscom.sync(None).await.expect("second sync");
    assert_eq!(
        fixture.to(&past_path).len(),
        1,
        "an ended month is read once"
    );
    let month_requests = fixture.to(&month_path);
    assert_eq!(month_requests.len(), 2);
    assert_eq!(month_requests[1].header("If-None-Match"), Some("\"now\""));
    assert_eq!(profile.games("Alice@chess.com").await.len(), 3);
    // Every request names the app.
    assert!(fixture.requests().iter().all(|seen| {
        seen.header("User-Agent")
            .is_some_and(|agent| agent.starts_with("KChess"))
    }));
}

#[tokio::test]
async fn a_rate_limited_month_is_asked_again_after_the_wait() {
    let limited = Arc::new(AtomicUsize::new(0));
    let seen_limits = Arc::clone(&limited);
    let fixture = serve(move |seen: &Seen| match seen.path() {
        "/pub/player/alice" => Reply::Json(200, profile_body()),
        "/pub/player/alice/games/archives" => Reply::Json(
            200,
            json!({ "archives": [format!("https://api.chess.com/pub/player/alice/games/{PAST}")] })
                .to_string(),
        ),
        path if path.ends_with(PAST) => {
            if seen_limits.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply::WithHeaders(429, vec![("Retry-After", "0".into())], "{}".into())
            } else {
                Reply::Json(
                    200,
                    month_body(&[game(7, "Alice", "bob", "win", "timeout")]),
                )
            }
        }
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let profile = profile(&fixture);
    profile.chesscom.add("alice").await.expect("added");
    assert_eq!(limited.load(Ordering::SeqCst), 2);
    let games = profile.games("Alice@chess.com").await;
    assert_eq!(games.len(), 1);
    assert_eq!(games[0]["status"], "outoftime");
}

#[tokio::test]
async fn removing_a_player_during_their_sync_leaves_nothing_behind() {
    let (open, gate) = watch::channel(false);
    let gate = Arc::new(Mutex::new(Some(gate)));
    let fixture = serve(move |seen: &Seen| match seen.path() {
        "/pub/player/alice" => Reply::Json(200, profile_body()),
        "/pub/player/alice/games/archives" => Reply::Json(
            200,
            json!({ "archives": [format!("https://api.chess.com/pub/player/alice/games/{PAST}")] })
                .to_string(),
        ),
        path if path.ends_with(PAST) => match gate.lock().unwrap().take() {
            Some(gate) => Reply::Gated(
                gate,
                Box::new(Reply::Json(
                    200,
                    month_body(&[game(9, "Alice", "bob", "win", "resigned")]),
                )),
            ),
            None => Reply::Json(200, month_body(&[])),
        },
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let profile = Arc::new(profile(&fixture));
    let adding = {
        let profile = Arc::clone(&profile);
        tokio::spawn(async move { profile.chesscom.add("alice").await })
    };
    for _ in 0..200 {
        if !fixture
            .to(&format!("/pub/player/alice/games/{PAST}"))
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let left = profile.chesscom.remove("alice").await.expect("removed");
    assert_eq!(left, json!([]));
    open.send(true).unwrap();
    let outcome = adding.await.unwrap();
    assert!(outcome.is_err(), "the sync was overtaken: {outcome:?}");
    assert!(profile.games("Alice@chess.com").await.is_empty());
    let accounts = profile
        .database
        .call_async("store.chesscom.accounts", &[])
        .await
        .unwrap();
    assert_eq!(accounts, json!([]));
}

#[tokio::test]
async fn an_unknown_player_is_not_added() {
    let fixture = serve(|_: &Seen| Reply::Json(404, "{\"code\":0}".into())).await;
    let profile = profile(&fixture);
    let error = profile.chesscom.add("nobody_here").await.unwrap_err();
    assert_eq!(error.message, "No Chess.com player is called nobody_here.");
    let accounts = profile
        .database
        .call_async("store.chesscom.accounts", &[])
        .await
        .unwrap();
    assert_eq!(accounts, json!([]));
}

#[tokio::test]
async fn chesscom_games_are_reviewed_without_asking_lichess() {
    let fixture = serve(move |seen: &Seen| match seen.path() {
        "/pub/player/alice" => Reply::Json(200, profile_body()),
        "/pub/player/alice/games/archives" => Reply::Json(
            200,
            json!({ "archives": [format!("https://api.chess.com/pub/player/alice/games/{PAST}")] })
                .to_string(),
        ),
        path if path.ends_with(PAST) => Reply::Json(
            200,
            month_body(&[game(5, "Alice", "bob", "win", "resigned")]),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let profile = profile(&fixture);
    profile.chesscom.add("alice").await.expect("added");
    let queue = profile
        .database
        .call_async(
            "store.reviewStore.gamesToReview",
            &[json!(["Alice@chess.com"]), json!(0), json!(10)],
        )
        .await
        .unwrap();
    assert_eq!(queue[0]["id"], "cc-l5");
    assert_eq!(queue[0]["checked"], true, "{queue}");
}

/// A Chess.com game keeps the rating after it and no change: its history is those ratings, a
/// point a day per speed (Home's rating cards and chart for a Chess.com player).
#[tokio::test]
async fn chesscom_ratings_make_a_history_without_rating_changes() {
    let fixture = serve(move |seen: &Seen| match seen.path() {
        "/pub/player/alice" => Reply::Json(200, profile_body()),
        "/pub/player/alice/games/archives" => Reply::Json(
            200,
            json!({ "archives": [format!("https://api.chess.com/pub/player/alice/games/{PAST}")] })
                .to_string(),
        ),
        path if path.ends_with(PAST) => Reply::Json(
            200,
            month_body(&[
                game(5, "Alice", "bob", "win", "resigned"),
                game(6, "bob", "Alice", "win", "checkmated"),
            ]),
        ),
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let profile = profile(&fixture);
    profile.chesscom.add("alice").await.expect("added");
    let history = profile
        .database
        .call_async("store.games.gameRatingHistory", &[json!("alice@chess.com")])
        .await
        .unwrap();
    assert_eq!(
        history,
        json!([{ "name": "rapid", "points": [[2020, 0, 6, 1500], [2020, 0, 7, 1480]] }])
    );
}
