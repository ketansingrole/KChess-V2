//! Accounts, tokens, played games, settings and the API cache in `kchess.db` (`store/games.rs`).
//! Ports `tests/core/store-accounts.test.ts` case for case; the token encryption is the fake
//! secret store of `support/store.rs`.

mod support;

use serde_json::{Value, json};
use support::store::{Secrets, check_page_query, check_username, game, profile};

fn usernames(data: &Value) -> Vec<String> {
    data["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["username"].as_str().expect("username").to_string())
        .collect()
}

#[test]
fn a_login_is_stored_with_its_token_which_reads_back_decrypted() {
    let store = profile();
    let data = store.save_login("Alice", "lip_alice", true).expect("login");
    assert_eq!(usernames(&data), ["Alice"]);
    assert_eq!(data["accounts"][0]["connected"], json!(true));
    assert_eq!(store.token("Alice").as_deref(), Some("lip_alice"));
    assert_eq!(store.token("nobody"), None);
}

#[test]
fn a_login_is_refused_without_os_encryption_and_when_a_logout_overtook_it() {
    let mut store = profile();
    store.secrets = Secrets::OFF;
    assert_eq!(
        store.save_login("Alice", "lip_alice", true).unwrap_err(),
        "OS credential encryption is unavailable."
    );
    store.secrets = Secrets::ON;
    assert_eq!(
        store.save_login("Alice", "lip_alice", false).unwrap_err(),
        "Login was cancelled by logout."
    );
    assert_eq!(
        usernames(&store.ok("store.games.loadData", vec![])),
        Vec::<String>::new()
    );
}

#[test]
fn no_token_is_read_without_os_encryption_or_when_it_cannot_be_decrypted() {
    let mut store = profile();
    store.save_login("Alice", "lip_alice", true).expect("login");
    store.secrets = Secrets::OFF;
    assert_eq!(store.token("Alice"), None);
    store.secrets = Secrets {
        available: true,
        decrypts: false,
    };
    assert_eq!(store.token("Alice"), None);
}

#[test]
fn accounts_are_added_followed_dismissed_and_removed() {
    let store = profile();
    store.ok("store.games.addAccount", vec![json!("Alice"), json!(true)]);
    assert!(check_username("no good name!").is_err());
    let friends = store.ok("store.games.addFriends", vec![json!(["Bob", "carol"])]);
    assert_eq!(usernames(&friends), ["Alice", "Bob", "carol"]);
    assert_eq!(store.ok("store.games.dismissedFriends", vec![]), json!([]));
    store.ok("store.games.removeAccount", vec![json!("carol")]);
    assert_eq!(
        store.ok("store.games.dismissedFriends", vec![]),
        json!(["carol"])
    );
    store.ok("store.games.removeAccount", vec![json!("Bob")]);
    let after = store.ok("store.games.loadData", vec![]);
    assert_eq!(usernames(&after), ["Alice"]);
}

#[test]
fn one_account_or_all_are_logged_out_and_one_accounts_downloads_are_cleared() {
    let store = profile();
    store.save_login("Alice", "lip_alice", true).expect("login");
    store.save_login("Carol", "lip_carol", true).expect("login");
    store.ok(
        "store.games.saveGames",
        vec![
            json!("Alice"),
            json!([game("Game0001", &[])]),
            json!(2_000),
            json!(true),
        ],
    );
    store.ok(
        "store.games.writeApiCache",
        vec![
            json!("alice:profile"),
            json!({ "name": "Alice" }),
            json!(true),
        ],
    );
    store.ok("store.games.clearAccountData", vec![json!("Alice")]);
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(0)
    );
    assert_eq!(
        store.ok("store.games.readApiCache", vec![json!("alice:profile")]),
        Value::Null
    );
    store.ok(
        "store.games.saveGames",
        vec![
            json!("Alice"),
            json!([game("Game0002", &[])]),
            json!(2_000),
            json!(true),
        ],
    );
    let after_one = store.ok("store.games.logoutAccounts", vec![json!("Alice")]);
    assert_eq!(usernames(&after_one), ["Carol"]);
    assert_eq!(store.token("Alice"), None);
    let after_all = store.ok("store.games.logoutAccounts", vec![Value::Null]);
    assert_eq!(usernames(&after_all), Vec::<String>::new());
    assert_eq!(store.token("Carol"), None);
}

