//! The one-time import into an empty profile (`store/import.rs`): the JSON backup with its token
//! file, and an earlier release's main database, whose plaintext tokens are encrypted with the
//! fake secret store. Ports `tests/core/store-import.test.ts` case for case.

mod support;

use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::{Value, json};
use support::store::{Secrets, Store, encrypt};

/// An earlier release's database: the current schema, with the legacy tables and rows written
/// on top, as the test's `earlierDatabase` created it. Returns the store that owns the file and
/// the file's path.
fn earlier_database(sql: &str) -> (Store, PathBuf) {
    let earlier = Store::new(None, Secrets::OFF);
    earlier.ok("store.open", vec![]);
    earlier.close();
    let path = earlier.path().join("kchess.db");
    Connection::open(&path)
        .and_then(|connection| connection.execute_batch(sql))
        .expect("the earlier database is written");
    (earlier, path)
}

fn usernames(data: &Value) -> Vec<String> {
    data["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["username"].as_str().unwrap_or("<null>").to_string())
        .collect()
}

fn ids(page: &Value) -> Vec<String> {
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|game| game["id"].as_str().expect("id").to_string())
        .collect()
}

#[test]
fn the_json_backup_is_taken_with_its_tokens_and_both_files_are_archived() {
    let store = Store::new(None, Secrets::ON);
    let dir = store.path().to_path_buf();
    fs::write(
        dir.join("kchess-data.json"),
        json!({
            "settings": {
                "appearance": "dark",
                "boardPreset": "chess.com",
                "soundVolume": 80,
                "promotion": "odd",
            },
            "accounts": [
                { "username": "Alice", "connected": true },
                { "username": "Bob", "connected": false },
            ],
            "games": [
                {
                    "id": "Old0001",
                    "account": "Alice",
                    "createdAt": 1,
                    "lastMoveAt": 1,
                    "rated": true,
                    "speed": "blitz",
                    "perf": "blitz",
                    "status": "mate",
                    "color": "white",
                    "opponent": "Bob",
                    "moves": "e4 e5",
                },
                {
                    "id": "Live0001",
                    "account": "Alice",
                    "createdAt": 2,
                    "lastMoveAt": 2,
                    "rated": false,
                    "speed": "rapid",
                    "perf": "rapid",
                    "status": "started",
                    "color": "black",
                    "opponent": "Bob",
                    "moves": "",
                },
            ],
        })
        .to_string(),
    )
    .expect("backup written");
    fs::write(
        dir.join("lichess-tokens.json"),
        // The backup keeps ciphertext, as the earlier release wrote it.
        json!({ "Alice": encrypt("lip_alice") }).to_string(),
    )
    .expect("tokens written");

    let settings = store.ok("store.games.getSettings", vec![]);
    assert_eq!(settings["appearance"], json!("dark"));
    assert_eq!(settings["boardTheme"], json!("green"));
    assert_eq!(settings["promotion"], json!("ask"));
    assert_eq!(settings["soundVolume"], json!(0.8));

    let data = store.ok("store.games.loadData", vec![]);
    assert_eq!(usernames(&data), ["Alice", "Bob"]);
    assert_eq!(data["accounts"][0]["connected"], json!(true));
    assert_eq!(data["gameCount"], json!(1));
    let page = store.ok(
        "store.games.gamePage",
        vec![json!({ "offset": 0, "limit": 10 })],
    );
    assert_eq!(ids(&page), ["Old0001"]);
    assert_eq!(store.token("Alice").as_deref(), Some("lip_alice"));

    assert!(!dir.join("kchess-data.json").exists());
    assert!(dir.join("kchess-data.json.bak").exists());
    assert!(dir.join("lichess-tokens.json.bak").exists());
}

