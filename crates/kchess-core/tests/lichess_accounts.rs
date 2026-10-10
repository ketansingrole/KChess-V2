//! Accounts, profiles and game sync (`core/src/services/lichess.ts`) over local fixtures. Ports the
//! sync and export-authentication scenarios of `core/tests/unit/game-library.test.ts` (paging,
//! cursor, pending games, sync cancelled by logout or removal, token fallback), plus the
//! reconnect, stale-token, profile-cache and log-redaction rules.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, capabilities_for, game_json, http_client, ndjson,
    serve,
};
use kchess_core::lichess::accounts::{Lichess, Reply as Outcome};
use kchess_core::lichess::client::LichessClient;
use kchess_core::lichess::policy::Policy;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

fn service(
    base: &str,
    host: &Arc<Recording>,
    store: Arc<MemoryStore>,
    tokens: Arc<MemoryTokens>,
) -> Arc<Lichess> {
    let policy = Policy::with_timeout(
        Arc::clone(host) as Arc<dyn kchess_core::host::Host>,
        http_client(),
        CancellationToken::new(),
        Duration::from_secs(5),
    );
    let client = LichessClient::with_base(base, Arc::clone(&policy), http_client());
    Arc::new(Lichess::new(
        client,
        capabilities_for(host),
        store,
        tokens,
        Arc::clone(host) as Arc<dyn kchess_core::host::Host>,
        CancellationToken::new(),
    ))
}

fn page_of(prefix: &str, count: usize, first_created: i64) -> Vec<Value> {
    (0..count)
        .map(|i| {
            game_json(
                &format!("{prefix}{i:04}"),
                first_created - i as i64,
                "Alice",
                "Bob",
            )
        })
        .collect()
}

#[tokio::test]
async fn syncing_pages_backwards_and_saves_each_page() {
    let fixture = serve(|seen| {
        if seen.query("until").is_none() {
            Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&page_of("Newer", 1000, 10_000_000)),
            )
        } else {
            Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&page_of("Older", 5, 9_000_000)),
            )
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let tokens = MemoryTokens::with(&[("Alice", "lip_alice")]);
    let lichess = service(&fixture.base, &host, Arc::clone(&store), tokens);

    lichess.sync_games(Some("Alice")).await.unwrap();

    let pages = fixture.to("/api/games/user/Alice");
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].query("ongoing").as_deref(), Some("true"));
    assert_eq!(pages[0].query("max").as_deref(), Some("1000"));
    assert_eq!(pages[1].query("until").as_deref(), Some("9999001"));
    assert!(
        pages
            .iter()
            .all(|seen| seen.header("authorization") == Some("Bearer lip_alice"))
    );
    assert_eq!(store.games.lock().unwrap().len(), 1005);
    assert!(store.cursors.lock().unwrap().contains_key("alice"));
}

