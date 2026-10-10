//! Lichess puzzle endpoints (the puzzle functions of `core/src/services/lichess.ts`) over local
//! fixtures: reporting a solved puzzle, the next puzzle, and the reconnect and unreadable-answer
//! rules. The TypeScript suite has no direct puzzle test, so these cover the endpoint contract.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;

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
async fn a_signed_in_batch_without_puzzles_says_so() {
    let fixture = serve(|_| Reply::Json(200, "{\"puzzles\":[]}".into())).await;
    let lichess = service(&fixture.base);
    let request = PuzzleRequest {
        account: "Alice".into(),
        angle: "fork".into(),
        difficulty: "hard".into(),
        color: Some("white".into()),
    };
    let error = puzzle_next(&lichess, &request).await.unwrap_err();
    assert!(error.message.contains("no more puzzles"));
    let sent = &fixture.requests()[0];
    assert_eq!(sent.query("nb").as_deref(), Some("1"));
    assert_eq!(sent.query("color").as_deref(), Some("white"));
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
