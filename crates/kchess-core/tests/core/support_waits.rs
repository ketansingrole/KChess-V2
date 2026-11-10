//! The tests' shared waits fail the test when what they wait for never comes. A wait that ran out
//! and carried on let a test go on in a state it never reached: the Chess.com removal tests then
//! failed far from the cause on a loaded Windows runner, or passed without testing their race.

use std::time::Duration;

use crate::fixtures::{Reply, http_client, serve};
use crate::support;

#[tokio::test]
async fn a_request_wait_returns_the_requests_once_they_arrive() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    let base = fixture.base.clone();
    let asking = tokio::spawn(async move {
        for _ in 0..2 {
            let _ = http_client().get(format!("{base}/asked?n=1")).send().await;
        }
    });
    let seen = fixture.requested("/asked", 2).await;
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter()
            .all(|seen| seen.query("n").as_deref() == Some("1"))
    );
    asking.await.unwrap();
}

#[tokio::test]
#[should_panic(expected = "waiting for the request that never comes; requested: [\"/other\"]")]
async fn a_request_that_never_comes_fails_the_wait() {
    let fixture = serve(|_| Reply::Json(200, "{}".into())).await;
    http_client()
        .get(format!("{}/other", fixture.base))
        .send()
        .await
        .unwrap();
    fixture
        .wait_for_within(
            Duration::from_millis(50),
            "the request that never comes",
            |requests| requests.iter().any(|seen| seen.path() == "/asked"),
        )
        .await;
}

#[tokio::test]
#[should_panic(expected = "timed out waiting for a condition that never holds")]
async fn a_condition_that_never_holds_fails_the_wait() {
    support::wait_until(
        Duration::from_millis(50),
        "a condition that never holds",
        || None::<()>,
    )
    .await;
}
