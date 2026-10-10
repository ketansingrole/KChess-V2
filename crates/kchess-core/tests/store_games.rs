//! The SQLite game library (`store/games.rs`): paging and filters, aggregates, pending games,
//! retention, indexes, and the removal of account data after logout, removal or clearing.
//! Ports `tests/core/game-library.test.ts` case for case.

mod support;

use kchess_core::store::games::MAX_GAMES;
use serde_json::{Value, json};
use support::store::{Secrets, Store, canonical, check_page_query, library_game, profile};

const SYNCED_AT: i64 = 1_790_812_800_000;

/// The library's fixture: Alice and Bob followed (Bob is not connected), as the test's beforeEach.
fn fresh() -> Store {
    let store = profile();
    store.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1), ('Bob', 0)");
    store.ok(
        "store.games.saveGames",
        vec![json!("Alice"), json!([]), json!(1), json!(true)],
    );
    store
}

fn save_games(store: &Store, account: &str, games: &[Value], synced_at: i64) -> Value {
    store.ok(
        "store.games.saveGames",
        vec![json!(account), json!(games), json!(synced_at), json!(true)],
    )
}

fn save_page(store: &Store, account: &str, games: &[Value]) -> Value {
    store.ok(
        "store.games.saveGamesPage",
        vec![json!(account), json!(games), json!([]), json!(true)],
    )
}

fn page(store: &Store, query: Value) -> Value {
    store.ok("store.games.gamePage", vec![query])
}

fn ids(page: &Value) -> Vec<String> {
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|game| game["id"].as_str().expect("id").to_string())
        .collect()
}

