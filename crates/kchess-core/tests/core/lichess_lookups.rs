//! Position explorer and cloud evaluation
//! over local fixtures: the lookup scenarios, the explorer and the cloud evaluation rules.

use crate::fixtures;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, Seen, http_client, lichess_service, serve,
};
use kchess_contracts::types::{LookupOptions, PositionLookupKind as Kind};
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::lookups::{
    CloudCache, Endpoints, LOOKUP_TIMEOUT, LookupCache, PositionLookups, cloud_eval,
};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const INITIAL: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Default)]
struct MemoryCache {
    values: Mutex<HashMap<String, Value>>,
}

impl LookupCache for MemoryCache {
    fn read<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<Option<Value>>> {
        Box::pin(async move { Ok(self.values.lock().unwrap().get(key).cloned()) })
    }

    fn write<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
    ) -> futures_util::future::BoxFuture<'a, kchess_core::error::Result<()>> {
        Box::pin(async move {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), value.clone());
            Ok(())
        })
    }
}

/// A lookup service whose endpoints are the fixture, with a settable clock.
struct Lookups {
    service: PositionLookups,
    cache: Arc<MemoryCache>,
    clock: Arc<AtomicI64>,
}

fn lookups(base: &str, start: i64) -> Lookups {
    lookups_with(base, start, CancellationToken::new(), LOOKUP_TIMEOUT)
}

/// `lookups` ended by `lifetime`, with each answer given at most `timeout`.
fn lookups_with(base: &str, start: i64, lifetime: CancellationToken, timeout: Duration) -> Lookups {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(AtomicI64::new(start));
    let endpoints = Endpoints {
        explorer: base.to_string(),
        tablebase: base.to_string(),
        wikibooks: format!("{base}/wikibooks/w/api.php"),
    };
    let now_clock = Arc::clone(&clock);
    let service = PositionLookups::new(
        Arc::new(Recording::default()),
        http_client(),
        endpoints,
        Arc::clone(&cache) as Arc<dyn LookupCache>,
        Arc::new(move || now_clock.load(Ordering::SeqCst)),
        lifetime,
        timeout,
    );
    Lookups {
        service,
        cache,
        clock,
    }
}

/// Explorer filters as a frontend sends them.
fn options(sent: Value) -> LookupOptions {
    serde_json::from_value(sent).expect("filters")
}

/// A line of UCI moves, as the opening pages ask for it.
fn line(moves: &[&str]) -> Vec<String> {
    moves.iter().map(|uci| (*uci).to_string()).collect()
}

fn opening_body() -> Value {
    json!({ "moves": [{ "uci": "e2e4", "white": 100, "draws": 20, "black": 50 }] })
}