#[test]
fn played_games_are_saved_listed_summarised_and_read_as_pgn() {
    let store = profile();
    store.ok("store.games.addAccount", vec![json!("Alice"), json!(true)]);
    store.ok(
        "store.games.saveGames",
        vec![
            json!("Alice"),
            json!([
                game("Game0001", &[]),
                game("Game0002", &[("createdAt", json!(2_000))])
            ]),
            json!(3_000),
            json!(true),
        ],
    );
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(2)
    );
    let query = json!({ "offset": 0, "limit": 1, "account": "alice" });
    let page = store.ok("store.games.gamePage", vec![query]);
    assert_eq!(page["total"], json!(2));
    assert_eq!(page["games"][0]["id"], json!("Game0002"));
    let invalid = json!({ "offset": -1, "limit": 1 });
    assert!(check_page_query(&invalid).is_err());
    let overview = store.ok("store.games.gameLibraryOverview", vec![]);
    assert_eq!(overview["byAccount"]["alice"]["total"], json!(2));
    assert_eq!(overview["byAccount"]["alice"]["win"], json!(2));
    let history = store.ok("store.games.gameRatingHistory", vec![json!("Alice")]);
    assert_eq!(history.as_array().map(Vec::len), Some(1));
    assert_eq!(history[0]["name"], json!("blitz"));
    assert_eq!(
        store.ok(
            "store.games.gamePgn",
            vec![json!("Alice"), json!("Game0001")]
        ),
        Value::Null
    );
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!([])
    );
}

#[test]
fn a_sync_that_was_cancelled_or_whose_account_was_removed_meanwhile_is_refused() {
    let store = profile();
    store.ok("store.games.addAccount", vec![json!("Alice"), json!(true)]);
    let games = json!([game("Game0001", &[])]);
    assert_eq!(
        store.err(
            "store.games.saveGamesPage",
            vec![json!("Alice"), games, json!([]), json!(false)],
        ),
        "Sync was cancelled."
    );
    assert_eq!(
        store.err(
            "store.games.saveGames",
            vec![json!("Alice"), json!([]), json!(1), json!(false)],
        ),
        "Sync was cancelled."
    );
    store.ok("store.games.removeAccount", vec![json!("Alice")]);
    assert_eq!(
        store.err(
            "store.games.saveGamesPage",
            vec![
                json!("Alice"),
                json!([game("Game0003", &[])]),
                json!([]),
                json!(true)
            ],
        ),
        "This account was removed during sync."
    );
}

#[test]
fn an_in_progress_game_is_pending_until_it_finishes() {
    let store = profile();
    store.ok("store.games.addAccount", vec![json!("Alice"), json!(true)]);
    let live = game(
        "Live0001",
        &[("status", json!("started")), ("winner", Value::Null)],
    );
    store.ok(
        "store.games.saveGamesPage",
        vec![json!("Alice"), json!([live]), json!([]), json!(true)],
    );
    assert_eq!(
        store.ok("store.games.pendingGameIds", vec![json!("Alice")]),
        json!(["Live0001"])
    );
    assert_eq!(
        store.ok("store.games.loadData", vec![])["gameCount"],
        json!(0)
    );
}

#[test]
fn settings_are_saved_and_read_back_and_cache_entries_are_kept_only_while_current() {
    let store = profile();
    let mut settings = store.ok("store.games.getSettings", vec![]);
    settings["zenMode"] = json!(true);
    let saved = store.ok("store.games.saveSettings", vec![settings]);
    assert_eq!(saved["zenMode"], json!(true));
    assert_eq!(
        store.ok("store.games.getSettings", vec![])["zenMode"],
        json!(true)
    );

    store.ok(
        "store.games.writeApiCache",
        vec![json!("ratings"), json!({ "perf": 1 }), json!(false)],
    );
    assert_eq!(
        store.ok("store.games.readApiCache", vec![json!("ratings")]),
        Value::Null
    );

    store.ok(
        "store.games.writeApiCache",
        vec![json!("ratings"), json!({ "perf": 1 }), json!(true)],
    );
    let cached = store.ok("store.games.readApiCache", vec![json!("ratings")]);
    assert_eq!(cached["value"], json!({ "perf": 1 }));
    assert!(cached["fetchedAt"].is_number());
}
