//! The Lichess client over local fixtures: error messages (`lichessError`, `throwLichessErrors`,
//! `unwrap`), the helpers `authorize` and `urlencoded`, streaming NDJSON with cancellation,
//! byte accounting and the redaction of tokens from the host log.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use fixtures::{Recording, Reply, http_client, serve};
use kchess_core::lichess::client::{Failure, LichessClient, authorize, urlencoded};
use kchess_core::lichess::policy::Policy;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn client_for(base: &str, host: Arc<Recording>) -> (LichessClient, Arc<Policy>) {
    let policy = Policy::with_timeout(
        host,
        http_client(),
        CancellationToken::new(),
        Duration::from_secs(5),
    );
    (
        LichessClient::with_base(base, Arc::clone(&policy), http_client()),
        policy,
    )
}

#[tokio::test]
async fn an_api_error_names_the_request_and_its_detail() {
    let fixture = serve(|_| Reply::Json(401, r#"{"error":"No such token"}"#.into())).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let failure = client
        .get_json::<Value>(
            "/api/account",
            &[],
            Some("Bearer lip_x"),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.status(), Some(401));
    assert_eq!(
        failure.message(),
        r#"Lichess 401 from GET /api/account: {"error":"No such token"}"#
    );
}

#[tokio::test]
async fn a_website_page_is_never_shown_to_the_user() {
    let fixture = serve(|_| {
        Reply::Text(
            404,
            "text/html",
            "<!doctype html><html><body>Not found</body></html>".into(),
        )
    })
    .await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let failure = client
        .get_json::<Value>("/api/user/nobody", &[], None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        failure.message(),
        "Lichess 404 from GET /api/user/nobody: not found"
    );
}

#[tokio::test]
async fn an_empty_success_names_the_path() {
    let fixture = serve(|_| Reply::Json(200, String::new())).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let failure = client
        .get_json::<Value>("/api/user/x", &[], None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        Failure::Lichess(kchess_core::lichess::client::LichessError {
            status: 200,
            endpoint: "/api/user/x".into(),
            detail: "empty response".into(),
        })
    );
    assert_eq!(
        failure.message(),
        "Lichess 200 from /api/user/x: empty response"
    );
}

#[tokio::test]
async fn the_token_is_sent_as_a_bearer_header_and_never_logged() {
    let fixture = serve(|seen| {
        if seen.path() == "/api/account" {
            Reply::Json(401, r#"{"error":"Invalid token"}"#.into())
        } else {
            Reply::Json(200, r#"{"username":"alice"}"#.into())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let (client, _) = client_for(&fixture.base, Arc::clone(&host));
    let token = "lip_secret_token_value";
    let rejected = client
        .get_json::<Value>(
            "/api/account",
            &[],
            Some(&authorize(token)),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.status(), Some(401));
    let sent = fixture.to("/api/account");
    assert_eq!(
        sent[0].header("authorization"),
        Some("Bearer lip_secret_token_value")
    );
    let error_text = rejected.message();
    assert!(!error_text.contains("lip_secret"));
    assert!(!host.logged().contains("lip_secret"));
}

#[tokio::test]
async fn a_form_post_sends_urlencoded_text() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let _: Value = client
        .post_form(
            "/inbox/alice",
            &[("text", "hi there & more")],
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let sent = fixture.requests();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].body, "text=hi+there+%26+more");
    assert_eq!(
        sent[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
}

#[test]
fn the_helpers_match_the_typescript_ones() {
    assert_eq!(authorize("abc"), "Bearer abc");
    assert_eq!(urlencoded(&[("text", "a b&c*-._~")]), "text=a+b%26c*-._%7E");
}

#[tokio::test]
async fn ndjson_lines_reach_the_caller_while_the_stream_is_open() {
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let (lines_out, mut lines_in) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    let reading = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            client
                .ndjson(
                    reqwest::Method::GET,
                    "/api/stream/event",
                    &[],
                    None,
                    Some("Bearer lip_x"),
                    &cancel,
                    |line| {
                        let _ = lines_out.send(line.to_string());
                        Ok(())
                    },
                )
                .await
        }
    });
    send.send("{\"type\":\"gameStart\"}\n".into()).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), lines_in.recv())
        .await
        .expect("the first line arrives before the stream ends")
        .unwrap();
    assert_eq!(first, r#"{"type":"gameStart"}"#);
    drop(send);
    reading.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_ndjson_error_names_the_request() {
    let fixture = serve(|_| Reply::Text(404, "text/html", "<html>nope</html>".into())).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let failure = client
        .ndjson(
            reqwest::Method::GET,
            "/api/games/user/nobody",
            &[],
            None,
            None,
            &CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.message(),
        "Lichess 404 from GET /api/games/user/nobody: not found"
    );
}

#[tokio::test]
async fn cancelling_an_open_stream_ends_the_read_with_an_abort() {
    let (_keep_open, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel();
    });
    let failure = client
        .ndjson(
            reqwest::Method::GET,
            "/api/stream/event",
            &[],
            None,
            None,
            &cancel,
            |_| Ok(()),
        )
        .await
        .unwrap_err();
    assert!(failure.is_aborted());
}

#[tokio::test]
async fn streamed_bytes_are_counted_for_the_account_that_read_them() {
    let body = "{\"id\":\"abcdefgh\"}\n";
    let mut sent = Some(body.to_string());
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let host = Arc::new(Recording::default());
    let (client, policy) = client_for(&fixture.base, Arc::clone(&host));
    let cancel = CancellationToken::new();
    let reading = policy.with_usage("Alice", "games", {
        let client = client.clone();
        let cancel = cancel.clone();
        async move {
            client
                .ndjson(
                    reqwest::Method::GET,
                    "/api/games/user/Alice",
                    &[],
                    None,
                    None,
                    &cancel,
                    |_| Ok(()),
                )
                .await
        }
    });
    send.send(sent.take().unwrap()).unwrap();
    drop(send);
    reading.await.unwrap();
    let bytes: u64 = host
        .usage_events()
        .iter()
        .filter(|event| event["account"] == "alice" && event["category"] == "games")
        .map(|event| event["bytes"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(bytes, body.len() as u64);
}
