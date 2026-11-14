//! Lichess request admission: the rate-limit, priority, deadline and accounting rules, over local
//! fixtures.

use crate::fixtures;

use std::sync::Arc;
use std::time::{Duration, Instant};

use fixtures::{Recording, Reply, http_client, serve};
use kchess_contracts::types::UsageKind;
use kchess_core::lichess::policy::{Lane, Policy, Usage};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

fn policy_with(host: Arc<Recording>, timeout: Duration) -> Arc<Policy> {
    Policy::with_timeout(
        host.clone(),
        host,
        http_client(),
        CancellationToken::new(),
        timeout,
    )
}

/// Traffic counted against nobody.
fn nobody(policy: &Policy) -> Usage {
    policy.usage("", UsageKind::Other)
}

/// Lets spawned requests reach the queue before the next step.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(60)).await;
}

#[tokio::test]
async fn runs_one_export_at_a_time_until_its_body_ends_without_blocking_other_requests() {
    let (chunks, first_body) = mpsc::unbounded_channel::<String>();
    let mut first_body = Some(first_body);
    let fixture = serve(move |seen| {
        if seen.path() == "/api/games/user/Alice" {
            Reply::Stream(first_body.take().expect("one export stream"))
        } else {
            Reply::Json(200, "{}".into())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let policy = policy_with(host, Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();

    let first = policy
        .send(
            client
                .get(format!("{}/api/games/user/Alice", fixture.base))
                .build()
                .unwrap(),
            Lane::Export,
            &nobody(&policy),
            &cancel,
        )
        .await
        .unwrap();
    let mut response = first.response;
    let hold = first.hold;
    chunks.send("{}\n".into()).unwrap();
    let chunk = response.chunk().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&chunk).contains("{}"));

    // A second export waits for the first body to end.
    let second_policy = Arc::clone(&policy);
    let second_cancel = cancel.clone();
    let second_request = client
        .post(format!("{}/api/games/export/_ids", fixture.base))
        .body("abcdefgh")
        .build()
        .unwrap();
    let second = tokio::spawn(async move {
        let usage = nobody(&second_policy);
        second_policy
            .send(second_request, Lane::Export, &usage, &second_cancel)
            .await
            .map(|admitted| admitted.response.status().as_u16())
    });
    settle().await;
    assert!(!second.is_finished());

    // Interactive traffic is not held behind the open export.
    let account = policy
        .send(
            client
                .get(format!("{}/api/account", fixture.base))
                .build()
                .unwrap(),
            Lane::Background,
            &nobody(&policy),
            &cancel,
        )
        .await
        .unwrap();
    assert!(account.response.status().is_success());

    drop(chunks);
    while response.chunk().await.unwrap().is_some() {}
    drop(hold);
    let status = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("second export starts once the first body ends")
        .unwrap()
        .unwrap();
    assert_eq!(status, 200);
}

#[tokio::test]
async fn releases_the_export_lane_when_a_body_is_cancelled_or_the_request_fails() {
    let (chunks, first_body) = mpsc::unbounded_channel::<String>();
    let mut first_body = Some(first_body);
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        match calls {
            1 => Reply::Stream(first_body.take().expect("one export stream")),
            2 => Reply::Text(500, "text/plain", "unavailable".into()),
            _ => Reply::Json(200, "{}\n".into()),
        }
    })
    .await;
    let policy = policy_with(Arc::new(Recording::default()), Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();
    let export = |base: &str| {
        client
            .get(format!("{base}/api/games/user/Alice"))
            .build()
            .unwrap()
    };

    // A body dropped before it ends frees the lane.
    let admitted = policy
        .send(
            export(&fixture.base),
            Lane::Export,
            &nobody(&policy),
            &cancel,
        )
        .await
        .unwrap();
    assert!(admitted.hold.is_some());
    drop(admitted);
    drop(chunks);

    // A request that never connects frees it too.
    let failed = policy
        .send(
            export("http://127.0.0.1:1"),
            Lane::Export,
            &nobody(&policy),
            &cancel,
        )
        .await;
    assert!(failed.is_err());

    // A failed status is answered and keeps no lane.
    let refused = policy
        .send(
            export(&fixture.base),
            Lane::Export,
            &nobody(&policy),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(refused.response.status().as_u16(), 500);
    assert!(refused.hold.is_none());

    let ok = tokio::time::timeout(
        Duration::from_secs(5),
        policy.send(
            export(&fixture.base),
            Lane::Export,
            &nobody(&policy),
            &cancel,
        ),
    )
    .await
    .expect("the lane is free")
    .unwrap();
    assert!(ok.response.status().is_success());
}

#[tokio::test]
async fn a_rate_limit_cools_the_next_request_down_for_retry_after() {
    let mut calls = 0;
    let fixture = serve(move |_| {
        calls += 1;
        if calls == 1 {
            Reply::WithHeaders(429, vec![("Retry-After", "1".into())], "slow down".into())
        } else {
            Reply::Json(200, "{}".into())
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let policy = policy_with(Arc::clone(&host), Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();
    let account = || {
        client
            .get(format!("{}/api/account?token=lip_secret", fixture.base))
            .build()
            .unwrap()
    };

    let limited = policy
        .send(account(), Lane::Background, &nobody(&policy), &cancel)
        .await
        .unwrap();
    assert_eq!(limited.response.status().as_u16(), 429);

    let started = Instant::now();
    // A cooldown whose clock stands still would never end.
    let next = tokio::time::timeout(
        Duration::from_secs(10),
        policy.send(account(), Lane::Background, &nobody(&policy), &cancel),
    )
    .await
    .expect("the cooldown ends")
    .unwrap();
    assert!(next.response.status().is_success());
    assert!(started.elapsed() >= Duration::from_millis(900));

    let logged = host.logged();
    assert!(logged.contains("Lichess rate limited: /api/account retryAfter=1 cooldownMs=1000"));
    assert!(
        !logged.contains("lip_secret"),
        "the query never reaches the log"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_game_moves_go_before_background_reads() {
    // Each slow request has a gate of its own, so one slot is freed at a time: freed together,
    // the move and the read ran side by side and reached the server in either order.
    let (release_one, gate_one) = watch::channel(false);
    let (release_two, gate_two) = watch::channel(false);
    let fixture = serve(move |seen| {
        if seen.path() == "/api/hold/one" {
            Reply::Gated(gate_one.clone(), Box::new(Reply::Json(200, "{}".into())))
        } else if seen.path() == "/api/hold/two" {
            Reply::Gated(gate_two.clone(), Box::new(Reply::Json(200, "{}".into())))
        } else {
            Reply::Json(200, "{}".into())
        }
    })
    .await;
    let policy = policy_with(Arc::new(Recording::default()), Duration::from_secs(5));
    let client = http_client();
    let base = fixture.base.clone();

    // Two slow requests take both slots.
    let mut holds = Vec::new();
    for name in ["one", "two"] {
        let policy = Arc::clone(&policy);
        let request = client
            .get(format!("{base}/api/hold/{name}"))
            .build()
            .unwrap();
        holds.push(tokio::spawn(async move {
            let usage = nobody(&policy);
            policy
                .send(request, Lane::Background, &usage, &CancellationToken::new())
                .await
                .map(|_| ())
        }));
    }
    settle().await;

    let background_policy = Arc::clone(&policy);
    let background_request = client.get(format!("{base}/api/account")).build().unwrap();
    let background = tokio::spawn(async move {
        let usage = nobody(&background_policy);
        background_policy
            .send(
                background_request,
                Lane::Background,
                &usage,
                &CancellationToken::new(),
            )
            .await
            .map(|_| ())
    });
    settle().await;
    let move_policy = Arc::clone(&policy);
    let move_request = client
        .post(format!("{base}/api/board/game/abcdefgh/move/e2e4"))
        .build()
        .unwrap();
    let game_move = tokio::spawn(async move {
        let usage = nobody(&move_policy);
        move_policy
            .send(
                move_request,
                Lane::Interactive,
                &usage,
                &CancellationToken::new(),
            )
            .await
            .map(|_| ())
    });
    settle().await;

    // The one freed slot goes to the move, though the read queued before it; the read gets
    // the slot only once the move's answer has come back.
    release_one.send(true).unwrap();
    game_move.await.unwrap().unwrap();
    release_two.send(true).unwrap();
    for handle in holds.into_iter().chain([background]) {
        handle.await.unwrap().unwrap();
    }
    let order: Vec<String> = fixture
        .requests()
        .into_iter()
        .map(|seen| seen.path().to_string())
        .collect();
    let game_at = order.iter().position(|p| p.contains("/move/")).unwrap();
    let account_at = order.iter().position(|p| p == "/api/account").unwrap();
    assert!(
        game_at < account_at,
        "the move was admitted first: {order:?}"
    );
}

#[tokio::test]
async fn a_request_that_never_answers_fails_at_the_deadline() {
    let fixture = serve(|_| Reply::Hang).await;
    let policy = policy_with(Arc::new(Recording::default()), Duration::from_millis(100));
    let started = Instant::now();
    let error = policy
        .send(
            http_client()
                .get(format!("{}/api/account", fixture.base))
                .build()
                .unwrap(),
            Lane::Background,
            &nobody(&policy),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.message, "Lichess didn’t answer in time.");
    assert_eq!(error.kind, Some(kchess_contracts::ErrorKind::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_cancelled_request_ends_with_an_abort() {
    let fixture = serve(|_| Reply::Hang).await;
    let policy = policy_with(Arc::new(Recording::default()), Duration::from_secs(30));
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        canceller.cancel();
    });
    let error = policy
        .send(
            http_client()
                .get(format!("{}/api/account", fixture.base))
                .build()
                .unwrap(),
            Lane::Background,
            &nobody(&policy),
            &cancel,
        )
        .await
        .unwrap_err();
    assert!(error.aborted);
}

#[tokio::test]
async fn usage_is_counted_against_the_account_and_kind_the_caller_names() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let host = Arc::new(Recording::default());
    let policy = policy_with(Arc::clone(&host), Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();
    let request = client
        .get(format!("{}/api/account", fixture.base))
        .build()
        .unwrap();
    let usage = policy.usage("Alice", UsageKind::Games);
    policy
        .send(request, Lane::Background, &usage, &cancel)
        .await
        .unwrap();
    let usage = host.usage_events();
    assert!(
        usage.contains(&serde_json::json!({
            "account": "alice",
            "category": "games",
            "requests": 1,
            "bytes": 0,
        })),
        "{usage:?}"
    );
}

#[tokio::test]
async fn traffic_of_a_forgotten_account_is_not_counted() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let host = Arc::new(Recording::default());
    let policy = policy_with(Arc::clone(&host), Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();
    let request = client
        .get(format!("{}/api/account", fixture.base))
        .build()
        .unwrap();
    let usage = policy.usage("Alice", UsageKind::Games);
    policy.forget_usage(&["ALICE".to_string()]);
    policy
        .send(request, Lane::Background, &usage, &cancel)
        .await
        .unwrap();
    assert!(
        host.usage_events()
            .iter()
            .all(|event| event["account"] != "alice"),
        "logged out traffic must not be counted"
    );
}

/// Once the core is closing, traffic still arriving is no longer counted.
#[tokio::test]
async fn traffic_after_the_core_closed_is_not_counted() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let host = Arc::new(Recording::default());
    let lifetime = CancellationToken::new();
    let policy = Policy::with_timeout(
        host.clone(),
        host.clone(),
        http_client(),
        lifetime.clone(),
        Duration::from_secs(5),
    );
    lifetime.cancel();
    let request = http_client()
        .get(format!("{}/api/account", fixture.base))
        .build()
        .unwrap();
    let usage = policy.usage("Alice", UsageKind::Games);
    // The request has a cancellation of its own, so it still runs.
    policy
        .send(request, Lane::Background, &usage, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fixture.requests().len(), 1);
    assert_eq!(host.usage_events(), Vec::<serde_json::Value>::new());
}

/// A page showed reqwest's "error sending request for url (…)" when Lichess could not be reached.
#[tokio::test]
async fn an_unreachable_server_is_said_in_plain_words() {
    let host = Arc::new(Recording::default());
    let policy = policy_with(host, Duration::from_secs(5));
    let request = http_client()
        .get("http://127.0.0.1:9/api/puzzle/daily")
        .build()
        .unwrap();
    let Err(failure) = policy
        .send(
            request,
            Lane::Background,
            &nobody(&policy),
            &CancellationToken::new(),
        )
        .await
    else {
        panic!("nothing listens on the discard port");
    };
    assert_eq!(failure.message, "Couldn’t reach Lichess");
    assert_eq!(failure.kind, Some(kchess_contracts::ErrorKind::Offline));
}

/// A gzip body as a compressing server sends it.
fn gzipped(text: &str) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn ordinary_calls_accept_gzip_and_live_streams_ask_for_an_uncompressed_body() {
    let account = r#"{"id":"alice","username":"Alice"}"#;
    let fixture = serve(move |seen| {
        if seen.path() == "/api/account" {
            Reply::Bytes(
                200,
                vec![
                    ("Content-Type", "application/json".into()),
                    ("Content-Encoding", "gzip".into()),
                ],
                gzipped(account),
            )
        } else {
            Reply::Json(200, "{}".into())
        }
    })
    .await;
    let policy = policy_with(Arc::new(Recording::default()), Duration::from_secs(5));
    let client = http_client();
    let cancel = CancellationToken::new();
    let send = |path: &str, lane: Lane| {
        let request = client
            .get(format!("{}{path}", fixture.base))
            .build()
            .unwrap();
        let policy = Arc::clone(&policy);
        let cancel = cancel.clone();
        async move {
            let usage = nobody(&policy);
            policy
                .send(request, lane, &usage, &cancel)
                .await
                .unwrap()
                .response
        }
    };

    // The compressed answer arrives decoded.
    let answer = send("/api/account", Lane::Background).await;
    assert_eq!(answer.text().await.unwrap(), account);
    let encoding = |path: &str| {
        fixture.to(path)[0]
            .header("accept-encoding")
            .map(str::to_string)
    };
    assert!(encoding("/api/account").is_some_and(|value| value.contains("gzip")));

    // A live stream must not wait on a compressor's buffer; a bulk export may.
    let _ = send("/api/stream/event", Lane::Live).await.text().await;
    assert_eq!(encoding("/api/stream/event").as_deref(), Some("identity"));
    let _ = send("/api/games/user/Alice", Lane::Export)
        .await
        .text()
        .await;
    assert!(encoding("/api/games/user/Alice").is_some_and(|value| value.contains("gzip")));
}
