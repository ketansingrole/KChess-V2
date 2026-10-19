//! Lichess puzzle endpoints (the puzzle functions of `crates/kchess-node/js/lichess.ts`) over local
//! fixtures: reporting a solved puzzle, the next puzzle, and the reconnect and unreadable-answer
//! rules. The TypeScript suite has no direct puzzle test, so these cover the endpoint contract.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;

use serde_json::{Value, json};

use fixtures::{MemoryStore, MemoryTokens, Recording, Reply, lichess_service, serve};
use kchess_core::lichess::accounts::Reply as Outcome;
use kchess_core::lichess::puzzles::{PuzzleRequest, PuzzleSolveRequest, puzzle_next, puzzle_solve};

fn service(base: &str) -> Arc<kchess_core::lichess::accounts::Lichess> {
    lichess_service(
        base,
        &Arc::new(Recording::default()),
        MemoryStore::with_accounts(&[("Alice", true)]),
        MemoryTokens::with(&[("Alice", "lip_alice")]),
    )
}

#[tokio::test]
async fn cancels_a_puzzle_result_after_headers_while_its_body_is_stalled() {
    use std::time::Duration;
    use tokio::sync::mpsc;
    for status in [200, 500] {
        let (send, receive) = mpsc::unbounded_channel();
        let mut receive = Some(receive);
        let fixture =
            serve(move |_| Reply::StreamText(status, "application/json", receive.take().unwrap()))
                .await;
        let host = Arc::new(Recording::default());
        let lichess = lichess_service(
            &fixture.base,
            &host,
            MemoryStore::with_accounts(&[("Alice", true)]),
            MemoryTokens::with(&[("Alice", "lip_alice")]),
        );
        let solving = tokio::spawn({
            let lichess = Arc::clone(&lichess);
            async move {
                puzzle_solve(
                    &lichess,
                    &PuzzleSolveRequest {
                        account: "Alice".into(),
                        angle: "fork".into(),
                        id: "p1".into(),
                        win: true,
                        rated: true,
                    },
                )
                .await
            }
        });
        send.send("{\"rounds\":".into()).unwrap();
        // Policy records a request only after accepting its response headers.
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.usage_events().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("response headers accepted");
        assert!(!solving.is_finished());
        lichess.lifetime().cancel();
        let error = tokio::time::timeout(Duration::from_secs(1), solving)
            .await
            .expect("body cancellation is prompt")
            .expect("task")
            .expect_err("cancelled result");
        assert!(error.aborted);
        assert_eq!(
            fixture.requests().len(),
            1,
            "ambiguous result is never retried"
        );
        drop(send);
    }
}

#[tokio::test]
async fn reports_a_solved_puzzle_as_json_and_returns_the_rating_change() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            "{\"rounds\":[{\"id\":\"p1\",\"ratingDiff\":12}]}".into(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let request = PuzzleSolveRequest {
        account: "Alice".into(),
        angle: "mateIn2".into(),
        id: "p1".into(),
        win: true,
        rated: true,
    };
    let Outcome::Done(result) = puzzle_solve(&lichess, &request).await.unwrap() else {
        panic!("the login is connected");
    };
    assert_eq!(result.rating_diff, Some(serde_json::json!(12)));
    let sent = &fixture.requests()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path(), "/api/puzzle/batch/mateIn2");
    assert_eq!(sent.header("authorization"), Some("Bearer lip_alice"));
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "solutions": [{ "id": "p1", "win": true, "rated": true }] })
    );
}

#[tokio::test]
async fn an_unrated_result_reports_no_rating_change() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            "{\"rounds\":[{\"id\":\"p1\",\"ratingDiff\":12}]}".into(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let request = PuzzleSolveRequest {
        account: "Alice".into(),
        angle: "fork".into(),
        id: "p1".into(),
        win: false,
        rated: false,
    };
    let Outcome::Done(result) = puzzle_solve(&lichess, &request).await.unwrap() else {
        panic!("the login is connected");
    };
    assert_eq!(result.rating_diff, None);
}