fn usernames(data: &Value) -> Vec<String> {
    data["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["username"].as_str().expect("username").to_string())
        .collect()
}

/// `SELECT` rows as the tests compare them.
fn rows(store: &Store, sql: &str) -> Vec<Value> {
    store.rows(sql)
}

const RESULT_SQL: &str =
    "CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END";

#[test]
fn logging_out_every_connected_account_erases_their_data_and_keeps_public_following() {
    let store = fresh();
    store.exec(
        "INSERT INTO accounts (username, connected) VALUES ('Carol', 1);
         INSERT INTO tokens VALUES ('Alice', 'secret'), ('Carol', 'secret');
         INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('carol:rating', '[]', 0), ('bob:profile', '{}', 0);
         INSERT INTO usage VALUES ('alice', 'games', 2, 200, 0), ('bob', 'games', 3, 300, 0)",
    );
    save_games(&store, "Alice", &[library_game("Private1", &[])], SYNCED_AT);
    save_games(
        &store,
        "Carol",
        &[library_game("Private2", &[("account", json!("Carol"))])],
        SYNCED_AT,
    );
    save_games(
        &store,
        "Bob",
        &[library_game("Public01", &[("account", json!("Bob"))])],
        SYNCED_AT,
    );
    save_page(
        &store,
        "Alice",
        &[library_game("Pending1", &[("status", json!("started"))])],
    );
    store.exec(
        "INSERT INTO lichess_review_checks VALUES ('Private1', 0), ('Public01', 0);
         INSERT INTO reviews VALUES ('private', 'Private1', 'local', 0, 0, '{}', '{}', 0), ('shared', 'Private2', 'local', 0, 0, '{}', '{}', 0);
         INSERT INTO game_reviews VALUES ('Private1', 'private'), ('Private2', 'shared'), ('Public01', 'shared')",
    );

    let result = store.ok("store.games.logoutAccounts", vec![Value::Null]);
    assert_eq!(usernames(&result), ["Bob"]);
    assert_eq!(result["accounts"][0]["connected"], json!(false));
    assert_eq!(result["gameCount"], json!(1));
    assert_eq!(
        rows(&store, "SELECT key FROM reviews"),
        [json!({ "key": "shared" })]
    );
    assert_eq!(
        rows(&store, "SELECT id FROM lichess_review_checks"),
        [json!({ "id": "Public01" })]
    );
    assert_eq!(rows(&store, "SELECT * FROM tokens"), Vec::<Value>::new());
    assert_eq!(
        rows(&store, "SELECT * FROM pending_game_sync"),
        Vec::<Value>::new()
    );
    assert_eq!(
        rows(&store, "SELECT key FROM api_cache"),
        [json!({ "key": "bob:profile" })]
    );
    assert_eq!(
        rows(&store, "SELECT account FROM usage"),
        [json!({ "account": "bob" })]
    );
    assert_eq!(
        ids(&page(&store, json!({ "offset": 0, "limit": 20 })))[0],
        "Public01"
    );
    let again = store.ok("store.games.logoutAccounts", vec![Value::Null]);
    assert_eq!(usernames(&again), ["Bob"]);
    assert_eq!(again["accounts"][0]["connected"], json!(false));
}

#[test]
fn logging_out_one_account_keeps_another_connected_account_and_a_followed_player() {
    let store = fresh();
    store.exec(
        "INSERT INTO accounts VALUES ('Carol', 1, NULL);
         INSERT INTO tokens VALUES ('Alice', 'secret-a'), ('Carol', 'secret-c');
         INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('carol:profile', '{}', 0)",
    );
    save_games(&store, "Alice", &[library_game("Private1", &[])], SYNCED_AT);
    save_games(
        &store,
        "Carol",
        &[library_game("Private2", &[("account", json!("Carol"))])],
        SYNCED_AT,
    );
    save_games(
        &store,
        "Bob",
        &[library_game("Public01", &[("account", json!("Bob"))])],
        SYNCED_AT,
    );
    let result = store.ok("store.games.logoutAccounts", vec![json!("aLiCe")]);
    let mut names = usernames(&result);
    names.sort();
    assert_eq!(names, ["Bob", "Carol"]);
    let carol = result["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| account["username"] == "Carol")
        .expect("Carol remains");
    assert_eq!(carol["connected"], json!(true));
    assert_eq!(result["gameCount"], json!(2));
    assert_eq!(
        rows(&store, "SELECT username FROM tokens"),
        [json!({ "username": "Carol" })]
    );
    assert_eq!(
        rows(&store, "SELECT key FROM api_cache"),
        [json!({ "key": "carol:profile" })]
    );
    store.ok("store.games.logoutAccounts", vec![json!("Bob")]);
    assert_eq!(
        usernames(&store.ok("store.games.loadData", vec![])).len(),
        2
    );
}

#[test]
fn bounded_pages_transfer_with_combined_filters_and_pgn_is_read_on_demand() {
    let store = fresh();
    let games: Vec<Value> = (0..240)
        .map(|i| {
            library_game(
                &format!("G{i:07}"),
                &[
                    ("rated", json!(i % 2 == 0)),
                    ("winner", json!(if i % 3 == 0 { "black" } else { "white" })),
                    ("createdAt", json!(i)),
                ],
            )
        })
        .collect();
    save_games(&store, "Alice", &games, SYNCED_AT);
    let data = store.ok("store.games.loadData", vec![]);
    assert_eq!(data["gameCount"], json!(240));
    assert!(data.get("games").is_none());

    let query = |offset: i64| json!({ "account": "alice", "result": "loss", "rated": true, "offset": offset, "limit": 10 });
    let first = page(&store, query(0));
    let next = page(&store, query(10));
    assert_eq!(first["total"], json!(40));
    assert_eq!(first["games"].as_array().map(Vec::len), Some(10));
    let mut seen = ids(&first);
    seen.extend(ids(&next));
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 20);
    for entry in first["games"].as_array().expect("games") {
        assert_eq!(entry["rated"], json!(true));
        assert_ne!(entry["winner"], entry["color"]);
        assert!(entry.get("pgn").is_none());
    }
    let first_id = ids(&first)[0].clone();
    assert_eq!(
        store.ok("store.games.gamePgn", vec![json!("ALICE"), json!(first_id)]),
        json!("1. e4 e5 *")
    );

    let full = page(&store, json!({ "offset": 0, "limit": 100 }));
    assert_eq!(full["games"].as_array().map(Vec::len), Some(100));
    assert!(
        full["games"]
            .as_array()
            .expect("games")
            .iter()
            .all(|g| g.get("pgn").is_none())
    );
    // The actual API must retain the representative 64 KiB transfer budget.
    assert!(serde_json::to_string(&full).expect("json").len() <= 64 * 1024);

    for invalid in [
        json!({ "limit": 101, "offset": 0 }),
        json!({ "limit": 10, "offset": -1 }),
        json!({ "limit": 10, "offset": 0, "result": "other" }),
        json!({ "limit": 10, "offset": 0, "account": "Alice' OR 1=1" }),
    ] {
        assert!(
            check_page_query(&invalid).is_err(),
            "{invalid} must be refused"
        );
    }
}

#[test]
fn stable_tie_ordering_and_aggregates_match_the_daily_rating_fallback() {
    let store = fresh();
    let games = [
        library_game("Abcd0001", &[]),
        library_game(
            "Abcd0002",
            &[
                ("winner", json!("black")),
                ("ratingDiff", json!(-8)),
                ("createdAt", json!(1_790_899_200_000_i64)),
            ],
        ),
        library_game(
            "Abcd0003",
            &[("winner", Value::Null), ("rated", json!(false))],
        ),
        library_game(
            "Abcd0004",
            &[
                ("account", json!("Bob")),
                ("opponent", json!("Alice")),
                ("color", json!("black")),
                ("winner", json!("black")),
                ("perf", json!("rapid")),
            ],
        ),
    ];
    save_games(&store, "Alice", &games[..3], SYNCED_AT);
    save_games(&store, "Bob", &games[3..], SYNCED_AT);
    let overview = store.ok("store.games.gameLibraryOverview", vec![]);
    assert_eq!(
        overview["byAccount"]["alice"],
        json!({ "total": 3, "win": 1, "loss": 1, "draw": 1 })
    );
    assert_eq!(
        overview["versus"]["bob"],
        json!({ "total": 3, "win": 1, "loss": 1, "draw": 1 })
    );
    assert!(overview["versus"].get("alice").is_none());

    // The rule the TypeScript test compared against (`ratingHistoryFromGames`, pure).
    let expected: Value = serde_json::from_str(
        &kchess_domain::api::call(
            "ratingHistoryFromGames",
            &json!([games[..3].to_vec()]).to_string(),
        )
        .expect("the rule answers"),
    )
    .expect("the history is JSON");
    assert_eq!(
        canonical(&store.ok("store.games.gameRatingHistory", vec![json!("alice")])),
        canonical(&expected)
    );

    let first = page(&store, json!({ "offset": 0, "limit": 2 }));
    let next = page(&store, json!({ "offset": 2, "limit": 2 }));
    let mut keys: Vec<String> = first["games"]
        .as_array()
        .expect("games")
        .iter()
        .chain(next["games"].as_array().expect("games"))
        .map(|game| {
            format!(
                "{}{}",
                game["account"].as_str().unwrap_or(""),
                game["id"].as_str().unwrap_or("")
            )
        })
        .collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 4);

    store.ok("store.games.clearAccountData", vec![json!("Alice")]);
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(1)
    );
    assert!(
        store.ok("store.games.gameLibraryOverview", vec![])["byAccount"]
            .get("alice")
            .is_none()
    );
}

