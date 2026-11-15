//! The facade's account calls end what an account had: signing out or removing an account forgets
//! the tournaments it joined (whose event streams would otherwise reopen), and each call tells
//! the other windows the accounts changed.

use crate::fixtures::{Reply, serve};
use crate::signed_in;

use kchess_contracts::types::JoinedTournament;
use kchess_core::store::library;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_or_removing_an_account_forgets_the_tournaments_it_joined() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    core.call("addAccount", vec![json!("Bob")]).await.unwrap();
    let now = kchess_core::time::now_ms() as f64;
    core.database_for_tests()
        .write(|c| {
            for (id, account) in [
                ("Arena001", "Alice"),
                ("Arena002", "Bob"),
                ("Arena003", "Carol"),
            ] {
                let entry: JoinedTournament = serde_json::from_value(json!({
                    "system": "arena", "id": id, "account": account, "name": id,
                    "until": now + 86_400_000.0,
                }))
                .expect("a joined tournament");
                library::remember_tournament(c.host, c.db, &entry, now)?;
            }
            Ok(())
        })
        .unwrap();
    let joined = || async {
        let mut accounts: Vec<String> = core
            .call("joinedTournaments", vec![])
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["account"].as_str().unwrap().to_string())
            .collect();
        accounts.sort();
        accounts
    };
    assert_eq!(joined().await, ["Alice", "Bob", "Carol"]);

    // Bob has no login: removing him still ends what he had.
    core.call("removeAccount", vec![json!("Bob")])
        .await
        .unwrap();
    assert_eq!(joined().await, ["Alice", "Carol"]);
    // Alice's session ends with her login.
    let data = core.call("logout", vec![json!("Alice")]).await.unwrap();
    assert!(
        data["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|account| account["connected"] == json!(false)),
        "{data}"
    );
    assert_eq!(joined().await, ["Carol"]);

    let changes: Vec<Value> = profile
        .events()
        .into_iter()
        .filter(|(name, _)| name == "profile:changed")
        .map(|(_, payload)| payload["areas"].clone())
        .collect();
    assert_eq!(changes, vec![json!(["accounts"]); 3], "{changes:?}");
    core.close().await;
}

/// 2026-11-15: a Chess.com game's review was refused as if its player were gone (only Lichess
/// accounts were looked for), and removing the player left its running review to write the
/// deleted game back. The review runs, and stops when the player goes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chesscom_review_runs_and_stops_when_its_player_is_removed() {
    use crate::support::{copy_log, engine_copy, received, save_engine_path, wait_until};
    let fixture = serve(|seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/pub/player/alice" => Reply::Json(
            200,
            json!({ "username": "alice", "url": "https://www.chess.com/member/Alice" }).to_string(),
        ),
        "/pub/player/alice/games/archives" => {
            Reply::Json(200, json!({ "archives": [] }).to_string())
        }
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    let profile = signed_in::open_with_chesscom(&fixture.base).await;
    let core = &profile.core;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_copy(dir.path(), "stall");
    save_engine_path(core, &engine);
    core.call("resumeOnline", vec![]).await.unwrap();
    core.call("addChesscomAccount", vec![json!("alice")])
        .await
        .unwrap();
    let request = json!({
        "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "moves": ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"],
        "gameId": "cc-l1",
        "account": "alice@chess.com",
    });
    core.call("reviewRequest", vec![request]).await.unwrap();
    let log = copy_log(&engine);
    let wait = std::time::Duration::from_secs(30);
    wait_until(wait, "the review to search", || {
        received(&log)
            .iter()
            .any(|line| line.starts_with("go"))
            .then_some(())
    })
    .await;
    core.call("removeChesscomAccount", vec![json!("alice")])
        .await
        .unwrap();
    wait_until(wait, "the review to stop", || {
        received(&log)
            .iter()
            .any(|line| line == "stop")
            .then_some(())
    })
    .await;
}