#[test]
fn an_earlier_main_database_is_imported_once_with_its_plaintext_tokens_encrypted() {
    let (_earlier, legacy) = earlier_database(
        "CREATE TABLE app_settings (key TEXT, value TEXT);
         INSERT INTO app_settings VALUES ('app.appearance_mode', 'light'), ('sound.enabled', 'false'),
           ('sound.volume', '40'), ('engine.custom_path', '/opt/fish');
         CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
         INSERT INTO lichess_accounts VALUES ('Carol', 'oauth', 5);
         CREATE TABLE lichess_games (game_id TEXT, account_username TEXT, played_at INTEGER, rated INTEGER,
           speed TEXT, perf TEXT, status TEXT, winner TEXT, color TEXT, opponent_name TEXT,
           opponent_rating INTEGER, player_rating INTEGER, rating_diff INTEGER, opening_name TEXT, moves TEXT);
         INSERT INTO lichess_games VALUES ('Legacy01', 'Carol', 7, 1, 'bullet', 'bullet', 'resign', 'black',
           'white', 'Dan', 1500, 1510, -4, NULL, 'e4 e5');
         CREATE TABLE lichess_tokens (username TEXT, token TEXT);
         INSERT INTO lichess_tokens VALUES ('Carol', 'lip_carol');",
    );
    let store = Store::new(Some(legacy.clone()), Secrets::ON);

    let settings = store.ok("store.games.getSettings", vec![]);
    assert_eq!(settings["appearance"], json!("light"));
    assert_eq!(settings["soundEnabled"], json!(false));
    assert_eq!(settings["soundVolume"], json!(0.4));
    assert_eq!(settings["enginePath"], json!("/opt/fish"));
    let data = store.ok("store.games.loadData", vec![]);
    assert_eq!(usernames(&data), ["Carol"]);
    assert_eq!(data["accounts"][0]["connected"], json!(true));
    let page = store.ok(
        "store.games.gamePage",
        vec![json!({ "offset": 0, "limit": 10 })],
    );
    assert_eq!(page["games"][0]["id"], json!("Legacy01"));
    assert_eq!(page["games"][0]["opponentRating"], json!(1500));
    assert_eq!(store.token("Carol").as_deref(), Some("lip_carol"));

    // Once imported, the profile is never imported again: a later change to the earlier database
    // does not reach it.
    store.close();
    let store = store.reopen_with(Secrets::ON);
    assert_eq!(
        usernames(&store.ok("store.games.loadData", vec![])),
        ["Carol"]
    );
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(1)
    );
    assert_eq!(
        store.ok("store.setupPositionLookup.explorerAccount", vec![]),
        json!("Carol")
    );
}

#[test]
fn an_earlier_database_leaves_its_tokens_out_when_os_encryption_is_unavailable() {
    let (_earlier, legacy) = earlier_database(
        "CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
         INSERT INTO lichess_accounts VALUES ('Erin', 'password', NULL);
         CREATE TABLE lichess_tokens (username TEXT, token TEXT);
         INSERT INTO lichess_tokens VALUES ('Erin', 'lip_erin');",
    );
    let store = Store::new(Some(legacy), Secrets::OFF);
    let data = store.ok("store.games.loadData", vec![]);
    assert_eq!(usernames(&data), ["Erin"]);
    assert_eq!(data["accounts"][0]["connected"], json!(false));
    assert_eq!(store.token("Erin"), None);
}

#[test]
fn damaged_backups_are_read_as_no_backup_and_nothing_is_imported_from_them() {
    let store = Store::new(None, Secrets::ON);
    let dir = store.path().to_path_buf();
    fs::write(dir.join("kchess-data.json"), "{ not json").expect("backup written");
    fs::write(dir.join("lichess-tokens.json"), "{ not json").expect("tokens written");
    assert_eq!(
        usernames(&store.ok("store.games.loadData", vec![])),
        Vec::<String>::new()
    );
    assert_eq!(
        fs::read_to_string(dir.join("kchess-data.json")).expect("backup kept"),
        "{ not json"
    );
}

#[test]
fn a_null_token_rolls_the_whole_earlier_import_back_when_encryption_is_available() {
    let (_earlier, legacy) = earlier_database(
        "CREATE TABLE app_settings (key TEXT, value TEXT);
         INSERT INTO app_settings VALUES ('app.appearance_mode', 'dark');
         CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
         INSERT INTO lichess_accounts VALUES ('Frank', 'oauth', NULL);
         CREATE TABLE lichess_tokens (username TEXT, token TEXT);
         INSERT INTO lichess_tokens VALUES ('Frank', NULL);",
    );
    let store = Store::new(Some(legacy), Secrets::ON);
    assert_eq!(
        store.ok("store.games.getSettings", vec![])["appearance"],
        json!("system")
    );
    assert_eq!(
        usernames(&store.ok("store.games.loadData", vec![])),
        Vec::<String>::new()
    );
}

#[test]
fn null_stays_null_and_null_tokens_are_skipped_without_encryption() {
    let (_earlier, legacy) = earlier_database(
        "CREATE TABLE app_settings (key TEXT, value TEXT);
         INSERT INTO app_settings VALUES ('app.appearance_mode', 'dark');
         CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
         INSERT INTO lichess_accounts VALUES (NULL, 'password', NULL);
         CREATE TABLE lichess_tokens (username TEXT, token TEXT);
         INSERT INTO lichess_tokens VALUES (NULL, 'lip_orphan'), ('Grace', NULL);",
    );
    let store = Store::new(Some(legacy), Secrets::OFF);
    assert_eq!(
        store.ok("store.games.getSettings", vec![])["appearance"],
        json!("dark")
    );
    assert_eq!(
        store.rows("SELECT username FROM accounts"),
        [json!({ "username": null })]
    );
    assert_eq!(
        store.rows("SELECT COUNT(*) AS n FROM tokens WHERE username = 'null'"),
        [json!({ "n": 0 })]
    );
}
