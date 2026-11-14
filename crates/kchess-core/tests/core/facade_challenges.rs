//! The facade's challenge answers for a signed-in account: a challenge from the account's event
//! stream is answered on Lichess and then leaves the inbox.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::fixtures;
use crate::signed_in;
use crate::support;

use fixtures::{Reply, serve};
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declined_challenge_is_answered_and_leaves_the_inbox() {
    let stream: Arc<Mutex<Option<UnboundedSender<String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&stream);
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/api/challenge/Chal1234/decline" => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    core.call("stayConnected", vec![json!("Alice")])
        .await
        .unwrap();
    let sender = support::wait_until(support::UNTIL_LIMIT, "the event stream to open", || {
        stream.lock().unwrap().clone()
    })
    .await;
    let challenge = json!({
        "type": "challenge",
        "challenge": {
            "id": "Chal1234",
            "challenger": { "id": "bob", "name": "Bob", "rating": 1700 },
            "destUser": { "id": "alice", "name": "Alice", "rating": 1650 },
            "variant": { "key": "standard", "name": "Standard" },
            "rated": false,
            "speed": "rapid",
            "timeControl": { "type": "clock", "limit": 600, "increment": 0 },
            "color": "random"
        },
        "compat": { "board": true }
    });
    sender.send(format!("{challenge}\n")).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while core.call("challenges", vec![]).await.unwrap() == json!([]) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the challenge reaches the inbox");
    // The time it arrived is the clock's: an integer, recorded as 0.
    let mut listed = core.call("challenges", vec![]).await.unwrap();
    assert!(listed[0]["receivedAt"].is_i64());
    listed[0]["receivedAt"] = json!(0);
    crate::support::wire::assert_wire("challenges.facade", &listed);

    core.call(
        "declineChallenge",
        vec![json!("Chal1234"), json!("generic")],
    )
    .await
    .unwrap();
    assert_eq!(fixture.to("/api/challenge/Chal1234/decline").len(), 1);
    assert_eq!(core.call("challenges", vec![]).await.unwrap(), json!([]));
}

/// Only a challenge sent to the account that KChess can play is accepted, and only one the
/// account sent is withdrawn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_playable_incoming_challenge_is_accepted_and_only_an_outgoing_one_withdrawn() {
    let stream: Arc<Mutex<Option<UnboundedSender<String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&stream);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        (_, "/api/stream/event") => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        (_, "/api/account/playing") => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        ("POST", _) => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    core.call("stayConnected", vec![json!("Alice")])
        .await
        .unwrap();
    let sender = support::wait_until(support::UNTIL_LIMIT, "the event stream to open", || {
        stream.lock().unwrap().clone()
    })
    .await;
    let alice = json!({ "id": "alice", "name": "Alice", "rating": 1650 });
    let bob = json!({ "id": "bob", "name": "Bob", "rating": 1700 });
    for (id, from, to, variant) in [
        ("Incoming1", &bob, &alice, "standard"),
        ("Bughouse1", &bob, &alice, "bughouse"),
        ("Outgoing1", &alice, &bob, "standard"),
    ] {
        let challenge = json!({
            "type": "challenge",
            "challenge": {
                "id": id, "challenger": from, "destUser": to,
                "variant": { "key": variant, "name": variant },
                "rated": false, "speed": "rapid",
                "timeControl": { "type": "clock", "limit": 600, "increment": 0 },
                "color": "random"
            },
            "compat": { "board": true }
        });
        sender.send(format!("{challenge}\n")).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        while core
            .call("challenges", vec![])
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len()
            < 3
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the challenges reach the inbox");

    for (method, id, refusal) in [
        ("acceptChallenge", "Bughouse1", "bughouse"),
        (
            "acceptChallenge",
            "Outgoing1",
            "Only challenges sent to you can be accepted.",
        ),
        (
            "cancelChallenge",
            "Incoming1",
            "Decline a challenge sent to you instead.",
        ),
    ] {
        let refused = core.call(method, vec![json!(id)]).await.unwrap_err();
        assert!(
            refused.message.contains(refusal),
            "{method} {id}: {}",
            refused.message
        );
    }
    assert!(
        fixture
            .requests()
            .iter()
            .all(|seen| !seen.path().starts_with("/api/challenge/")),
        "nothing was answered"
    );
    core.call("cancelChallenge", vec![json!("Outgoing1")])
        .await
        .unwrap();
    assert_eq!(fixture.to("/api/challenge/Outgoing1/cancel").len(), 1);
    core.call("acceptChallenge", vec![json!("Incoming1")])
        .await
        .unwrap();
    assert_eq!(fixture.to("/api/challenge/Incoming1/accept").len(), 1);
    let left = core.call("challenges", vec![]).await.unwrap();
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
    assert_eq!(left[0]["id"], json!("Bughouse1"));
    core.close().await;
}
