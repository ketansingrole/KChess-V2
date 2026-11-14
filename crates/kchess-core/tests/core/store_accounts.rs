//! Accounts, tokens, played games, settings and the API cache in `kchess.db` (`store/games.rs`).
//! The token encryption is the fake
//! secret store of `support/store.rs`.

use crate::support;

use kchess_core::store::games;
use serde_json::{Value, json};
use support::store::{
    Secrets, check_page_query, check_username, game, game_page, list, parse_settings, profile,
};

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
        usernames(
            &store
                .read(|c| games::load_app_data(c.db))
                .expect("loadData")
        ),
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
    store
        .write(|c| games::add_account(c.db, "Alice", true))
        .expect("addAccount");
    assert!(check_username("no good name!").is_err());
    let friends = store
        .write(|c| games::add_friends(c.db, &["Bob".into(), "carol".into()]))
        .expect("addFriends");
    assert_eq!(usernames(&friends), ["Alice", "Bob", "carol"]);
    assert_eq!(
        store
            .read(|c| games::dismissed_friends(c.db))
            .expect("dismissedFriends"),
        json!([])
    );
    store
        .write(|c| games::remove_account(c.db, "carol"))
        .expect("removeAccount");
    assert_eq!(
        store
            .read(|c| games::dismissed_friends(c.db))
            .expect("dismissedFriends"),
        json!(["carol"])
    );
    store
        .write(|c| games::remove_account(c.db, "Bob"))
        .expect("removeAccount");
    let after = store
        .read(|c| games::load_app_data(c.db))
        .expect("loadData");
    assert_eq!(usernames(&after), ["Alice"]);
}

#[test]
fn one_account_or_all_are_logged_out_and_one_accounts_downloads_are_cleared() {
    let store = profile();
    store.save_login("Alice", "lip_alice", true).expect("login");
    store.save_login("Carol", "lip_carol", true).expect("login");
    store
        .write(|c| {
            games::save_games(
                c.host,
                c.db,
                "Alice",
                list(&json!([game("Game0001", &[])])),
                Some(2_000),
            )
        })
        .expect("saveGames");
    store
        .write(|c| games::write_api_cache(c.db, "alice:profile", &json!({ "name": "Alice" })))
        .expect("writeApiCache");
    store
        .write(|c| games::clear_account_data(c.db, "Alice"))
        .expect("clearAccountData");
    assert_eq!(
        store
            .read(|c| games::load_app_data(c.db))
            .expect("loadData")["gameCount"],
        json!(0)
    );
    assert_eq!(
        store
            .read(|c| games::read_api_cache(c.host, c.db, "alice:profile"))
            .expect("readApiCache"),
        Value::Null
    );
    store
        .write(|c| {
            games::save_games(
                c.host,
                c.db,
                "Alice",
                list(&json!([game("Game0002", &[])])),
                Some(2_000),
            )
        })
        .expect("saveGames");
    let after_one = store
        .write(|c| games::logout_accounts(c.db, Some("Alice")))
        .expect("logoutAccounts");
    assert_eq!(usernames(&after_one), ["Carol"]);
    assert_eq!(store.token("Alice"), None);
    let after_all = store
        .write(|c| games::logout_accounts(c.db, None))
        .expect("logoutAccounts");
    assert_eq!(usernames(&after_all), Vec::<String>::new());
    assert_eq!(store.token("Carol"), None);
}

#[test]
fn played_games_are_saved_listed_summarised_and_read_as_pgn() {
    let store = profile();
    store
        .write(|c| games::add_account(c.db, "Alice", true))
        .expect("addAccount");
    store
        .write(|c| {
            games::save_games(
                c.host,
                c.db,
                "Alice",
                list(&json!([
                    game("Game0001", &[]),
                    game("Game0002", &[("createdAt", json!(2_000))])
                ])),
                Some(3_000),
            )
        })
        .expect("saveGames");
    assert_eq!(
        store
            .read(|c| games::load_app_data(c.db))
            .expect("loadData")["gameCount"],
        json!(2)
    );
    let query = json!({ "offset": 0, "limit": 1, "account": "alice" });
    let page = store.read(|c| game_page(c, &query)).expect("gamePage");
    assert_eq!(page["total"], json!(2));
    assert_eq!(page["games"][0]["id"], json!("Game0002"));
    let invalid = json!({ "offset": -1, "limit": 1 });
    assert!(check_page_query(&invalid).is_err());
    let overview = store
        .read(games::game_library_overview)
        .expect("gameLibraryOverview");
    assert_eq!(overview["byAccount"]["alice"]["total"], json!(2));
    assert_eq!(overview["byAccount"]["alice"]["win"], json!(2));
    let history = store
        .read(|c| games::rating_history(c.db, "Alice"))
        .expect("gameRatingHistory");
    assert_eq!(history.as_array().map(Vec::len), Some(1));
    assert_eq!(history[0]["name"], json!("blitz"));
    assert_eq!(
        store
            .read(|c| games::game_pgn(c.db, "Alice", "Game0001"))
            .expect("gamePgn"),
        Value::Null
    );
    assert_eq!(
        store
            .read(|c| games::pending_game_ids(c.db, "Alice"))
            .expect("pendingGameIds"),
        json!([])
    );
}

#[test]
fn a_sync_whose_account_was_removed_meanwhile_is_refused() {
    let store = profile();
    store
        .write(|c| games::add_account(c.db, "Alice", true))
        .expect("addAccount");
    store
        .write(|c| games::remove_account(c.db, "Alice"))
        .expect("removeAccount");
    assert_eq!(
        store
            .write(|c| games::save_games_page(
                c.host,
                c.db,
                "Alice",
                list(&json!([game("Game0003", &[])])),
                &[]
            ))
            .expect_err("saveGamesPage")
            .message,
        "This account was removed during sync."
    );
}

#[test]
fn an_in_progress_game_is_pending_until_it_finishes() {
    let store = profile();
    store
        .write(|c| games::add_account(c.db, "Alice", true))
        .expect("addAccount");
    let live = game(
        "Live0001",
        &[("status", json!("started")), ("winner", Value::Null)],
    );
    store
        .write(|c| games::save_games_page(c.host, c.db, "Alice", list(&json!([live])), &[]))
        .expect("saveGamesPage");
    assert_eq!(
        store
            .read(|c| games::pending_game_ids(c.db, "Alice"))
            .expect("pendingGameIds"),
        json!(["Live0001"])
    );
    assert_eq!(
        store
            .read(|c| games::load_app_data(c.db))
            .expect("loadData")["gameCount"],
        json!(0)
    );
}

#[test]
fn settings_are_saved_and_read_back_and_cache_entries_are_kept() {
    let store = profile();
    let mut settings = store
        .read(|c| games::get_settings(c.db))
        .expect("getSettings");
    settings["zenMode"] = json!(true);
    let saved = store
        .write(|c| games::save_settings(c.db, &parse_settings(settings)?))
        .expect("saveSettings");
    assert_eq!(saved["zenMode"], json!(true));
    assert_eq!(
        store
            .read(|c| games::get_settings(c.db))
            .expect("getSettings")["zenMode"],
        json!(true)
    );

    store
        .write(|c| games::write_api_cache(c.db, "ratings", &json!({ "perf": 1 })))
        .expect("writeApiCache");
    let cached = store
        .read(|c| games::read_api_cache(c.host, c.db, "ratings"))
        .expect("readApiCache");
    assert_eq!(cached["value"], json!({ "perf": 1 }));
    assert!(cached["fetchedAt"].is_number());
}