#[test]
fn pending_changes_are_saved_atomically_live_games_are_excluded_and_cleaned_up_with_accounts() {
    let store = fresh();
    save_page(
        &store,
        "Alice",
        &[library_game("Pending1", &[("status", json!("started"))])],
    );
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("alice")]),
        json!(["Pending1"])
    );
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(0)
    );

    // A broken row rolls back the whole page, including removal from pending.
    let broken = library_game("Broken01", &[("createdAt", Value::Null)]);
    store.err(
        "store.games.saveGamesPage",
        vec![
            json!("Alice"),
            json!([library_game("Pending1", &[]), broken]),
            json!([]),
            json!(true),
        ],
    );
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!(["Pending1"])
    );
    assert_eq!(
        ids(&page(&store, json!({ "offset": 0, "limit": 20 }))),
        Vec::<String>::new()
    );

    store.ok("store.games.clearAccountData", vec![json!("Alice")]);
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!([])
    );
    save_page(
        &store,
        "Alice",
        &[library_game("Pending2", &[("status", json!("created"))])],
    );
    store.ok("store.games.removeAccount", vec![json!("Alice")]);
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!([])
    );
    let refused = store.err(
        "store.games.saveGamesPage",
        vec![
            json!("Alice"),
            json!([library_game("Pending2", &[])]),
            json!([]),
            json!(true),
        ],
    );
    assert!(refused.contains("removed"), "{refused}");
}

