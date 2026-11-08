//! The facade's challenge answers for a signed-in account: a challenge from the account's event
//! stream is answered on Lichess and then leaves the inbox.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::fixtures;
use crate::signed_in;

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
    let sender = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(sender) = stream.lock().unwrap().clone() {
                return sender;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the event stream opens");
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

    core.call(
        "declineChallenge",
        vec![json!("Chal1234"), json!("generic")],
    )
    .await
    .unwrap();
    assert_eq!(fixture.to("/api/challenge/Chal1234/decline").len(), 1);
    assert_eq!(core.call("challenges", vec![]).await.unwrap(), json!([]));
}
