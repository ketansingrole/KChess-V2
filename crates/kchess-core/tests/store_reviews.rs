//! Review associations in `kchess.db` (`store/reviews.rs`): a review is shared by every game that
//! reaches its position, and each account's copy of a game is excluded from its review queue.
//! Ports `tests/core/review-associations.test.ts`. The review request runs through the core with
//! no engine (it never starts a search), as the TypeScript test did.

mod support;

use std::path::Path;
use std::sync::Arc;

use kchess_core::{Config, Core};
use kchess_domain::api;
use rusqlite::Connection;
use serde_json::{Value, json};
use support::Recording;

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// A core on a fresh profile with no engine: only associations are read and written.
struct Profile {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    core: Core,
}

fn profile() -> Profile {
    let dir = tempfile::tempdir().expect("temporary profile");
    let path = dir.path().to_path_buf();
    let mut config = Config::new(path.clone());
    config.bundled_engine_path = Some(path.join("no-engine.js"));
    let core = Core::open(config, Arc::new(Recording::default())).expect("the profile opens");
    core.call_sync("store.open", vec![])
        .expect("the database opens");
    Profile {
        _dir: dir,
        path,
        core,
    }
}

impl Profile {
    fn exec(&self, sql: &str) {
        database(&self.path).execute_batch(sql).expect("seeding");
    }

    fn sync(&self, method: &str, args: Vec<Value>) -> Value {
        self.core
            .call_sync(method, args)
            .unwrap_or_else(|cause| panic!("{method} failed: {cause}"))
    }

    fn write_review(&self, review: &Value) -> Value {
        self.sync("store.reviewStore.writeReview", vec![review.clone()])
    }

    fn read_review(&self, key: &str) -> Value {
        self.sync("store.reviewStore.readReview", vec![json!(key)])
    }

    fn review_summaries(&self, ids: &[&str]) -> Value {
        self.sync("store.reviewStore.reviewSummaries", vec![json!(ids)])
    }

    fn games_to_review(&self, accounts: &[&str]) -> Value {
        self.sync(
            "store.reviewStore.gamesToReview",
            vec![json!(accounts), json!(0), json!(300)],
        )
    }

    fn insights(&self, account: &str) -> Value {
        self.sync(
            "store.insights.insights",
            vec![json!({ "account": account })],
        )
    }

    /// `reviews.request`, which answers from the store when a review is known and otherwise
    /// queues a search (there is no engine here).
    async fn request_review(&self, request: Value) -> Value {
        self.core
            .call("reviews.request", vec![request])
            .await
            .expect("the review request answers")
    }

    /// Inserts a game the way the TypeScript test did: a resigned game of 1 ms, as White.
    fn game(&self, id: &str, account: &str) {
        self.exec(&format!(
            "INSERT INTO games (account,id,createdAt,lastMoveAt,rated,speed,perf,status,color,opponent,moves) VALUES ('{account}', '{id}', 1,1,0,'blitz','blitz','resign','white','Bob','e4 e5')"
        ));
    }

    fn clear(&self) {
        self.exec("DELETE FROM game_reviews; DELETE FROM reviews; DELETE FROM games");
    }
}

fn database(dir: &Path) -> Connection {
    Connection::open(dir.join("kchess.db")).expect("the database file opens")
}

fn keys(summaries: &Value) -> Vec<String> {
    let mut keys: Vec<String> = summaries
        .as_object()
        .expect("summaries")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn review(game: Option<&str>, overrides: &[(&str, Value)]) -> Value {
    let moves = json!(["e2e4", "e7e5"]);
    let key = review_key();
    let mut review = json!({
        "key": key,
        "fen": INITIAL_FEN,
        "moves": moves,
        "source": "local",
        "complete": true,
        "depth": 18,
        "updatedAt": 1,
        "evals": [{ "cp": 0 }, { "cp": 0 }, { "cp": 0 }],
    });
    if let Some(game) = game {
        review["gameId"] = json!(game);
    }
    let object = review.as_object_mut().expect("a review is an object");
    for (field, value) in overrides {
        object.insert((*field).to_string(), value.clone());
    }
    review
}

/// The position's review key, from the rules (`reviewKey`).
fn review_key() -> String {
    let answer = api::call(
        "reviewKey",
        &json!([INITIAL_FEN, ["e2e4", "e7e5"]]).to_string(),
    )
    .expect("the rule answers");
    serde_json::from_str(&answer).expect("the key is a string")
}

fn moves() -> Value {
    json!(["e2e4", "e7e5"])
}

fn fresh() -> Profile {
    let profile = profile();
    profile.clear();
    profile
}

#[test]
fn all_associations_are_retained_through_updates_and_each_account_copy_leaves_the_queue() {
    let profile = fresh();
    profile.game("Game0001", "Alice");
    profile.game("Game0002", "Alice");
    profile.game("Game0001", "Bob");
    profile.write_review(&review(Some("Game0001"), &[]));
    profile.write_review(&review(Some("Game0002"), &[]));
    profile.write_review(&review(Some("Game0002"), &[("depth", json!(20))]));
    assert_eq!(
        keys(&profile.review_summaries(&["Game0001", "Game0002"])),
        ["Game0001", "Game0002"]
    );
    assert_eq!(profile.games_to_review(&["Alice", "Bob"]), json!([]));
    assert_eq!(profile.insights("Alice")["accuracy"]["games"], json!(2));
}

#[test]
fn a_second_game_is_linked_even_when_a_stronger_review_prevents_replacing_its_evaluations() {
    let profile = fresh();
    profile.write_review(&review(Some("Game0001"), &[("source", json!("lichess"))]));
    let result = profile.write_review(&review(Some("Game0002"), &[("complete", json!(false))]));
    assert_eq!(result["review"]["source"], json!("lichess"));
    assert_eq!(result["review"]["complete"], json!(true));
    assert_eq!(result["review"]["gameId"], json!("Game0002"));
    assert_eq!(
        profile.read_review(&review_key())["source"],
        json!("lichess")
    );
    assert_eq!(
        profile.review_summaries(&["Game0001", "Game0002"])["Game0002"]["complete"],
        json!(true)
    );
}

#[tokio::test]
async fn a_requesting_game_is_linked_before_a_cached_lichess_review_is_returned() {
    let profile = fresh();
    profile.write_review(&review(Some("Game0001"), &[("source", json!("lichess"))]));
    let answer = profile
        .request_review(json!({ "fen": INITIAL_FEN, "moves": moves(), "gameId": "Game0002" }))
        .await;
    assert_eq!(answer["gameId"], json!("Game0002"));
    assert_eq!(
        profile
            .review_summaries(&["Game0001", "Game0002"])
            .as_object()
            .map(|map| map.len()),
        Some(2)
    );
}

#[tokio::test]
async fn both_associations_are_retained_when_identical_searches_are_queued_before_their_first_output()
 {
    let profile = fresh();
    profile
        .request_review(json!({ "fen": INITIAL_FEN, "moves": moves(), "gameId": "Game0001" }))
        .await;
    profile
        .request_review(json!({ "fen": INITIAL_FEN, "moves": moves(), "gameId": "Game0002" }))
        .await;
    profile.write_review(&review(Some("Game0002"), &[]));
    assert_eq!(
        profile
            .review_summaries(&["Game0001", "Game0002"])
            .as_object()
            .map(|map| map.len()),
        Some(2)
    );
    profile
        .core
        .call("close", vec![])
        .await
        .expect("the core closes");
}
