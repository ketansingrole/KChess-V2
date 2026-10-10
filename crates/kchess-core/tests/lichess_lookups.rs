//! Position explorer and cloud evaluation (`crates/kchess-node/js/positionLookup.ts` and `cloudEval.ts`)
//! over local fixtures. Ports `tests/core/position-lookup.test.ts` and the explorer scenarios
//! of `tests/core/watch-and-lookup.test.ts`, plus the cloud evaluation rules.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, Seen, http_client, lichess_service, serve,
};
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::lookups::{
    CloudCache, Endpoints, LookupCache, PositionLookups, cloud_eval,
};
use serde_json::{Value, json};
use tokio::sync::watch;

const INITIAL: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Default)]
struct MemoryCache {
    values: Mutex<HashMap<String, Value>>,
}

impl LookupCache for MemoryCache {
    fn read(&self, key: &str) -> kchess_core::error::Result<Option<Value>> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    fn write(&self, key: &str, value: &Value) -> kchess_core::error::Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.clone());
        Ok(())
    }
}

/// A lookup service whose endpoints are the fixture, with a settable clock.
struct Lookups {
    service: PositionLookups,
    cache: Arc<MemoryCache>,
    clock: Arc<AtomicI64>,
}

fn lookups(base: &str, start: i64) -> Lookups {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(start));
    let endpoints = Endpoints {
        opening: format!("{base}/lichess"),
        masters: format!("{base}/masters"),
        player: format!("{base}/player"),
        tablebase: format!("{base}/standard"),
        masters_game: format!("{base}/masters/pgn"),
    };
    let now_clock = Arc::clone(&clock);
    let service = PositionLookups::new(
        http_client(),
        endpoints,
        Arc::clone(&cache) as Arc<dyn LookupCache>,
        Arc::new(move || now_clock.load(Ordering::SeqCst)),
    );
    Lookups {
        service,
        cache,
        clock,
    }
}

fn opening_body() -> Value {
    json!({ "moves": [{ "uci": "e2e4", "white": 100, "draws": 20, "black": 50 }] })
}