#[tokio::test]
async fn a_failed_export_does_not_move_the_cursor() {
    let fixture = serve(|_| Reply::Json(503, String::new())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    let error = lichess.sync_games(Some("Alice")).await.unwrap_err();
    assert_eq!(error.message, "Lichess 503 from GET /api/games/user/Alice");
    assert!(store.cursors.lock().unwrap().is_empty());
    assert!(store.games.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_rejected_token_falls_back_to_an_anonymous_export() {
    let fixture = serve(|seen| {
        if seen.header("authorization").is_some() {
            Reply::Json(401, r#"{"error":"No such token"}"#.into())
        } else {
            Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&page_of("Game", 1, 5_000_000)),
            )
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    lichess.sync_games(Some("Alice")).await.unwrap();

    let auths: Vec<Option<String>> = fixture
        .to("/api/games/user/Alice")
        .iter()
        .map(|seen| seen.header("authorization").map(str::to_string))
        .collect();
    assert_eq!(auths, vec![Some("Bearer lip_alice".to_string()), None]);
    assert_eq!(store.games.lock().unwrap().len(), 1);
    assert!(store.cursors.lock().unwrap().contains_key("alice"));
}

#[tokio::test]
async fn an_account_without_a_login_syncs_anonymously() {
    let fixture = serve(|_| Reply::Text(200, "application/x-ndjson", String::new())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true), ("Bob", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    lichess.sync_games(None).await.unwrap();

    let bob = fixture.to("/api/games/user/Bob");
    assert_eq!(bob.len(), 1);
    assert_eq!(bob[0].header("authorization"), None);
    let alice = fixture.to("/api/games/user/Alice");
    assert_eq!(alice[0].header("authorization"), Some("Bearer lip_alice"));
}

/// A sync whose download is interrupted by `interrupt`, run from inside the fixture (so the
/// interruption lands while the page is in flight).
async fn interrupted_sync(interrupt: fn(&Lichess)) -> (Arc<MemoryStore>, Result<(), String>) {
    let slot: Arc<OnceLock<Arc<Lichess>>> = Arc::new(OnceLock::new());
    let for_fixture = Arc::clone(&slot);
    let fixture = serve(move |seen| {
        if seen.path() == "/api/games/user/Alice" {
            if let Some(lichess) = for_fixture.get() {
                interrupt(lichess);
            }
            Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&page_of("Game", 1, 7_000_000)),
            )
        } else {
            Reply::Json(404, "{}".into())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );
    let _ = slot.set(Arc::clone(&lichess));
    let outcome = lichess
        .sync_games(Some("Alice"))
        .await
        .map_err(|error| error.message);
    (store, outcome)
}

#[tokio::test]
async fn a_logout_during_a_download_saves_nothing() {
    let (store, outcome) =
        interrupted_sync(|lichess| lichess.invalidate_login(&["Alice".to_string()])).await;
    assert_eq!(outcome.unwrap_err(), "Sync was cancelled.");
    assert!(store.games.lock().unwrap().is_empty());
    assert!(store.cursors.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_account_removed_during_a_download_is_not_saved() {
    let (store, outcome) =
        interrupted_sync(|lichess| lichess.cancel_account_syncs(&["Alice".to_string()])).await;
    assert_eq!(outcome.unwrap_err(), "Sync was cancelled.");
    assert!(store.games.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_second_sync_joins_the_running_one() {
    let (gate_open, gate) = watch::channel(false);
    let fixture = serve(move |_| {
        Reply::Gated(
            gate.clone(),
            Box::new(Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&page_of("Game", 2, 3_000_000)),
            )),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    let first = tokio::spawn({
        let lichess = Arc::clone(&lichess);
        async move { lichess.sync_games(Some("Alice")).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = tokio::spawn({
        let lichess = Arc::clone(&lichess);
        async move { lichess.sync_games(Some("Alice")).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate_open.send(true).unwrap();

    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(fixture.to("/api/games/user/Alice").len(), 1);
    assert_eq!(store.games.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_pending_game_is_fetched_by_id_and_saved_when_lichess_has_it() {
    let fixture = serve(|seen| {
        if seen.path() == "/api/games/export/_ids" {
            Reply::Text(
                200,
                "application/x-ndjson",
                ndjson(&[game_json("Pending1", 1_000, "Alice", "Bob")]),
            )
        } else {
            Reply::Text(200, "application/x-ndjson", String::new())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    store.pending.lock().unwrap().insert(
        "alice".to_string(),
        vec!["Pending1".to_string(), "Missing1".to_string()],
    );
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    lichess.sync_games(Some("Alice")).await.unwrap();

    let ids = fixture.to("/api/games/export/_ids");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].body, "Pending1,Missing1");
    let saved: Vec<String> = store
        .games
        .lock()
        .unwrap()
        .iter()
        .map(|game| game.id.clone())
        .collect();
    assert_eq!(saved, vec!["Pending1".to_string()]);
}

#[tokio::test]
async fn a_refused_permission_asks_the_user_to_reconnect() {
    let fixture = serve(|_| Reply::Json(403, r#"{"error":"Missing scope"}"#.into())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        store,
        MemoryTokens::with(&[("Alice", "lip_old")]),
    );

    let outcome = lichess.send_message("Alice", "bob", "hello").await.unwrap();
    assert_eq!(outcome, Outcome::NeedsReconnect);
    assert_eq!(fixture.to("/inbox/bob").len(), 1);
}

#[tokio::test]
async fn an_account_without_a_login_asks_to_reconnect_without_a_request() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(&fixture.base, &host, store, MemoryTokens::with(&[]));

    let outcome = lichess.send_message("Alice", "bob", "hello").await.unwrap();
    assert_eq!(outcome, Outcome::NeedsReconnect);
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn followed_users_report_the_accounts_that_need_attention() {
    let fixture = serve(|_| {
        Reply::Text(
            200,
            "application/x-ndjson",
            ndjson(&[json!({"username":"Carol","title":"IM","perfs":{"blitz":{"rating":2100}}})]),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true), ("Bob", true)]);
    store.dismissed.lock().unwrap().push("dave".to_string());
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    );

    let report = lichess.followed_users().await.unwrap();
    assert_eq!(report.users.len(), 1);
    assert_eq!(report.users[0].username, "Carol");
    assert_eq!(report.users[0].followed_by, vec!["Alice".to_string()]);
    assert_eq!(report.users[0].ratings.blitz, Some(2100));
    assert_eq!(report.problems.len(), 1);
    assert_eq!(report.problems[0].account, "Bob");
    assert_eq!(
        report.problems[0].message,
        "Its Lichess login is unavailable."
    );
    assert!(report.problems[0].needs_reconnect);
}

#[tokio::test]
async fn a_stale_login_for_followed_users_asks_to_reconnect() {
    let fixture = serve(|_| Reply::Json(401, r#"{"error":"Invalid token"}"#.into())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        store,
        MemoryTokens::with(&[("Alice", "lip_stale")]),
    );
    let report = lichess.followed_users().await.unwrap();
    assert!(report.users.is_empty());
    assert_eq!(report.problems.len(), 1);
    assert!(report.problems[0].needs_reconnect);
    assert_eq!(
        report.problems[0].message,
        "Lichess needs your permission to read who this account follows."
    );
}

#[tokio::test]
async fn a_missing_user_is_named_in_the_error() {
    let fixture = serve(|_| Reply::Text(404, "text/html", "<html>missing</html>".into())).await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );
    let error = lichess.profile("nobody").await.unwrap_err();
    assert_eq!(
        error.message,
        r#"Lichess 404 from GET /api/user/nobody: no such Lichess user "nobody""#
    );
}

#[tokio::test]
async fn a_profile_is_served_from_the_saved_copy_when_lichess_is_unreachable() {
    let fixture = serve(|_| Reply::Json(503, String::new())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[]);
    store.cache.lock().unwrap().insert(
        "alice:profile".to_string(),
        kchess_core::lichess::accounts::CachedValue {
            value: json!({"username": "Alice", "title": "IM"}),
            fetched_at: 42,
        },
    );
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[]),
    );

    let profile = lichess.profile("alice").await.unwrap();
    assert_eq!(profile["title"], "IM");
    let cached = lichess.cached_profile("alice").unwrap();
    assert_eq!(cached.profile_fetched_at, Some(42));
}

#[tokio::test]
async fn recent_games_are_newest_first_and_capped() {
    let fixture = serve(|_| {
        Reply::Text(
            200,
            "application/x-ndjson",
            ndjson(&page_of("Game", 12, 9_000_000)),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );
    let games = lichess.recent_games("Alice", false).await.unwrap();
    assert_eq!(games.len(), 10);
    assert_eq!(games[0].id, "Game0000");
    assert_eq!(games[0].color, "white");
    assert_eq!(
        fixture.to("/api/games/user/Alice")[0]
            .query("max")
            .as_deref(),
        Some("10")
    );
}

#[tokio::test]
async fn a_player_record_is_read_from_the_perf_endpoint() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({
                "perf": {"glicko": {"rating": 1612.5, "deviation": 60.0, "provisional": false}, "progress": 8},
                "rank": 1200,
                "percentile": 97.5,
                "stat": {
                    "count": {"all": 300},
                    "highest": {"int": 1700, "at": "2026-01-01", "gameId": "hi"},
                    "bestWins": {"results": [{"opRating": 1800, "opId": {"name": "Bob"}, "at": "2026-01-02", "gameId": "w1"}]},
                    "resultStreak": {"win": {"cur": {"v": 2}, "max": {"v": 5}}, "loss": {"cur": {"v": 0}, "max": {"v": 3}}}
                }
            })
            .to_string(),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );
    let stats = lichess.player_perf("Alice", "blitz").await.unwrap();
    assert_eq!(stats.rating, Some(1612.5));
    assert_eq!(stats.rank, Some(1200));
    assert_eq!(stats.highest.as_ref().map(|mark| mark.rating), Some(1700.0));
    assert_eq!(stats.best_wins[0].opponent, "Bob");
    assert_eq!(stats.win_streak.best, 5);
    assert_eq!(stats.loss_streak.best, 3);
}

#[tokio::test]
async fn tokens_never_reach_the_host_log() {
    let fixture = serve(|seen| {
        if seen.header("authorization").is_some() {
            Reply::Json(401, r#"{"error":"No such token"}"#.into())
        } else {
            Reply::Json(503, String::new())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let lichess = service(
        &fixture.base,
        &host,
        store,
        MemoryTokens::with(&[("Alice", "lip_super_secret_token")]),
    );
    let _ = lichess.sync_games(Some("Alice")).await;
    let logged = host.logged();
    assert!(logged.contains("Game sync failed: Alice"));
    assert!(!logged.contains("lip_super_secret_token"));
    assert!(!logged.contains("Bearer"));
}
