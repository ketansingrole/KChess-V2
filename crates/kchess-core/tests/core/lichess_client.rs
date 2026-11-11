//! The Lichess client over local fixtures: error messages and kinds, `urlencoded`, the request
//! readers (text, JSON, NDJSON lines) with cancellation and deadlines, byte accounting and the
//! redaction of tokens from the host log.

use crate::fixtures;

use std::sync::Arc;
use std::time::Duration;

use fixtures::{Recording, Reply, http_client, serve};
use kchess_contracts::types::UsageKind;
use kchess_core::ErrorKind;
use kchess_core::lichess::client::{Auth, Lane, LichessClient, LichessError, urlencoded};
use kchess_core::lichess::policy::Policy;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn client_for(base: &str, host: Arc<Recording>) -> LichessClient {
    client_with_timeout(base, host, Duration::from_secs(5))
}

fn client_with_timeout(base: &str, host: Arc<Recording>, timeout: Duration) -> LichessClient {
    let policy = Policy::with_timeout(
        host.clone(),
        host,
        http_client(),
        CancellationToken::new(),
        timeout,
    );
    LichessClient::with_base(base, policy)
}

fn nobody(client: &LichessClient) -> Auth {
    client.anonymous(UsageKind::Other)
}

#[tokio::test]
async fn an_api_error_is_plain_for_people_and_technical_for_the_log() {
    let fixture = serve(|_| Reply::Json(401, r#"{"error":"No such token"}"#.into())).await;
    let host = Arc::new(Recording::default());
    let client = client_for(&fixture.base, Arc::clone(&host));
    let error = client
        .get("/api/account", &nobody(&client).with_token("lip_x"))
        .read_json::<Value>()
        .await
        .unwrap_err();
    // A 401 asks for the account to be reconnected, without the status or the endpoint.
    assert_eq!(
        error.message,
        "Lichess no longer accepts this account’s login. Reconnect it."
    );
    assert_eq!(error.kind, Some(ErrorKind::Login));
    assert!(
        host.logged()
            .contains(r#"Lichess 401 from GET /api/account: {"error":"No such token"}"#),
        "{}",
        host.logged()
    );
}

async fn failure_for(status: u16, body: &str) -> kchess_core::CoreError {
    let body = body.to_string();
    let fixture = serve(move |_| Reply::Json(status, body.clone())).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    client
        .get("/api/board/game/stream/abc", &nobody(&client))
        .read_json::<Value>()
        .await
        .unwrap_err()
}

/// Opening a game Lichess keeps from the Board API showed
/// `Lichess 400 from GET /api/board/game/stream/…: {"error":…}`.
#[tokio::test]
async fn a_board_api_refusal_is_lichess_s_own_sentence() {
    let error = failure_for(
        400,
        r#"{"error":"This game cannot be played with the Board API."}"#,
    )
    .await;
    assert_eq!(
        error.message,
        "This game cannot be played with the Board API."
    );
    assert_eq!(error.kind, Some(ErrorKind::Refused));
    assert!(!ErrorKind::Refused.retryable());
}

/// A 403 that is not about the login (a game Lichess will not let this account play) is a
/// refusal: it never asks the person to reconnect an account that works.
#[tokio::test]
async fn a_403_about_the_request_is_a_refusal_and_one_about_the_token_is_a_login() {
    let refused = failure_for(403, r#"{"error":"You cannot play this game."}"#).await;
    assert_eq!(refused.kind, Some(ErrorKind::Refused));
    assert_eq!(refused.message, "You cannot play this game.");
    let login = failure_for(403, r#"{"error":"Missing scope: board:play"}"#).await;
    assert_eq!(login.kind, Some(ErrorKind::Login));
}

#[tokio::test]
async fn too_many_requests_asks_to_wait() {
    let error = failure_for(429, "").await;
    assert_eq!(
        error.message,
        "Lichess asked KChess to slow down. Try again in a minute."
    );
    assert_eq!(error.kind, Some(ErrorKind::RateLimited));
}

#[tokio::test]
async fn a_server_error_shows_no_status_or_body() {
    let error = failure_for(502, r#"{"error":"upstream <b>gateway</b>"}"#).await;
    assert_eq!(
        error.message,
        "Lichess isn’t working right now. Try again shortly."
    );
    assert_eq!(error.kind, Some(ErrorKind::Unavailable));
}

#[tokio::test]
async fn an_unreachable_lichess_is_offline() {
    let client = client_for("http://127.0.0.1:9", Arc::new(Recording::default()));
    let error = client
        .get("/api/account", &nobody(&client))
        .read_json::<Value>()
        .await
        .unwrap_err();
    assert_eq!(error.message, "Couldn’t reach Lichess");
    assert_eq!(error.kind, Some(ErrorKind::Offline));
}

/// No user-visible message carries a status, an endpoint or raw JSON, whatever Lichess answers.
#[test]
fn no_message_names_a_status_an_endpoint_or_json() {
    for status in [400, 401, 403, 404, 409, 410, 422, 429, 500, 502, 503] {
        for detail in [
            "",
            "not found",
            r#"{"error":"No such token"}"#,
            r#"{"error":{"nested":true}}"#,
            "<html>oops</html>",
        ] {
            let message = LichessError {
                status,
                endpoint: "GET /api/board/game/stream/abc".into(),
                detail: detail.into(),
            }
            .message();
            assert!(
                !message.contains(&status.to_string()),
                "{status} {detail}: {message}"
            );
            assert!(!message.contains("/api/"), "{status} {detail}: {message}");
            assert!(
                !message.contains(['{', '<']),
                "{status} {detail}: {message}"
            );
        }
    }
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
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let error = client
        .get("/api/user/nobody", &nobody(&client))
        .read_json::<Value>()
        .await
        .unwrap_err();
    assert_eq!(error.message, "Lichess has no such thing.");
    assert_eq!(error.kind, Some(ErrorKind::NotFound));
}

#[tokio::test]
async fn an_empty_success_is_unavailable() {
    let fixture = serve(|_| Reply::Json(200, String::new())).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let error = client
        .get("/api/user/x", &nobody(&client))
        .read_json::<Value>()
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "Lichess isn’t working right now. Try again shortly."
    );
    assert_eq!(error.kind, Some(ErrorKind::Unavailable));
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
    let client = client_for(&fixture.base, Arc::clone(&host));
    let auth = nobody(&client).with_token("lip_secret_token_value");
    assert!(!format!("{auth:?}").contains("lip_secret"));
    let rejected = client
        .get("/api/account", &auth)
        .read_json::<Value>()
        .await
        .unwrap_err();
    assert_eq!(rejected.kind, Some(ErrorKind::Login));
    let sent = fixture.to("/api/account");
    assert_eq!(
        sent[0].header("authorization"),
        Some("Bearer lip_secret_token_value")
    );
    assert!(!rejected.message.contains("lip_secret"));
    assert!(!host.logged().contains("lip_secret"));
    // Without a login, no header at all.
    client
        .get("/api/user/alice", &auth.without_login())
        .read_json::<Value>()
        .await
        .unwrap();
    assert!(
        fixture.to("/api/user/alice")[0]
            .header("authorization")
            .is_none()
    );
}

#[tokio::test]
async fn a_form_post_sends_urlencoded_text() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let _: Value = client
        .post("/inbox/alice", &nobody(&client))
        .form(&[("text", "hi there & more")])
        .read_json()
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
fn urlencoded_escapes_as_a_form_does() {
    assert_eq!(urlencoded(&[("text", "a b&c*-._~")]), "text=a+b%26c*-._%7E");
}

#[tokio::test]
async fn ndjson_lines_reach_the_caller_while_the_stream_is_open() {
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let (lines_out, mut lines_in) = mpsc::unbounded_channel::<String>();
    let reading = tokio::spawn(async move {
        client
            .get("/api/stream/event", &nobody(&client).with_token("lip_x"))
            .lane(Lane::Live)
            .lines(
                || {},
                |line| {
                    let _ = lines_out.send(line.to_string());
                    Ok(())
                },
            )
            .await
    });
    send.send("{\"type\":\"gameStart\"}\n".into()).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), lines_in.recv())
        .await
        .expect("the first line arrives before the stream ends")
        .unwrap();
    assert_eq!(first, r#"{"type":"gameStart"}"#);
    drop(send);
    reading.await.unwrap().unwrap();
    assert_eq!(
        fixture.requests()[0].header("accept"),
        Some("application/x-ndjson")
    );
}

#[tokio::test]
async fn an_ndjson_error_is_logged_with_the_request() {
    let fixture = serve(|_| Reply::Text(404, "text/html", "<html>nope</html>".into())).await;
    let host = Arc::new(Recording::default());
    let client = client_for(&fixture.base, Arc::clone(&host));
    let error = client
        .get("/api/games/user/nobody", &nobody(&client))
        .lane(Lane::Export)
        .lines(|| {}, |_| Ok(()))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Lichess has no such thing.");
    assert!(
        host.logged()
            .contains("Lichess 404 from GET /api/games/user/nobody: not found"),
        "{}",
        host.logged()
    );
}

#[tokio::test]
async fn cancelling_an_open_stream_ends_the_read_with_an_abort() {
    let (_keep_open, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel();
    });
    let error = client
        .get("/api/stream/event", &nobody(&client))
        .lane(Lane::Live)
        .cancel(&cancel)
        .lines(|| {}, |_| Ok(()))
        .await
        .unwrap_err();
    assert!(error.aborted);
}

#[tokio::test]
async fn streamed_bytes_are_counted_against_the_account_and_kind_named() {
    let body = "{\"id\":\"abcdefgh\"}\n";
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let host = Arc::new(Recording::default());
    let client = client_for(&fixture.base, Arc::clone(&host));
    let auth = client.counted("Alice", UsageKind::Games);
    let reading = client
        .get("/api/games/user/Alice", &auth)
        .lane(Lane::Export)
        .lines(|| {}, |_| Ok(()));
    send.send(body.to_string()).unwrap();
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

/// A short policy deadline bounds headers but must not bound a live stream's body: the stream
/// only needs to outlive its own fixture deadline and still end when cancelled.
#[tokio::test]
async fn active_stream_outlives_its_header_deadline_and_remains_cancellable() {
    let (send, receive) = mpsc::unbounded_channel::<String>();
    let mut receive = Some(receive);
    let fixture = serve(move |_| Reply::Stream(receive.take().expect("one stream"))).await;
    let client = client_with_timeout(
        &fixture.base,
        Arc::new(Recording::default()),
        Duration::from_millis(500),
    );
    let cancel = CancellationToken::new();
    let (observed, mut observations) = mpsc::unbounded_channel();
    let reading = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            client
                .get("/api/tv/blitz/feed", &nobody(&client))
                .lane(Lane::Live)
                .cancel(&cancel)
                .lines(
                    || {},
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
    assert!(error.aborted);
}

#[tokio::test]
async fn ordinary_and_error_response_bodies_keep_their_deadline() {
    for (status, stream) in [(200, false), (503, true)] {
        let (_keep_open, receive) = mpsc::unbounded_channel::<String>();
        let mut receive = Some(receive);
        let fixture =
            serve(move |_| Reply::StreamText(status, "application/json", receive.take().unwrap()))
                .await;
        let client = client_with_timeout(
            &fixture.base,
            Arc::new(Recording::default()),
            Duration::from_millis(100),
        );
        let error = if stream {
            client
                .get("/api/stream/event", &nobody(&client))
                .lane(Lane::Live)
                .lines(|| {}, |_| Ok(()))
                .await
                .unwrap_err()
        } else {
            client
                .get("/api/account", &nobody(&client))
                .read_json::<Value>()
                .await
                .unwrap_err()
        };
        assert_eq!(error.message, "Lichess didn’t answer in time.");
        assert_eq!(error.kind, Some(ErrorKind::TimedOut));
    }
}

#[tokio::test]
async fn text_limit_counts_utf16_units_while_collecting_chunks() {
    use kchess_core::lichess::client::TextLimit;
    let fixture = serve(|_| Reply::Text(200, "text/plain", "a😀é".into())).await;
    let client = client_for(&fixture.base, Arc::new(Recording::default()));
    let read = |units| {
        client
            .get("/text", &nobody(&client))
            .accept("text/plain")
            .read_text(Some(TextLimit {
                utf16_units: units,
                message: "too large",
            }))
    };
    assert_eq!(read(4).await.unwrap(), "a😀é");
    assert_eq!(read(3).await.unwrap_err().message, "too large");
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
    let host = Arc::new(Recording::default());
    let client = client_for(&fixture.base, Arc::clone(&host));
    let error = client
        .get("/game/export/abcd1234", &nobody(&client))
        .accept("application/x-chess-pgn")
        .read_text(None)
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "Lichess isn’t working right now. Try again shortly."
    );
    assert!(
        host.logged()
            .contains("Lichess 200 from GET /game/export/abcd1234: unexpected page response"),
        "{}",
        host.logged()
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
    let client = Arc::new(client_for(&fixture.base, Arc::new(Recording::default())));
    let sync = CancellationToken::new();
    let syncing = {
        let client = Arc::clone(&client);
        let sync = sync.clone();
        tokio::spawn(async move {
            client
                .get("/api/games/user/Alice", &nobody(&client))
                .lane(Lane::Export)
                .cancel(&sync)
                .lines(|| {}, |_| Ok(()))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pgn = tokio::time::timeout(
        Duration::from_secs(2),
        client
            .post("/api/games/export/_ids", &nobody(&client))
            .accept("application/x-chess-pgn")
            .text("abcd1234")
            .read_text(None),
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