#[tokio::test]
async fn a_refused_login_asks_the_user_to_connect_again() {
    let fixture = serve(|_| Reply::Json(401, "{}".into())).await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: "Alice".into(),
        angle: "mix".into(),
        difficulty: "normal".into(),
        color: None,
    };
    assert_eq!(
        puzzle_next(&lichess, &request).await.unwrap(),
        Outcome::NeedsReconnect
    );
}

#[tokio::test]
async fn a_theme_with_no_puzzles_left_is_an_empty_draw_not_a_failure() {
    // A narrow opening theme at Harder: Lichess answers an empty batch once the account has
    // played the puzzles near its rating. KChess showed that as "Couldn't get a puzzle".
    let fixture = serve(|_| {
        Reply::Json(
            200,
            "{\"puzzles\":[],\"glicko\":{\"rating\":1620.5}}".into(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: "Alice".into(),
        angle: "Anderssens_Opening_Polish_Gambit".into(),
        difficulty: "harder".into(),
        color: Some("white".into()),
    };
    let Outcome::Done(draw) = puzzle_next(&lichess, &request).await.unwrap() else {
        panic!("the login is connected");
    };
    assert_eq!(draw.puzzle, Value::Null);
    assert_eq!(draw.glicko, Some(json!({ "rating": 1620.5 })));
    assert_eq!(
        serde_json::to_value(&draw).unwrap(),
        json!({ "glicko": { "rating": 1620.5 } })
    );
    let sent = &fixture.requests()[0];
    assert_eq!(sent.query("nb").as_deref(), Some("10"));
    assert_eq!(sent.query("difficulty").as_deref(), Some("harder"));
    assert_eq!(sent.query("color").as_deref(), Some("white"));

    // Without an account, Lichess answers 404 when it has no puzzle to give.
    let fixture = serve(|_| Reply::Json(404, "{\"error\":\"Not found\"}".into())).await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: String::new(),
        ..request
    };
    let Outcome::Done(draw) = puzzle_next(&lichess, &request).await.unwrap() else {
        panic!("anonymous draws need no login");
    };
    assert_eq!(draw.puzzle, Value::Null);
}

#[tokio::test]
async fn an_answer_that_is_not_a_puzzle_is_refused() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: String::new(),
        angle: "mix".into(),
        difficulty: "normal".into(),
        color: None,
    };
    let error = puzzle_next(&lichess, &request).await.unwrap_err();
    assert!(error.message.contains("could not read"));
    // Anonymous training sends no login.
    assert!(fixture.requests()[0].header("authorization").is_none());
}

/// A puzzle `puzzleFromApi` reads, as an entry of a Lichess batch.
fn batch_entry(id: &str) -> Value {
    json!({
        "game": {},
        "puzzle": { "id": id, "rating": 1500, "solution": ["f2f4"], "themes": [], "initialPly": 0 }
    })
}

#[tokio::test]
async fn next_moves_on_while_lichess_still_serves_the_puzzle_just_shown() {
    // Lichess serves its next unplayed puzzles until a result reaches it: Next on a puzzle left
    // unfinished (or before its result arrived) drew the same puzzle again, so Next looked dead.
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({ "puzzles": [batch_entry("AAAAA"), batch_entry("BBBBB"), batch_entry("CCCCC")] })
                .to_string(),
        )
    })
    .await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: "Alice".into(),
        angle: "pawnEndgame".into(),
        difficulty: "harder".into(),
        color: None,
    };
    let mut drawn = Vec::new();
    for _ in 0..4 {
        let Outcome::Done(draw) = puzzle_next(&lichess, &request).await.unwrap() else {
            panic!("a puzzle");
        };
        drawn.push(draw.puzzle["id"].as_str().unwrap().to_string());
    }
    // Each Next shows another puzzle; once all were shown, the one shown longest ago returns.
    assert_eq!(drawn, ["AAAAA", "BBBBB", "CCCCC", "AAAAA"]);
}