#[test]
fn pending_ids_survive_closing_and_reopening_the_database() {
    let store = profile();
    store.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1)");
    save_games(
        &store,
        "Alice",
        &[library_game("Pending1", &[("status", json!("started"))])],
        1000,
    );
    store.close();
    let store = store.reopen_with(Secrets::ON);
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!(["Pending1"])
    );
    save_games(&store, "Alice", &[library_game("Pending1", &[])], 2000);
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!([])
    );
    assert_eq!(
        ids(&page(&store, json!({ "offset": 0, "limit": 20 }))),
        ["Pending1"]
    );
}

#[test]
fn the_retention_cap_is_applied_once_per_completed_sync_not_on_every_page() {
    let store = fresh();
    let games: Vec<Value> = (0..=MAX_GAMES)
        .map(|i| library_game(&format!("T{i:07}"), &[("createdAt", json!(i + 1))]))
        .collect();
    save_page(&store, "Alice", &games);
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(MAX_GAMES + 1)
    );
    save_games(&store, "Alice", &[], 3000);
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(MAX_GAMES)
    );
    assert_eq!(
        store.ok(
            "store.games.gamePgn",
            vec![json!("Alice"), json!("T0000000")]
        ),
        Value::Null
    );
}

#[test]
fn result_filters_are_served_from_the_result_indexes() {
    let store = fresh();
    let plan = |sql: &str, params: &[&str]| -> String {
        store
            .rows_with(&format!("EXPLAIN QUERY PLAN {sql}"), params)
            .iter()
            .map(|row| row["detail"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let by_result = plan(
        &format!("SELECT COUNT(*) FROM games WHERE ({RESULT_SQL}) = ?"),
        &["loss"],
    );
    assert!(by_result.contains("idx_games_result_page"), "{by_result}");
    let by_account = plan(
        &format!(
            "SELECT COUNT(*) FROM games WHERE account = ? COLLATE NOCASE AND ({RESULT_SQL}) = ?"
        ),
        &["alice", "loss"],
    );
    assert!(
        by_account.contains("idx_games_account_result_page"),
        "{by_account}"
    );
}

/// Alice and Bob both played Shared01; Private1 is Alice's alone, with a review and lookups.
fn seed_shared_and_private(store: &Store) {
    save_games(
        store,
        "Alice",
        &[library_game("Private1", &[]), library_game("Shared01", &[])],
        SYNCED_AT,
    );
    save_games(
        store,
        "Bob",
        &[library_game("Shared01", &[("account", json!("Bob"))])],
        SYNCED_AT,
    );
    store.exec(
        "INSERT INTO reviews VALUES ('rp', 'Private1', 'local', 1, 0, '{}', '{}', 0),
           ('rs', 'Shared01', 'local', 1, 0, '{}', '{}', 0);
         INSERT INTO game_reviews VALUES ('Private1', 'rp'), ('Shared01', 'rs');
         INSERT INTO lichess_review_checks VALUES ('Private1', 0), ('Shared01', 0);
         INSERT INTO usage VALUES ('alice', 'games', 1, 10, 0), ('bob', 'games', 1, 10, 0);
         INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('bob:profile', '{}', 0);
         INSERT INTO position_lookups VALUES
           ('player:{\"player\":\"ALICE\",\"color\":\"white\"}:fen', '{}', 0),
           ('player:{\"player\":\"Bob\",\"color\":\"white\"}:fen', '{}', 0),
           ('opening:fen', '{}', 0);",
    );
}

/// What remains of the account data: reviews, checks, cache entries and lookups, in key order.
fn kept(store: &Store) -> Value {
    json!({
        "reviews": rows(store, "SELECT key FROM reviews ORDER BY key"),
        "checks": rows(store, "SELECT id FROM lichess_review_checks ORDER BY id"),
        "cache": rows(store, "SELECT key FROM api_cache ORDER BY key"),
        "lookups": rows(store, "SELECT key FROM position_lookups ORDER BY key"),
    })
}

fn others() -> Value {
    json!({
        "reviews": [{ "key": "rs" }],
        "checks": [{ "id": "Shared01" }],
        "cache": [{ "key": "bob:profile" }],
        "lookups": [
            { "key": "opening:fen" },
            { "key": "player:{\"player\":\"Bob\",\"color\":\"white\"}:fen" },
        ],
    })
}

#[test]
fn removing_an_account_deletes_its_reviews_checks_lookups_and_usage_but_spares_shared_games() {
    let store = fresh();
    seed_shared_and_private(&store);
    store.ok("store.games.removeAccount", vec![json!("alice")]);
    assert_eq!(kept(&store), others());
    assert_eq!(
        rows(&store, "SELECT account FROM usage"),
        [json!({ "account": "bob" })]
    );
    assert_eq!(
        store.ok("store.reviewStore.hasAccount", vec![json!("Alice")]),
        json!(false)
    );
}

#[test]
fn logging_out_deletes_the_same_data() {
    let store = fresh();
    seed_shared_and_private(&store);
    store.ok("store.games.logoutAccounts", vec![json!("Alice")]);
    assert_eq!(kept(&store), others());
}

#[test]
fn clearing_data_deletes_it_too_but_keeps_the_account_and_its_usage_history() {
    let store = fresh();
    seed_shared_and_private(&store);
    store.ok("store.games.clearAccountData", vec![json!("Alice")]);
    assert_eq!(kept(&store), others());
    assert_eq!(
        rows(&store, "SELECT account FROM usage WHERE account = 'alice'").len(),
        1
    );
    assert_eq!(
        store.ok("store.reviewStore.hasAccount", vec![json!("Alice")]),
        json!(true)
    );
}

#[test]
fn no_profile_data_is_cached_once_its_login_has_ended() {
    let store = fresh();
    store.ok(
        "store.games.writeApiCache",
        vec![
            json!("alice:profile"),
            json!({ "username": "Alice" }),
            json!(false),
        ],
    );
    assert_eq!(
        rows(&store, "SELECT key FROM api_cache"),
        Vec::<Value>::new()
    );
}

#[test]
fn with_os_encryption_a_login_and_its_account_are_stored_together_or_neither_after_a_logout() {
    let mut store = fresh();
    store.secrets = Secrets::ON;
    let cancelled = store.save_login("Carol", "lip_carol", false).unwrap_err();
    assert!(cancelled.contains("cancelled"), "{cancelled}");
    assert_eq!(
        rows(
            &store,
            "SELECT username FROM tokens WHERE username = 'Carol'"
        ),
        Vec::<Value>::new()
    );
    assert_eq!(
        store.ok("store.reviewStore.hasAccount", vec![json!("Carol")]),
        json!(false)
    );
    let data = store.save_login("Carol", "lip_carol", true).expect("login");
    let carol = data["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| account["username"] == "Carol")
        .expect("Carol");
    assert_eq!(carol["connected"], json!(true));
    assert_eq!(
        rows(
            &store,
            "SELECT username FROM tokens WHERE username = 'Carol'"
        )
        .len(),
        1
    );
}
