//! Accounts, profiles and game sync (`crates/kchess-node/js/lichess.ts`) over local fixtures. Ports the
//! sync and export-authentication scenarios of `tests/core/game-library.test.ts` (paging,
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
use kchess_core::lichess::accounts::{
    AccountRow, CachedValue, Lichess, LichessGame, LichessStore, Reply as Outcome,
};
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
    // A 401-downgraded sync yields public games only: the games are kept, but the
    // cursor is unchanged so the gap is backfilled after reconnect.
    assert!(store.cursors.lock().unwrap().is_empty());
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

#[tokio::test]
async fn a_tracked_player_moves_its_cursor_while_a_connected_account_without_its_login_does_not() {
    // Every sync of a player without a login counted as downgraded, so its cursor never moved
    // and each sync downloaded its whole history again.
    let fixture = serve(|_| {
        Reply::Text(
            200,
            "application/x-ndjson",
            ndjson(&page_of("Game", 1, 5_000_000)),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Bob", true), ("Carol", false)]);
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[]),
    );

    lichess.sync_games(None).await.unwrap();

    let cursors = store.cursors.lock().unwrap();
    assert!(cursors.contains_key("carol"));
    assert!(!cursors.contains_key("bob"));
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
async fn concurrent_profile_reads_share_one_request() {
    let fixture = serve(|_| {
        std::thread::sleep(Duration::from_millis(100));
        Reply::Json(200, r#"{"username":"Alice","title":"IM"}"#.into())
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );

    let (first, second) = tokio::join!(lichess.profile("Alice"), lichess.profile("Alice"));
    assert_eq!(first.unwrap()["title"], "IM");
    assert_eq!(second.unwrap()["title"], "IM");
    assert_eq!(fixture.to("/api/user/Alice").len(), 1);
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
    let cached = lichess.cached_profile("alice").await.unwrap();
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

/// A store that saves reviews only with their page of games: its `save_reviews` refuses, so a sync
/// that wrote reviews on its own would fail (`saveGamesPage` wrote both in one transaction).
struct PageOnly(Arc<MemoryStore>);

impl LichessStore for PageOnly {
    fn accounts<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Vec<AccountRow>>> {
        Box::pin(async move { self.0.accounts().await })
    }
    fn dismissed_friends<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Vec<String>>> {
        Box::pin(async move { self.0.dismissed_friends().await })
    }
    fn pending_game_ids<'a>(
        &'a self,
        account: &'a str,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Vec<String>>> {
        Box::pin(async move { self.0.pending_game_ids(account).await })
    }
    fn save_games_page<'a>(
        &'a self,
        account: &'a str,
        games: &'a [LichessGame],
        reviews: &'a [Value],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move {
            self.0
                .save_games_page(account, games, reviews, current)
                .await
        })
    }
    fn save_games_cursor<'a>(
        &'a self,
        account: &'a str,
        synced_at: i64,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move { self.0.save_games_cursor(account, synced_at, current).await })
    }
    fn read_api_cache<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Option<CachedValue>>> {
        Box::pin(async move { self.0.read_api_cache(key).await })
    }
    fn write_api_cache<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move { self.0.write_api_cache(key, value, current).await })
    }
    fn save_login<'a>(
        &'a self,
        username: &'a str,
        ciphertext: &'a str,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move { self.0.save_login(username, ciphertext, current).await })
    }
    fn save_reviews<'a>(
        &'a self,
        _account: &'a str,
        _reviews: &'a [Value],
        _current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Vec<Value>>> {
        Box::pin(async move {
            Err(kchess_core::error::CoreError::new(
                "Reviews are saved with their page.",
            ))
        })
    }
    fn mark_checked<'a>(
        &'a self,
        ids: &'a [String],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move { self.0.mark_checked(ids, current).await })
    }
}

/// One player of an analysed game, as Lichess exports it.
fn analysed_player(name: &str, accuracy: f64) -> Value {
    json!({
        "user": { "id": name, "name": name },
        "rating": 1500,
        "analysis": { "inaccuracy": 0, "mistake": 0, "blunder": 0, "acpl": 10, "accuracy": accuracy }
    })
}

/// A finished game with Lichess's analysis: its page of games carries a review.
fn analysed_game() -> Value {
    let mut game = game_json("abcdefgh", 1_000, "alice", "bob");
    game["players"] = json!({
        "white": analysed_player("alice", 95.0),
        "black": analysed_player("bob", 40.0),
    });
    game["moves"] = json!("e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#");
    game["status"] = json!("mate");
    game["variant"] = json!("standard");
    game["speed"] = json!("blitz");
    game["perf"] = json!("blitz");
    game["analysis"] = json!([
        { "eval": 30 },
        { "eval": 40 },
        { "eval": 20 },
        { "eval": 30 },
        { "eval": 50 },
        { "mate": 1, "best": "g7g6", "variation": "g6 Qf3 Nf6", "judgment": { "name": "Blunder", "comment": "g6 was best." } }
    ]);
    game
}