#[tokio::test]
async fn deduplicates_lookups_validates_legal_san_and_reuses_the_saved_result() {
    let (gate_open, gate) = watch::channel(false);
    let fixture = serve(move |_| {
        Reply::Gated(
            gate.clone(),
            Box::new(Reply::Json(200, opening_body().to_string())),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let service = Arc::new(lookups.service);
    let kind = json!("opening");
    let fen = json!(INITIAL);
    let first = {
        let (service, kind, fen) = (Arc::clone(&service), kind.clone(), fen.clone());
        tokio::spawn(async move { service.lookup(&kind, &fen, None).await })
    };
    wait_for_requests(&fixture, 1).await;
    let second = {
        let (service, kind, fen) = (Arc::clone(&service), kind.clone(), fen.clone());
        tokio::spawn(async move { service.lookup(&kind, &fen, None).await })
    };
    // The second caller joins the pending lookup: still one request.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.requests().len(), 1);
    gate_open.send(true).unwrap();
    let one = first.await.unwrap().unwrap();
    let two = second.await.unwrap().unwrap();
    assert_eq!(one.data.moves[0].san, "e4");
    assert_eq!(two, one);
    let again = service.lookup(&kind, &fen, None).await.unwrap();
    assert!(again.cached);
    assert_eq!(fixture.requests().len(), 1);
}

async fn wait_for_requests(fixture: &fixtures::Fixture, count: usize) {
    for _ in 0..200 {
        if fixture.requests().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the lookup never reached the fixture");
}

#[tokio::test]
async fn shows_a_saved_result_as_stale_after_a_failed_refresh() {
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        if calls == 1 {
            Reply::Json(200, opening_body().to_string())
        } else {
            Reply::Json(500, "{}".into())
        }
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    lookups
        .service
        .lookup(&json!("opening"), &json!(INITIAL), None)
        .await
        .unwrap();
    lookups
        .clock
        .store(1_000 + 2 * 86_400_000, Ordering::SeqCst);
    let result = lookups
        .service
        .lookup(&json!("opening"), &json!(INITIAL), None)
        .await
        .unwrap();
    assert!(result.stale && result.cached);
    assert!(result.message.unwrap().contains("saved"));
}

#[tokio::test]
async fn rejects_malformed_and_illegal_remote_moves() {
    let bodies = [
        json!({ "moves": [{ "uci": "e2e5", "white": 10, "draws": 0, "black": 1 }] }),
        json!({ "moves": [{ "uci": "e2e4", "white": -10, "draws": 0, "black": 1 }] }),
        json!({ "moves": vec![json!({ "uci": "e2e4", "white": 1, "draws": 0, "black": 0 }); 257] }),
    ];
    for body in bodies {
        let fixture = serve(move |_| Reply::Json(200, body.to_string())).await;
        let lookups = lookups(&fixture.base, 1_000);
        let error = lookups
            .service
            .lookup(&json!("opening"), &json!(INITIAL), None)
            .await
            .unwrap_err();
        assert!(!error.message.is_empty());
    }
}

#[tokio::test]
async fn bounds_response_size_and_rejects_unsupported_tablebase_positions_before_fetching() {
    let fixture = serve(|_| Reply::Text(200, "text/plain", "x".repeat(512_001))).await;
    let lookups = lookups(&fixture.base, 1_000);
    let error = lookups
        .service
        .lookup(&json!("tablebase"), &json!(INITIAL), None)
        .await
        .unwrap_err();
    assert!(error.message.contains("seven pieces"));
    assert!(fixture.requests().is_empty());
    let error = lookups
        .service
        .lookup(&json!("opening"), &json!(INITIAL), None)
        .await
        .unwrap_err();
    assert!(error.message.contains("too much data"));
}

#[tokio::test]
async fn retains_tablebase_fifty_move_categories_and_dtz_without_inventing_a_mate_distance() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "category": "cursed-win", "dtz": 101, "moves": [] }).to_string(),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let result = lookups
        .service
        .lookup(
            &json!("tablebase"),
            &json!("8/8/8/8/8/8/R7/K6k w - - 0 1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.data.category.as_deref(), Some("cursed-win"));
    assert_eq!(result.data.dtz, Some(Some(101)));
}

#[tokio::test]
async fn loads_a_master_game_and_refuses_a_refused_failed_or_non_pgn_answer() {
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        match calls {
            1 => Reply::Text(200, "text/plain", "[Event \"x\"]\n1. e4 *".into()),
            2 => Reply::Json(401, String::new()),
            _ => Reply::Json(500, String::new()),
        }
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    assert!(
        lookups
            .service
            .masters_game("abcdefgh")
            .await
            .unwrap()
            .contains("[Event")
    );
    let error = lookups.service.masters_game("abcdefgh").await.unwrap_err();
    assert!(error.message.contains("Connect a Lichess account"));
    let error = lookups.service.masters_game("abcdefgh").await.unwrap_err();
    assert!(
        error.message.contains("could not be loaded (500)") || error.message.contains("Connect")
    );
    assert!(lookups.service.masters_game("not an id").await.is_err());
}

#[tokio::test]
async fn masters_game_refuses_a_body_that_is_not_a_pgn() {
    let fixture = serve(|_| Reply::Text(200, "text/plain", "no headers here".into())).await;
    let lookups = lookups(&fixture.base, 1_000);
    let error = lookups.service.masters_game("abcdefgh").await.unwrap_err();
    assert!(error.message.contains("not a PGN"));
}

#[tokio::test]
async fn refuses_a_tablebase_lookup_for_a_position_with_more_than_seven_pieces() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let lookups = lookups(&fixture.base, 1_000);
    let error = lookups
        .service
        .lookup(&json!("tablebase"), &json!(INITIAL), None)
        .await
        .unwrap_err();
    assert!(error.message.contains("seven pieces"));
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn saves_a_tablebase_answer_and_serves_it_again_while_fresh() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "category": "unknown", "dtz": null, "moves": [] }).to_string(),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let kings = json!("7k/8/8/8/8/8/8/K7 w - - 0 1");
    let first = lookups
        .service
        .lookup(&json!("tablebase"), &kings, None)
        .await
        .unwrap();
    assert_eq!(first.data.kind, "tablebase");
    assert_eq!(first.data.category.as_deref(), Some("unknown"));
    assert!(!first.cached);
    assert_eq!(lookups.cache.values.lock().unwrap().len(), 1);
    let again = lookups
        .service
        .lookup(&json!("tablebase"), &kings, None)
        .await
        .unwrap();
    assert!(again.cached && !again.stale);
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn asks_the_masters_database_and_lists_its_top_games() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({
                "white": 10,
                "draws": 5,
                "black": 5,
                "moves": [{ "uci": "e2e4", "white": 10, "draws": 5, "black": 5 }],
                "topGames": [{
                    "id": "AbCdEfGh",
                    "winner": "white",
                    "white": { "name": "Carlsen", "rating": 2850 },
                    "black": { "name": "Caruana", "rating": 2800 },
                    "year": 2019,
                    "uci": "e2e4"
                }]
            })
            .to_string(),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let result = lookups
        .service
        .lookup(
            &json!("masters"),
            &json!(INITIAL),
            Some(&json!({ "since": "2000-01" })),
        )
        .await
        .unwrap();
    let request = &fixture.requests()[0];
    assert!(request.target.contains("/masters"));
    assert_eq!(request.query("since").as_deref(), Some("2000"));
    assert_eq!(result.data.total, Some(20));
    let game = &result.data.games.as_ref().unwrap()[0];
    assert_eq!(game.white, "Carlsen");
    assert_eq!(game.san.as_deref(), Some("e4"));
    assert_eq!(game.winner.as_deref(), Some("white"));
}

