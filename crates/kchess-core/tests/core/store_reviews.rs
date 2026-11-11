//! Review associations in `kchess.db` (`store/reviews.rs`): a review is shared by every game that
//! reaches its position, and each account's copy of a game is excluded from its review queue.
//! The review request runs through the core with no engine (it never starts a search).

use crate::support;

use std::path::Path;
use std::sync::Arc;

use kchess_core::engine::review::GameToReview;
use kchess_core::store::{StoreContext, insights, reviews};
use kchess_core::{Config, Core, CoreError};
use rusqlite::Connection;
use serde_json::{Value, json};
use support::{Recording, wire};

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
    config.bundled_engine_path = Some(path.join("no-engine"));
    let core = Core::open(config, Arc::new(Recording::default())).expect("the profile opens");
    core.database_for_tests()
        .read(|_| Ok(()))
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

    fn read<T: serde::Serialize>(
        &self,
        work: impl FnOnce(&StoreContext) -> Result<T, CoreError>,
    ) -> Value {
        let value = self.core.database_for_tests().read(work).expect("read");
        serde_json::to_value(value).expect("json")
    }

    fn write_reviews(&self, reviews: &Value) -> Result<Value, CoreError> {
        self.core
            .database_for_tests()
            .write(|c| reviews::write_reviews(c.host, c.db, reviews))
    }

    fn write_review(&self, review: &Value) -> Value {
        self.core
            .database_for_tests()
            .write(|c| reviews::write_review(c.host, c.db, review))
            .expect("writeReview")
    }

    fn read_review(&self, key: &str) -> Value {
        self.read(|c| reviews::read_review(c.host, c.db, key))
    }

    fn review_summaries(&self, ids: &[&str]) -> Value {
        let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        self.read(|c| reviews::review_summaries(c.host, c.db, &ids))
    }

    fn games_to_review(&self, accounts: &[&str]) -> Value {
        let accounts: Vec<String> = accounts.iter().map(|name| name.to_string()).collect();
        self.read(|c| reviews::games_to_review(c.db, &accounts, 0.0, 300.0))
    }

    fn insights(&self, account: &str) -> Value {
        let query = support::store::insights_query(&json!({ "account": account }));
        self.read(|c| insights::insights(c.host, c.db, &query, kchess_core::time::now_ms() as f64))
    }

    /// `reviewRequest`, which answers from the store when a review is known and otherwise
    /// queues a search (there is no engine here).
    async fn request_review(&self, request: Value) -> Value {
        self.core
            .call("resumeOnline", vec![])
            .await
            .expect("no game to resume");
        self.core
            .call("reviewRequest", vec![request])
            .await
            .expect("the review request answers")
    }

    /// Inserts a resigned game of 1 ms, as White.
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

/// The position's review key, from the rules.
fn review_key() -> String {
    kchess_domain::records::review::review_key(INITIAL_FEN, &["e2e4", "e7e5"], None)
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

/// The queue parses these rows as `GameToReview` (`engine::bridge`): a floating-point
/// `createdAt` fails the whole pass ("expected i64") and automatic reviews stop.
#[test]
fn games_to_review_rows_parse_as_the_queue_expects() {
    let profile = fresh();
    profile.game("Game0001", "Alice");
    let games: Vec<GameToReview> = serde_json::from_value(profile.games_to_review(&["Alice"]))
        .expect("the queue reads the games to review");
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].id, "Game0001");
    assert_eq!(games[0].created_at, 1);
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
    wire::assert_wire_steady("reviewRequest.cached-lichess", &answer, &["updatedAt"]);
    assert_eq!(
        profile
            .review_summaries(&["Game0001", "Game0002"])
            .as_object()
            .map(|map| map.len()),
        Some(2)
    );
    let summaries = profile
        .core
        .call("reviewSummaries", vec![json!(["Game0002", "Game0001"])])
        .await
        .expect("the summaries");
    wire::assert_wire("reviewSummaries.two-games", &summaries);
    let found = profile
        .core
        .call("reviewGet", vec![json!(INITIAL_FEN), moves()])
        .await
        .expect("the stored review");
    wire::assert_wire_steady("reviewGet.lichess", &found, &["updatedAt"]);
    let missing = profile
        .core
        .call("reviewGet", vec![json!(INITIAL_FEN), json!(["d2d4"])])
        .await
        .expect("no review");
    wire::assert_wire("reviewGet.none", &missing);
    profile.core.close().await;
}