#[tokio::test]
async fn lifetime_cancellation_ends_pending_and_deduplicated_lookups_without_stale_fallback() {
    let (gate_open, gate) = watch::channel(false);
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        let reply = Reply::Json(200, opening_body().to_string());
        if calls == 1 {
            reply
        } else {
            Reply::Gated(gate.clone(), Box::new(reply))
        }
    })
    .await;
    let lifetime = CancellationToken::new();
    let lookups = lookups_with(&fixture.base, 1_000, lifetime.clone(), LOOKUP_TIMEOUT);
    let service = Arc::new(lookups.service);
    service.lookup(Kind::Opening, INITIAL, None).await.unwrap();
    lookups.clock.store(100_000_000, Ordering::SeqCst);
    let first = {
        let service = Arc::clone(&service);
        tokio::spawn(async move { service.lookup(Kind::Opening, INITIAL, None).await })
    };
    wait_for_requests(&fixture, 2).await;
    let kind = Kind::Opening;
    let fen = INITIAL;
    let second = service.lookup(kind, fen, None);
    tokio::pin!(second);
    assert!(futures_util::poll!(&mut second).is_pending());
    lifetime.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .aborted
    );
    assert!(second.await.unwrap_err().aborted);
    assert!(
        service
            .lookup(Kind::Opening, INITIAL, None)
            .await
            .unwrap_err()
            .aborted
    );
    gate_open.send(true).unwrap();
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn dropping_the_producer_releases_its_pending_key_for_retry() {
    let fixture = serve(|_| Reply::Hang).await;
    let service = lookups(&fixture.base, 1_000).service;
    let kind = Kind::Opening;
    let fen = INITIAL;
    {
        let first = service.lookup(kind, fen, None);
        tokio::pin!(first);
        tokio::select! { biased; _ = &mut first => panic!("fixture hangs"), _ = wait_for_requests(&fixture, 1) => {} }
    }
    let retry = service.lookup(kind, fen, None);
    tokio::pin!(retry);
    tokio::select! { biased; _ = &mut retry => panic!("fixture hangs"), _ = wait_for_requests(&fixture, 2) => {} }
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn lifetime_cancellation_also_stops_master_game_downloads() {
    let fixture = serve(|_| Reply::Hang).await;
    let lifetime = CancellationToken::new();
    let service = lookups_with(&fixture.base, 1_000, lifetime.clone(), LOOKUP_TIMEOUT).service;
    let download = service.masters_game("Game0001");
    tokio::pin!(download);
    tokio::select! { biased; _ = &mut download => panic!("fixture hangs"), _ = wait_for_requests(&fixture, 1) => {} }
    lifetime.cancel();
    assert!(download.await.unwrap_err().aborted);
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
    let kind = Kind::Opening;
    let fen = INITIAL;
    let first = {
        let (service, kind, fen) = (Arc::clone(&service), kind, fen);
        tokio::spawn(async move { service.lookup(kind, fen, None).await })
    };
    wait_for_requests(&fixture, 1).await;
    let second = {
        let (service, kind, fen) = (Arc::clone(&service), kind, fen);
        tokio::spawn(async move { service.lookup(kind, fen, None).await })
    };
    // The second caller joins the pending lookup: still one request.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.requests().len(), 1);
    gate_open.send(true).unwrap();
    let one = first.await.unwrap().unwrap();
    let two = second.await.unwrap().unwrap();
    assert_eq!(one.saved.moves[0].san, "e4");
    assert_eq!(two, one);
    crate::support::wire::assert_wire("positionLookup.opening", &one);
    let again = service.lookup(kind, fen, None).await.unwrap();
    assert!(again.cached);
    crate::support::wire::assert_wire("positionLookup.opening-cached", &again);
    assert_eq!(fixture.requests().len(), 1);
}

async fn wait_for_requests(fixture: &fixtures::Fixture, count: usize) {
    fixture
        .wait_for(&format!("{count} lookup request(s)"), |requests| {
            requests.len() >= count
        })
        .await;
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
        .lookup(Kind::Opening, INITIAL, None)
        .await
        .unwrap();
    lookups
        .clock
        .store(1_000 + 2 * 86_400_000, Ordering::SeqCst);
    let result = lookups
        .service
        .lookup(Kind::Opening, INITIAL, None)
        .await
        .unwrap();
    assert!(result.stale && result.cached);
    crate::support::wire::assert_wire("positionLookup.stale", &result);
    assert!(result.message.unwrap().contains("saved"));
}

#[tokio::test]
async fn a_429_pauses_every_explorer_request_until_its_retry_after_ends() {
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        if calls == 1 {
            Reply::WithHeaders(429, vec![("retry-after", "30".into())], "{}".into())
        } else {
            Reply::Json(200, opening_body().to_string())
        }
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let kind = Kind::Opening;
    let fen = INITIAL;
    let limited = lookups.service.lookup(kind, fen, None).await.unwrap_err();
    assert!(
        limited.message.contains("30 seconds"),
        "{}",
        limited.message
    );
    // Neither the lookup nor the opening page's history asks again during the cooldown.
    lookups.clock.store(21_000, Ordering::SeqCst);
    let waiting = lookups.service.lookup(kind, fen, None).await.unwrap_err();
    assert!(
        waiting.message.contains("10 seconds"),
        "{}",
        waiting.message
    );
    assert!(
        lookups
            .service
            .opening_popularity(&line(&["e2e4"]))
            .await
            .is_err()
    );
    assert_eq!(fixture.requests().len(), 1);
    lookups.clock.store(31_000, Ordering::SeqCst);
    let result = lookups.service.lookup(kind, fen, None).await.unwrap();
    assert!(!result.cached);
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn sibling_openings_reuse_their_shared_positions_history() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "history": [{ "month": "2024-01", "white": 400, "draws": 200, "black": 400 }] })
                .to_string(),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let king = lookups
        .service
        .opening_popularity(&line(&["e2e4", "e7e5", "f2f4"]))
        .await
        .unwrap();
    assert_eq!(king.len(), 1);
    crate::support::wire::assert_wire("openingPopularity.month", &king);
    // The Vienna shares 1. e4 e5 with the King's Gambit: only its own position is asked.
    lookups
        .service
        .opening_popularity(&line(&["e2e4", "e7e5", "b1c3"]))
        .await
        .unwrap();
    assert_eq!(fixture.to("/lichess/history").len(), 3);
}