#[tokio::test]
async fn reads_the_last_snapshot_of_the_player_database_and_keys_its_cache_by_filters() {
    let snapshot =
        |n: i64| json!({ "moves": [{ "uci": "d2d4", "white": n, "draws": 0, "black": 0 }] });
    let fixture = serve(move |seen: &Seen| {
        // The player database streams snapshots; the last one is the answer.
        let n = if seen.query("player").as_deref() == Some("Bob") {
            3
        } else {
            7
        };
        Reply::Text(
            200,
            "application/x-ndjson",
            format!("{}\n{}\n", snapshot(1), snapshot(n)),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let result = lookups
        .service
        .lookup(
            &json!("player"),
            &json!(INITIAL),
            Some(&json!({ "player": "Alice", "color": "black" })),
        )
        .await
        .unwrap();
    assert_eq!(result.data.moves[0].white, Some(7));
    let url = &fixture.requests()[0];
    assert_eq!(url.query("player").as_deref(), Some("Alice"));
    assert_eq!(url.query("color").as_deref(), Some("black"));
    lookups
        .service
        .lookup(
            &json!("player"),
            &json!(INITIAL),
            Some(&json!({ "player": "Bob", "color": "black" })),
        )
        .await
        .unwrap();
    assert_eq!(fixture.requests().len(), 2);
    assert_eq!(lookups.cache.values.lock().unwrap().len(), 2);
    let error = lookups
        .service
        .lookup(&json!("player"), &json!(INITIAL), Some(&json!({})))
        .await
        .unwrap_err();
    assert!(error.message.contains("whose games"));
}

#[tokio::test]
async fn rejects_unknown_filters() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let lookups = lookups(&fixture.base, 1_000);
    let result = lookups
        .service
        .lookup(
            &json!("opening"),
            &json!(INITIAL),
            Some(&json!({ "ratings": [1234] })),
        )
        .await;
    assert!(result.is_err());
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn cloud_evaluation_keeps_legal_lines_and_remembers_unknown_positions() {
    let fixture = serve(|seen: &Seen| {
        // The query text is not decoded, so the FEN's letters identify the position.
        if seen.target.contains("rnbqkbnr") {
            Reply::Json(
                200,
                json!({
                    "depth": 40,
                    "knodes": 900,
                    "pvs": [
                        { "cp": 25, "moves": "e2e4 e7e5 g1f3" },
                        { "mate": 3, "moves": "e2e5" }
                    ]
                })
                .to_string(),
            )
        } else {
            Reply::Json(404, "{}".into())
        }
    })
    .await;
    let lichess = service(&fixture.base, &Arc::new(Recording::default()));
    let cache = CloudCache::default();
    let found = cloud_eval(&lichess, &cache, INITIAL, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.lines.len(), 1);
    assert_eq!(found.lines[0].rank, 1);
    assert_eq!(found.lines[0].pv, ["e2e4", "e7e5", "g1f3"]);
    let missing = "8/8/8/8/8/8/8/K6k w - - 0 1";
    assert_eq!(
        cloud_eval(&lichess, &cache, missing, 1).await.unwrap(),
        None
    );
    assert_eq!(
        cloud_eval(&lichess, &cache, missing, 1).await.unwrap(),
        None
    );
    assert_eq!(fixture.to("/api/cloud-eval").len(), 2);
}

fn service(base: &str, host: &Arc<Recording>) -> Arc<Lichess> {
    lichess_service(
        base,
        host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    )
}

/// The explorer's login: the token of the account it is set to use, or none.
struct FixedLogin(Option<String>);

impl kchess_core::lichess::lookups::ExplorerLogin for FixedLogin {
    fn token(
        &self,
    ) -> futures_util::future::BoxFuture<'_, kchess_core::error::Result<Option<String>>> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

/// The opening explorer is asked with the account's token; without one it is not asked at all
/// (`setupPositionLookup.ts`), and the tablebase never carries a login.
#[tokio::test]
async fn the_explorer_is_asked_with_the_accounts_login_and_the_tablebase_never_is() {
    let fixture = serve(|_| Reply::Json(200, opening_body().to_string())).await;
    let with_login = lookups(&fixture.base, 1_000)
        .service
        .with_login(Arc::new(FixedLogin(Some("lip_alice".into()))));
    with_login
        .lookup(
            &json!("opening"),
            &json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"),
            None,
        )
        .await
        .expect("the explorer answers");
    let sent = fixture.requests();
    assert_eq!(sent[0].header("authorization"), Some("Bearer lip_alice"));

    let fixture = serve(|_| Reply::Json(200, opening_body().to_string())).await;
    let signed_out = lookups(&fixture.base, 1_000)
        .service
        .with_login(Arc::new(FixedLogin(None)));
    let error = signed_out
        .lookup(
            &json!("opening"),
            &json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "Connect a Lichess account in Settings to use the opening explorer."
    );
    assert!(
        fixture.requests().is_empty(),
        "no request is sent without a login"
    );

    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "category": "win", "dtz": 1, "moves": [] }).to_string(),
        )
    })
    .await;
    let tablebase = lookups(&fixture.base, 1_000)
        .service
        .with_login(Arc::new(FixedLogin(Some("lip_alice".into()))));
    tablebase
        .lookup(
            &json!("tablebase"),
            &json!("8/8/8/8/8/8/8/k3K2R w - - 0 1"),
            None,
        )
        .await
        .expect("the tablebase answers");
    assert_eq!(fixture.requests()[0].header("authorization"), None);
}