/// A review Lichess made keeps its own labels and accuracy figures, as Lichess sent them.
#[tokio::test]
async fn a_lichess_review_is_read_with_its_labels_and_accuracy() {
    let profile = fresh();
    profile.write_review(&review(
        Some("Game0001"),
        &[
            ("source", json!("lichess")),
            ("depth", json!(0)),
            (
                "evals",
                json!([{ "cp": 15 }, { "cp": 20, "best": "g1f3", "pv": ["g1f3"] }, null]),
            ),
            ("judgments", json!([null, "mistake"])),
            ("accuracy", json!({ "white": 87, "black": 64.5 })),
        ],
    ));
    profile
        .core
        .call("resumeOnline", vec![])
        .await
        .expect("no game to resume");
    let found = profile
        .core
        .call("reviewGet", vec![json!(INITIAL_FEN), moves()])
        .await
        .expect("the stored review");
    wire::assert_wire_steady("reviewGet.lichess-labels", &found, &["updatedAt"]);
    let summaries = profile
        .core
        .call("reviewSummaries", vec![json!(["Game0001"])])
        .await
        .expect("the summaries");
    wire::assert_wire("reviewSummaries.lichess", &summaries);
    profile.core.close().await;
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
    profile.core.close().await;
}

#[test]
fn a_review_batch_writes_atomically_and_a_failed_batch_keeps_the_previous_reviews() {
    let profile = fresh();
    let first = review(Some("Game0001"), &[]);
    let key = first["key"].as_str().expect("review key").to_string();
    profile.write_review(&first);
    // A batch with one malformed review fails as a whole: the good half is not kept.
    // NB: same position => same key, so the batch targets the stored review's key.
    let second = {
        let mut second = review(Some("Game0002"), &[]);
        second["key"] = json!("batch-second-key");
        second
    };
    let bad = json!({ "fen": INITIAL_FEN });
    let outcome = profile.write_reviews(&json!([second, bad]));
    assert!(outcome.is_err(), "a malformed batch member fails the batch");
    assert!(
        profile.read_review("batch-second-key").is_null(),
        "the failed batch keeps the previous reviews"
    );
    // A well-formed batch writes every member in one transaction.
    let stored = profile
        .write_reviews(&json!([second]))
        .expect("writeReviews");
    assert_eq!(stored.as_array().map(Vec::len), Some(1));
    assert_eq!(
        profile.read_review("batch-second-key")["gameId"],
        json!("Game0002")
    );
    // The first review (different key) is untouched.
    assert_eq!(profile.read_review(&key)["gameId"], json!("Game0001"));
}

/// Time use reads each synced game's clocks: White fell under a tenth of three minutes and
/// blundered there, so the game counts as time trouble and its blunder as one short of time.
#[test]
fn time_use_counts_blunders_made_short_of_time() {
    let profile = fresh();
    let pgn = "[TimeControl \"180+0\"]\n\n\
        1. e4 { [%clk 0:03:00] } 1... e5 { [%clk 0:03:00] } 2. Nf3 { [%clk 0:00:10] } \
        2... Nc6 { [%clk 0:02:50] } 3. Bc4 { [%clk 0:00:08] } 1-0";
    profile.exec(&format!(
        "INSERT INTO games (account,id,createdAt,lastMoveAt,rated,speed,perf,status,winner,color,opponent,moves,pgn) \
         VALUES ('Alice','Clocked1',1,1,1,'blitz','blitz','resign','white','white','Bob','e4 e5 Nf3 Nc6 Bc4','{pgn}')"
    ));
    // A game without clocks is left out.
    profile.game("Unclocked", "Alice");
    let report = profile.insights("Alice");
    assert_eq!(report["total"], json!(2));
    let time = &report["timeUse"];
    assert_eq!(time["games"], json!(1));
    assert_eq!(time["trouble"]["win"], json!(1));
    assert_eq!(time["spare"]["total"], json!(0));
    assert_eq!(time["phases"].as_array().map(Vec::len), Some(1));
    assert_eq!(time["phases"][0]["phase"], json!("opening"));
    assert_eq!(time["phases"][0]["moves"], json!(3));
    assert_eq!(time["phases"][0]["share"], json!(100));
    assert_eq!(time["clockAt"], json!([]), "the game never reached move 20");
    assert!(time.get("blunders").is_none(), "no review yet");

    let moves = json!(["e2e4", "e7e5", "g1f3", "b8c6", "f1c4"]);
    profile.write_review(&json!({
        "key": "clocked-review",
        "gameId": "Clocked1",
        "fen": INITIAL_FEN,
        "moves": moves,
        "source": "local",
        "complete": true,
        "depth": 18,
        "updatedAt": 1,
        "evals": [{ "cp": 15 }, { "cp": 15 }, { "cp": 15 }, { "cp": 15 }, { "cp": 15 }, { "cp": -900 }],
    }));
    let time = profile.insights("Alice")["timeUse"].clone();
    assert_eq!(
        time["blunders"],
        json!({
            "games": 1,
            "short": { "moves": 1, "blunders": 1 },
            "spare": { "moves": 2, "blunders": 0 },
        })
    );
}