#[tokio::test]
async fn a_saved_lookup_is_shown_without_asking_during_a_cooldown() {
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        if calls == 1 {
            Reply::Json(200, opening_body().to_string())
        } else {
            Reply::Json(429, "{}".into())
        }
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let kind = Kind::Opening;
    let fen = INITIAL;
    lookups.service.lookup(kind, fen, None).await.unwrap();
    lookups
        .clock
        .store(1_000 + 2 * 86_400_000, Ordering::SeqCst);
    for _ in 0..3 {
        let result = lookups.service.lookup(kind, fen, None).await.unwrap();
        assert!(result.stale && result.cached);
    }
    // Without Retry-After the cooldown is a minute; only the first refresh was sent.
    assert_eq!(fixture.requests().len(), 2);
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
            .lookup(Kind::Opening, INITIAL, None)
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
        .lookup(Kind::Tablebase, INITIAL, None)
        .await
        .unwrap_err();
    assert!(error.message.contains("seven pieces"));
    assert!(fixture.requests().is_empty());
    let error = lookups
        .service
        .lookup(Kind::Opening, INITIAL, None)
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
        .lookup(Kind::Tablebase, "8/8/8/8/8/8/R7/K6k w - - 0 1", None)
        .await
        .unwrap();
    assert_eq!(result.saved.category.as_deref(), Some("cursed-win"));
    crate::support::wire::assert_wire("positionLookup.tablebase", &result);
    assert_eq!(result.saved.dtz, Some(Some(101)));
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
    // A server error says what failed, never Lichess's status.
    assert_eq!(error.message, "The master game could not be loaded.");
    assert_eq!(error.kind, Some(kchess_core::ErrorKind::Unavailable));
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
        .lookup(Kind::Tablebase, INITIAL, None)
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
    let kings = "7k/8/8/8/8/8/8/K7 w - - 0 1";
    let first = lookups
        .service
        .lookup(Kind::Tablebase, kings, None)
        .await
        .unwrap();
    assert_eq!(first.saved.kind, Kind::Tablebase);
    assert_eq!(first.saved.category.as_deref(), Some("unknown"));
    assert!(!first.cached);
    assert_eq!(lookups.cache.values.lock().unwrap().len(), 1);
    let again = lookups
        .service
        .lookup(Kind::Tablebase, kings, None)
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
            Kind::Masters,
            INITIAL,
            Some(&options(json!({ "since": "2000-01" }))),
        )
        .await
        .unwrap();
    let request = &fixture.requests()[0];
    assert!(request.target.contains("/masters"));
    assert_eq!(request.query("since").as_deref(), Some("2000"));
    assert_eq!(result.saved.total, Some(20));
    crate::support::wire::assert_wire("positionLookup.masters", &result);
    let game = &result.saved.games.as_ref().unwrap()[0];
    assert_eq!(game.white, "Carlsen");
    assert_eq!(game.san.as_deref(), Some("e4"));
    assert_eq!(game.winner, Some(kchess_domain::color::Color::White));
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
            Kind::Player,
            INITIAL,
            Some(&options(json!({ "player": "Alice", "color": "black" }))),
        )
        .await
        .unwrap();
    assert_eq!(result.saved.moves[0].white, Some(7));
    crate::support::wire::assert_wire("positionLookup.player", &result);
    let url = &fixture.requests()[0];
    assert_eq!(url.query("player").as_deref(), Some("Alice"));
    assert_eq!(url.query("color").as_deref(), Some("black"));
    // `recentGames=0&recentGames=8` made Lichess read 0: no recent games for the player.
    assert_eq!(url.target.matches("recentGames=").count(), 1);
    assert_eq!(url.query("recentGames").as_deref(), Some("8"));
    lookups
        .service
        .lookup(
            Kind::Player,
            INITIAL,
            Some(&options(json!({ "player": "Bob", "color": "black" }))),
        )
        .await
        .unwrap();
    assert_eq!(fixture.requests().len(), 2);
    assert_eq!(lookups.cache.values.lock().unwrap().len(), 2);
    let saved: std::collections::BTreeMap<String, Value> = lookups
        .cache
        .values
        .lock()
        .unwrap()
        .clone()
        .into_iter()
        .collect();
    crate::support::wire::assert_wire("positionLookup.player-saved", &saved);
    let error = lookups
        .service
        .lookup(Kind::Player, INITIAL, Some(&options(json!({}))))
        .await
        .unwrap_err();
    assert!(error.message.contains("whose games"));
}