/// A sync saves a page's reviews with its games, never on their own.
#[tokio::test]
async fn a_synced_page_saves_its_reviews_with_its_games() {
    let fixture = serve(|seen| {
        if seen.query("until").is_none() {
            Reply::Text(200, "application/x-ndjson", ndjson(&[analysed_game()]))
        } else {
            Reply::Text(200, "application/x-ndjson", String::new())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let tokens = MemoryTokens::with(&[("Alice", "lip_alice")]);
    let client = LichessClient::with_base(
        &fixture.base,
        Policy::with_timeout(
            Arc::clone(&host) as Arc<dyn kchess_core::host::Host>,
            http_client(),
            CancellationToken::new(),
            Duration::from_secs(5),
        ),
        http_client(),
    );
    let lichess = Lichess::new(
        client,
        capabilities_for(&host),
        Arc::new(PageOnly(Arc::clone(&store))),
        tokens,
        Arc::clone(&host) as Arc<dyn kchess_core::host::Host>,
        CancellationToken::new(),
    );
    lichess.sync_games(Some("Alice")).await.unwrap();
    assert_eq!(store.games.lock().unwrap().len(), 1);
    let reviews = store.reviews.lock().unwrap().clone();
    assert_eq!(reviews.len(), 1, "the page's review is saved with the page");
    assert_eq!(reviews[0]["gameId"], "abcdefgh");
}

#[tokio::test]
async fn logout_during_a_cache_write_does_not_restore_the_memory_cache() {
    let fixture = serve(|_| Reply::Json(200, json!({ "username": "Alice" }).to_string())).await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    *store.cache_write_gate.lock().unwrap() = Some((started, held));
    let lichess = service(
        &fixture.base,
        &host,
        Arc::clone(&store),
        MemoryTokens::with(&[]),
    );
    let pending_lichess = Arc::clone(&lichess);
    let pending = tokio::spawn(async move { pending_lichess.profile("Alice").await });
    tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("cache write reached")
        .expect("started");
    lichess.invalidate_login(&["Alice".to_string()]);
    release.send(()).expect("release");
    assert_eq!(
        pending
            .await
            .expect("profile task")
            .expect_err("stale profile")
            .message,
        "Account was logged out."
    );
    assert!(store.cache.lock().unwrap().is_empty());
    lichess.profile("Alice").await.expect("fresh profile");
    assert_eq!(
        fixture.to("/api/user/Alice").len(),
        2,
        "the stale result was not remembered"
    );
}

// A puzzle's source game could not be opened without a connection, even after it had been.
#[tokio::test]
async fn a_game_exported_once_opens_again_without_a_connection() {
    let offline = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let down = Arc::clone(&offline);
    let fixture = serve(move |seen| {
        if down.load(std::sync::atomic::Ordering::SeqCst) {
            return Reply::Text(503, "text/plain", String::new());
        }
        match (seen.path(), seen.body.as_str()) {
            ("/api/games/export/_ids", "game0001") => Reply::Text(
                200,
                "application/x-chess-pgn",
                "[White \"Alice\"]\n[Black \"Bob\"]\n\n1. e4 e5 1-0\n".into(),
            ),
            _ => Reply::Text(200, "application/x-chess-pgn", String::new()),
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );
    let pgn = lichess.export_game("game0001").await.unwrap();
    offline.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(lichess.export_game("game0001").await.unwrap(), pgn);
    // A game never fetched still says why it cannot open.
    assert!(lichess.export_game("game0002").await.is_err());
}

#[tokio::test]
async fn a_game_is_exported_without_an_account_and_a_missing_one_is_named() {
    let fixture = serve(|seen| match (seen.path(), seen.body.as_str()) {
        ("/api/games/export/_ids", "game0001") => Reply::Text(
            200,
            "application/x-chess-pgn",
            "[White \"Alice\"]\n[Black \"Bob\"]\n\n1. e4 e5 1-0\n".into(),
        ),
        ("/api/games/export/_ids", _) => Reply::Text(200, "application/x-chess-pgn", String::new()),
        _ => Reply::WithHeaders(303, vec![("location", "/signup".into())], String::new()),
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    );
    let pgn = lichess.export_game("game0001").await.unwrap();
    assert!(pgn.contains("1. e4 e5"));
    let error = lichess.export_game("gone0001").await.unwrap_err();
    assert_eq!(error.message, "That game is not on Lichess.");
    let asked = fixture.requests();
    assert!(
        asked
            .iter()
            .all(|seen| seen.path() == "/api/games/export/_ids")
    );
    assert_eq!(asked[0].query("clocks").as_deref(), Some("true"));
    assert_eq!(asked[0].header("authorization"), None);
}
