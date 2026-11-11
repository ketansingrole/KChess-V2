//! What the facade's plain calls change or bring back: clearing runs and removing a played game
//! leave them gone, the explorer's pages come from Lichess and Wikibooks, a reset forgets the
//! network usage, and the puzzle database download fails, or stops when cancelled.

use crate::fixtures::{Reply, serve};
use crate::signed_in;
use crate::support;

use std::sync::Arc;

use kchess_core::test_host::TestHost;
use kchess_core::{Config, Core, TestSwitches};
use serde_json::json;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const DATABASE: &str = "/database/lichess_db_puzzle.csv.zst";

fn open(dir: &tempfile::TempDir, origin: &str) -> Arc<Core> {
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(origin.to_string()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.path().to_path_buf())
    };
    Arc::new(Core::open(config, TestHost::new()).expect("the profile opens"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cleared_runs_and_a_removed_game_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir, "http://127.0.0.1:9");
    let run = json!({ "kind": "storm", "variant": "3min", "score": 12, "detail": {} });
    core.call("saveRun", vec![run]).await.unwrap();
    let total =
        || async { core.call("runSummary", vec![json!("storm")]).await.unwrap()["total"].clone() };
    assert_eq!(total().await, json!(1));
    core.call("clearRuns", vec![json!("storm")]).await.unwrap();
    assert_eq!(total().await, json!(0));

    let game = json!({
        "id": "Played01", "source": "board", "startedAt": 1, "updatedAt": 2, "white": "A",
        "black": "B", "result": "*", "reason": "-", "timeControl": "-",
        "finished": false, "setup": { "variant": "standard", "fen": START },
        "moves": ["e2e4"],
    });
    core.call("saveArchivedGame", vec![game]).await.unwrap();
    let games = || async {
        core.call("library", vec![]).await.unwrap()["games"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(games().await, 1);
    core.call("removeArchivedGame", vec![json!("Played01")])
        .await
        .unwrap();
    assert_eq!(games().await, 0);
    core.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_explorers_pages_come_from_lichess_and_a_reset_forgets_their_usage() {
    let fixture = serve(|seen| {
        match seen.path() {
        "/explorer/masters/pgn/abcdefgh" => {
            Reply::Text(200, "text/plain", "[Event \"Masters\"]\n1. e4 *".into())
        }
        "/explorer/lichess/history" => Reply::Json(
            200,
            json!({ "history": [{ "month": "2024-01", "white": 400, "draws": 200, "black": 400 }] })
                .to_string(),
        ),
        "/wikibooks/w/api.php" => Reply::Json(
            200,
            json!({ "query": { "pages": [{
                "title": "Chess Opening Theory/1. e4",
                "extract": "\n1. e4 · King's pawn\nWhite takes the centre."
            }] } })
            .to_string(),
        ),
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        _ => Reply::Json(404, "{}".into()),
    }
    })
    .await;
    // The explorer needs a Lichess account.
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    core.call("resumeOnline", vec![]).await.expect("no game");
    let pgn = core
        .call("mastersGame", vec![json!("abcdefgh")])
        .await
        .unwrap();
    assert_eq!(pgn, json!("[Event \"Masters\"]\n1. e4 *"));
    let description = core
        .call("openingDescription", vec![json!(["e2e4"])])
        .await
        .unwrap();
    assert!(
        !description["blocks"].as_array().unwrap().is_empty(),
        "{description}"
    );
    let popularity = core
        .call("openingPopularity", vec![json!(["e2e4"])])
        .await
        .unwrap();
    assert_eq!(popularity[0]["month"], json!("2024-01"));

    let usage = core.call("usage", vec![]).await.unwrap();
    assert_ne!(usage["accounts"], json!({}), "{usage}");
    core.call("resetUsage", vec![]).await.unwrap();
    let usage = core.call("usage", vec![]).await.unwrap();
    assert_eq!(usage["accounts"], json!({}), "{usage}");
    core.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_puzzle_database_download_fails_or_stops_when_cancelled() {
    let mut calls = 0;
    let fixture = serve(move |seen| match seen.path() {
        DATABASE => {
            calls += 1;
            if calls == 1 {
                Reply::Json(500, "{}".into())
            } else {
                Reply::Hang
            }
        }
        _ => Reply::Json(404, "{}".into()),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(&dir, &fixture.base);
    core.call("puzzleDbInstall", vec![])
        .await
        .expect_err("Lichess refused the download");
    let installing = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("puzzleDbInstall", vec![]).await })
    };
    fixture.requested(DATABASE, 2).await;
    core.call("puzzleDbCancel", vec![]).await.unwrap();
    let status = tokio::time::timeout(support::UNTIL_LIMIT, installing)
        .await
        .expect("the download stops once cancelled")
        .unwrap()
        .expect("the status after the cancelled download");
    assert_eq!(status["installed"], json!(false));
    core.close().await;
}