/// The Lichess database answers for every variant: a Crazyhouse position is asked in its
/// variant, its drops are checked by Crazyhouse rules, and it is cached apart from standard chess.
#[tokio::test]
async fn asks_the_lichess_database_in_a_variant_and_checks_its_moves_by_that_variant() {
    const ZH: &str = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[P] w KQkq - 0 3";
    let fixture = serve(|seen: &Seen| {
        let body = if seen.query("variant").as_deref() == Some("crazyhouse") {
            json!({
                "moves": [
                    { "uci": "P@e4", "white": 5, "draws": 0, "black": 1 },
                    { "uci": "b1c3", "white": 9, "draws": 1, "black": 3 },
                ],
                "recentGames": [],
            })
        } else {
            opening_body()
        };
        Reply::Json(200, body.to_string())
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let zh = lookups
        .service
        .lookup(
            Kind::Opening,
            ZH,
            Some(&options(json!({ "variant": "crazyhouse" }))),
        )
        .await
        .unwrap();
    let sans: Vec<&str> = zh
        .saved
        .moves
        .iter()
        .map(|entry| entry.san.as_str())
        .collect();
    assert_eq!(sans, ["P@e4", "Nc3"]);
    crate::support::wire::assert_wire("positionLookup.crazyhouse", &zh);
    let url = &fixture.requests()[0];
    assert_eq!(url.query("variant").as_deref(), Some("crazyhouse"));
    // Read again from the cache, under its own key.
    let again = lookups
        .service
        .lookup(
            Kind::Opening,
            ZH,
            Some(&options(json!({ "variant": "crazyhouse" }))),
        )
        .await
        .unwrap();
    assert!(again.cached);
    let standard = lookups
        .service
        .lookup(
            Kind::Opening,
            INITIAL,
            Some(&options(json!({ "variant": "standard" }))),
        )
        .await
        .unwrap();
    assert_eq!(standard.saved.moves[0].san, "e4");
    assert_eq!(
        fixture.requests()[1].query("variant").as_deref(),
        Some("standard")
    );
    let keys: Vec<String> = lookups
        .cache
        .values
        .lock()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert!(keys.contains(&format!("opening:{INITIAL}")), "{keys:?}");
    let saved: std::collections::BTreeMap<String, Value> = lookups
        .cache
        .values
        .lock()
        .unwrap()
        .clone()
        .into_iter()
        .collect();
    crate::support::wire::assert_wire("positionLookup.variant-saved", &saved);
    assert!(
        keys.iter()
            .any(|key| key.contains("\"variant\":\"crazyhouse\"")),
        "{keys:?}"
    );
    // Masters games and tablebases are standard chess only, and nothing is asked for them.
    let masters = lookups
        .service
        .lookup(
            Kind::Masters,
            ZH,
            Some(&options(json!({ "variant": "crazyhouse" }))),
        )
        .await
        .unwrap_err();
    assert!(
        masters.message.contains("standard chess only"),
        "{}",
        masters.message
    );
    assert_eq!(fixture.requests().len(), 2);
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
    crate::support::wire::assert_wire("cloudEval.lines", &found);
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

/// The opening explorer is asked with the account's token; without one it is not asked at all,
/// and the tablebase never carries a login.
#[tokio::test]
async fn the_explorer_is_asked_with_the_accounts_login_and_the_tablebase_never_is() {
    let fixture = serve(|_| Reply::Json(200, opening_body().to_string())).await;
    let with_login = lookups(&fixture.base, 1_000)
        .service
        .with_login(Arc::new(FixedLogin(Some("lip_alice".into()))));
    with_login
        .lookup(
            Kind::Opening,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
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
            Kind::Opening,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
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
        .lookup(Kind::Tablebase, "8/8/8/8/8/8/8/k3K2R w - - 0 1", None)
        .await
        .expect("the tablebase answers");
    assert_eq!(fixture.requests()[0].header("authorization"), None);
}

/// A failed lookup showed `Position lookup returned 500. Try again shortly.`: the person is told
/// what failed, never Lichess's status.
#[tokio::test]
async fn a_failed_lookup_shows_no_status() {
    let fixture = serve(|_| Reply::Json(500, "{}".into())).await;
    let lookups = lookups(&fixture.base, 1_000);
    for kind in [Kind::Opening, Kind::Masters] {
        let error = lookups
            .service
            .lookup(kind, INITIAL, None)
            .await
            .unwrap_err();
        assert!(!error.message.contains("500"), "{}", error.message);
        assert_eq!(error.kind, Some(kchess_core::ErrorKind::Unavailable));
    }
    let error = lookups
        .service
        .opening_popularity(&line(&["e2e4"]))
        .await
        .unwrap_err();
    assert!(!error.message.contains("500"), "{}", error.message);
}

/// A lookup that never answers said "The operation was aborted due to timeout" with no kind, so
/// it was not offered a retry.
#[tokio::test]
async fn a_lookup_that_never_answers_times_out_as_a_timeout() {
    let fixture = serve(|_| Reply::Hang).await;
    let service = lookups_with(
        &fixture.base,
        1_000,
        CancellationToken::new(),
        Duration::from_millis(100),
    )
    .service;
    let error = service
        .lookup(Kind::Opening, INITIAL, None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, Some(kchess_core::ErrorKind::TimedOut));
    assert!(!error.message.contains("aborted"), "{}", error.message);
}

/// Wikibooks has nothing to do with Lichess: its refusal asked to connect a Lichess account.
#[tokio::test]
async fn a_wikibooks_refusal_does_not_ask_to_connect_lichess() {
    let fixture = serve(|_| Reply::Json(401, "{}".into())).await;
    let lookups = lookups(&fixture.base, 1_000);
    let error = lookups
        .service
        .opening_description(&line(&["e2e4"]))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "The opening description could not be loaded. Try again shortly."
    );
    assert!(!error.message.contains("Lichess"));
    assert_eq!(fixture.to("/wikibooks/w/api.php").len(), 1);
}

#[tokio::test]
async fn an_opening_description_is_read_from_wikibooks() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "query": { "pages": [{
                "title": "Chess Opening Theory/1. e4",
                "extract": "\n1. e4 · King's pawn\nWhite takes the centre.\n\n\nHistory\nAn old move."
            }] } })
            .to_string(),
        )
    })
    .await;
    let lookups = lookups(&fixture.base, 1_000);
    let page = lookups
        .service
        .opening_description(&line(&["e2e4"]))
        .await
        .unwrap()
        .expect("a description");
    assert!(page.blocks.iter().any(|block| block.heading));
    crate::support::wire::assert_wire("openingDescription.e4", &page);
}
