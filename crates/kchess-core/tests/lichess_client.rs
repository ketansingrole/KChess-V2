//! The Lichess client over local fixtures: error messages (`lichessError`, `throwLichessErrors`,
//! `unwrap`), the helpers `authorize` and `urlencoded`, streaming NDJSON with cancellation,
//! byte accounting and the redaction of tokens from the host log.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use fixtures::{Recording, Reply, http_client, serve};
use kchess_core::lichess::client::{Failure, LichessClient, authorize, urlencoded};
use kchess_core::lichess::policy::{Policy, is_stream};
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

/// A short policy deadline bounds headers but must not bound the stream body. The old
/// 30-second total deadline is covered statically (`is_stream` classifies the TV feed,
/// `http_client_builder` sets only a connect timeout, and `Policy::send` applies its
/// deadline to headers while `read_with_deadline` exempts streams), so the live part
/// only needs to outlive its own fixture deadline.
#[tokio::test]
async fn active_stream_outlives_its_header_deadline_and_remains_cancellable() {
    assert!(is_stream("https://lichess.org/api/tv/blitz/feed", None));
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let http = http_client();
    let policy = Policy::with_timeout(
        Arc::new(Recording::default()),
        http.clone(),
        CancellationToken::new(),
        Duration::from_millis(500),
    );
    let client = LichessClient::with_base(&fixture.base, policy, http);
    let cancel = CancellationToken::new();
    let (observed, mut observations) = mpsc::unbounded_channel();
    let reading = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            client
                .ndjson(
                    reqwest::Method::GET,
                    "/api/tv/blitz/feed",
                    &[],
                    None,
                    None,
                    &cancel,
                    |line| {
                        observed.send(line.to_string()).expect("observer");
                        Ok(())
                    },
                )
                .await
        }
    });
    send.send("{\"started\":true}\n".into()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), observations.recv())
        .await
        .unwrap()
        .unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        send.send("\n".into()).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !reading.is_finished(),
            "healthy stream ended before cancellation"
        );
    }
    send.send("{\"afterDeadline\":true}\n".into()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observations.recv())
            .await
            .unwrap()
            .unwrap(),
        "{\"afterDeadline\":true}"
    );
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(5), reading)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.is_aborted());
}

#[tokio::test]
async fn ordinary_and_error_response_bodies_keep_their_deadline() {
    for (status, stream) in [(200, false), (503, true)] {
        let (_keep_open, receive) = mpsc::unbounded_channel::<String>();
        let mut receive = Some(receive);
        let fixture =
            serve(move |_| Reply::StreamText(status, "application/json", receive.take().unwrap()))
                .await;
        let http = http_client();
        let policy = Policy::with_timeout(
            Arc::new(Recording::default()),
            http.clone(),
            CancellationToken::new(),
            Duration::from_millis(100),
        );
        let client = LichessClient::with_base(&fixture.base, policy, http);
        let cancel = CancellationToken::new();
        let error = if stream {
            client
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
                .unwrap_err()
        } else {
            client
                .get_json::<Value>("/api/account", &[], None, &cancel)
                .await
                .unwrap_err()
        };
        assert!(error.message().contains("timed out"));
    }
}

#[tokio::test]
async fn text_limit_counts_utf16_units_while_collecting_chunks() {
    use kchess_core::lichess::client::TextLimit;
    let fixture = serve(|_| Reply::Text(200, "text/plain", "a😀é".into())).await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let cancel = CancellationToken::new();
    let text = client
        .get_text_bounded(
            "/text",
            &[],
            "text/plain",
            None,
            &cancel,
            TextLimit {
                utf16_units: 4,
                message: "too large",
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "a😀é");
    let error = client
        .get_text_bounded(
            "/text",
            &[],
            "text/plain",
            None,
            &cancel,
            TextLimit {
                utf16_units: 3,
                message: "too large",
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.message(), "too large");
}

#[tokio::test]
async fn a_request_redirected_to_a_web_page_fails_instead_of_returning_the_page() {
    // Lichess sends `/game/export/{id}` without an account to its sign-up page.
    let fixture = serve(|seen| match seen.path() {
        "/game/export/abcd1234" => {
            Reply::WithHeaders(303, vec![("location", "/signup".into())], String::new())
        }
        _ => Reply::Text(
            200,
            "text/html",
            "<!doctype html><html>Sign up</html>".into(),
        ),
    })
    .await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let failure = client
        .get_text(
            "/game/export/abcd1234",
            &[],
            "application/x-chess-pgn",
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.message(),
        "Lichess 200 from GET /game/export/abcd1234: unexpected page response"
    );
}

#[tokio::test]
async fn one_exported_game_does_not_wait_for_a_running_bulk_export() {
    let fixture = serve(|seen| match seen.path() {
        "/api/games/user/Alice" => Reply::Hang,
        _ => Reply::Text(
            200,
            "application/x-chess-pgn",
            format!("[Site \"{}\"]\n\n1. e4 *\n", seen.body),
        ),
    })
    .await;
    let (client, _) = client_for(&fixture.base, Arc::new(Recording::default()));
    let client = Arc::new(client);
    let sync = CancellationToken::new();
    let syncing = {
        let client = Arc::clone(&client);
        let sync = sync.clone();
        tokio::spawn(async move {
            client
                .ndjson(
                    reqwest::Method::GET,
                    "/api/games/user/Alice",
                    &[],
                    None,
                    None,
                    &sync,
                    |_| Ok(()),
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pgn = tokio::time::timeout(
        Duration::from_secs(2),
        client.export_one(
            "abcd1234",
            &[],
            "application/x-chess-pgn",
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("the game waited for the bulk export")
    .unwrap();
    assert!(pgn.contains("abcd1234"));
    let asked = fixture.requests();
    let export = asked
        .iter()
        .find(|seen| seen.path() == "/api/games/export/_ids")
        .unwrap();
    assert_eq!(
        (export.method.as_str(), export.body.as_str()),
        ("POST", "abcd1234")
    );
    sync.cancel();
    let _ = syncing.await;
}
