//! Port of `tests/core/uci-lifecycle.test.ts`: UCI failure recovery, maintenance and the
//! exit guarantees, against the scripted fake engine (`tests/support/fake_uci.rs`).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use kchess_core::engine::uci::{UciController, search_cancelled};
use kchess_core::error::CoreError;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const HANDSHAKE: Duration = Duration::from_secs(2);

async fn cancel_after(token: &CancellationToken, delay: Duration) {
    tokio::time::sleep(delay).await;
    token.cancel();
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn bounds_a_silent_startup_and_terminates_its_process() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(
        &hub,
        support::fake("silent", None),
        Duration::from_millis(20),
    )
    .expect("spawn the fake engine");
    let error = engine.ready().await.unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
    tokio::time::timeout(Duration::from_secs(5), engine.closed())
        .await
        .expect("the timed-out engine process exits");
}

#[tokio::test]
async fn rejects_an_ignored_readiness_request() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(
        &hub,
        support::fake("no-readyok", None),
        Duration::from_millis(20),
    )
    .expect("spawn the fake engine");
    let error = engine.ready().await.unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
}

#[tokio::test]
async fn cancels_an_active_search_after_the_stop_acknowledgment() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("stop-ack", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");
    let token = CancellationToken::new();
    let (result, ()) = tokio::join!(
        engine.search("go infinite", |_line: &str| {}, None, Some(&token)),
        cancel_after(&token, Duration::from_millis(20)),
    );
    let error = result.unwrap_err();
    assert_eq!(error, search_cancelled());
    assert!(error.aborted, "a cancelled search is an AbortError");
    engine.close();
}

#[tokio::test]
async fn kills_a_search_that_ignores_stop_instead_of_waiting_indefinitely() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("stop-ignore", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");
    let token = CancellationToken::new();
    let (result, ()) = tokio::join!(
        engine.search("go infinite", |_line: &str| {}, None, Some(&token)),
        cancel_after(&token, Duration::from_millis(20)),
    );
    let error = result.unwrap_err();
    assert!(error.message.contains("did not stop"), "{}", error.message);
    tokio::time::timeout(Duration::from_secs(5), engine.closed())
        .await
        .expect("the engine that ignored stop is terminated");
}

#[tokio::test]
async fn rejects_a_bounded_search_with_no_bestmove() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("handshake", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");
    let error = engine
        .search(
            "go movetime 1",
            |_line: &str| {},
            Some(Duration::from_millis(20)),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
    assert!(
        host.contains("Engine command timed out: go timeoutMs=20"),
        "the timeout is logged with the command name only"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn waits_for_process_exit_before_replacement_and_blocks_new_engines_until_it_finishes() {
    let host = support::host();
    let hub = Arc::new(support::hub(&host));
    // The engine ignores SIGTERM: maintenance has to wait for the SIGKILL escalation.
    let engine = UciController::start(&hub, support::fake("ignore-term", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");

    let replaced = Arc::new(AtomicBool::new(false));
    let (release, released) = oneshot::channel::<()>();
    let operation = {
        let hub = Arc::clone(&hub);
        let replaced = Arc::clone(&replaced);
        tokio::spawn(async move {
            hub.with_maintenance(
                || {},
                move || async move {
                    replaced.store(true, Ordering::SeqCst);
                    let _ = released.await;
                    Ok::<(), CoreError>(())
                },
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !replaced.load(Ordering::SeqCst),
        "the replacement waits for the engine to exit"
    );
    let blocked = hub.assert_available().unwrap_err();
    assert_eq!(
        blocked.message,
        "Stockfish is being updated. Retry shortly."
    );

    wait_until("the replacement to start", || {
        replaced.load(Ordering::SeqCst)
    })
    .await;
    assert!(
        hub.assert_available().is_err(),
        "still blocked while the replacement runs"
    );
    release.send(()).expect("release the replacement");
    operation
        .await
        .expect("join")
        .expect("maintenance succeeds");
    hub.assert_available()
        .expect("available again after maintenance");

    let failed = hub
        .with_maintenance(
            || {},
            || async { Err::<(), _>(CoreError::new("replacement failed")) },
        )
        .await;
    assert_eq!(failed.unwrap_err().message, "replacement failed");
    hub.assert_available()
        .expect("a failed replacement also clears the flag");
}

#[cfg(unix)]
#[tokio::test]
async fn retains_the_executable_when_an_engine_will_not_exit_before_the_maintenance_deadline() {
    let host = support::host();
    // Shorter than the 1 s SIGKILL escalation, so the engine is still running at the deadline.
    let hub = support::hub(&host).with_exit_deadline(Duration::from_millis(100));
    let engine = UciController::start(&hub, support::fake("ignore-term", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");

    let calls = AtomicUsize::new(0);
    let operation = hub
        .with_maintenance(
            || {},
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok::<(), CoreError>(()) }
            },
        )
        .await;
    let error = operation.unwrap_err();
    assert!(
        error.message.contains("previous engine was retained"),
        "{}",
        error.message
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the executable is not replaced"
    );
    tokio::time::timeout(Duration::from_secs(5), engine.closed())
        .await
        .expect("the SIGKILL escalation ends the engine");
}
