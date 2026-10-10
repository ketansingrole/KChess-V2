//! The online-play session (`core/src/services/lichess.ts`'s `OnlineSession`) over local fixtures.
//! Ports `core/tests/unit/online-session.test.ts` (recovery, protection, attachments, cancellation)
//! and `core/tests/unit/online-actions.test.ts` (actions, chat, lobby, challenges, seeks), plus the
//! scenarios the brief names that the TypeScript suites do not cover in full: a move is sent once,
//! a dropped game stream reconnects, and a tournament join and withdrawal use the account's login.

#[path = "support/lichess_http.rs"]
mod fixtures;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{MemoryStore, Recording, Reply, Seen, capabilities_for, http_client, serve};
use futures_util::future::BoxFuture;
use kchess_core::error::{CoreError, Result as CoreResult};
use kchess_core::host::Host;
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::challenges::{ChallengeInfo, PlayerRef, TimeControl};
use kchess_core::lichess::client::{LichessClient, TokenSource};
use kchess_core::lichess::online::{OnlineAction, OnlineOptions, OnlineSession, Resumed};
use kchess_core::lichess::policy::Policy;
use serde_json::{Number, Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// One scripted answer of the token source (`getToken`).
#[derive(Clone)]
enum Script {
    /// The token, or none when the account has no login.
    Now(Option<String>),
    /// Waits for the gate, then answers.
    Gate(watch::Receiver<bool>, Option<String>),
    /// The credential store fails.
    Fail(String),
}

/// Tokens per account. Each account answers with its queued scripts in order; the last one
/// repeats. An account with nothing queued answers `<account>-token`.
#[derive(Default)]
struct ScriptedTokens {
    queued: Mutex<HashMap<String, VecDeque<Script>>>,
    calls: Mutex<HashMap<String, usize>>,
}

impl ScriptedTokens {
    fn set(&self, account: &str, script: Script) {
        self.queued
            .lock()
            .unwrap()
            .insert(account.to_lowercase(), VecDeque::from([script]));
    }

    fn queue(&self, account: &str, scripts: Vec<Script>) {
        self.queued
            .lock()
            .unwrap()
            .insert(account.to_lowercase(), VecDeque::from(scripts));
    }

    fn calls_for(&self, account: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .get(&account.to_lowercase())
            .copied()
            .unwrap_or(0)
    }
}

impl TokenSource for ScriptedTokens {
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, CoreResult<Option<String>>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            *self.calls.lock().unwrap().entry(key.clone()).or_default() += 1;
            let script = {
                let mut queued = self.queued.lock().unwrap();
                match queued.get_mut(&key) {
                    Some(queue) if queue.len() > 1 => queue.pop_front(),
                    Some(queue) => queue.front().cloned(),
                    None => None,
                }
            };
            match script.unwrap_or_else(|| Script::Now(Some(format!("{account}-token")))) {
                Script::Now(token) => Ok(token),
                Script::Gate(mut gate, token) => {
                    while !*gate.borrow() {
                        if gate.changed().await.is_err() {
                            break;
                        }
                    }
                    Ok(token)
                }
                Script::Fail(message) => Err(CoreError::new(message)),
            }
        })
    }
}

fn policy(host: &Arc<Recording>) -> Arc<Policy> {
    Policy::with_timeout(
        Arc::clone(host) as Arc<dyn Host>,
        http_client(),
        CancellationToken::new(),
        Duration::from_secs(5),
    )
}

/// A session over the fixture at `base`, with `store`'s accounts and `tokens`.
fn session(
    base: &str,
    host: &Arc<Recording>,
    store: &Arc<MemoryStore>,
    tokens: &Arc<ScriptedTokens>,
) -> OnlineSession {
    let client = LichessClient::with_base(base, policy(host), http_client());
    let lichess = Arc::new(Lichess::new(
        client,
        capabilities_for(host),
        Arc::clone(store) as Arc<dyn kchess_core::lichess::accounts::LichessStore>,
        Arc::clone(tokens) as Arc<dyn TokenSource>,
        Arc::clone(host) as Arc<dyn Host>,
        CancellationToken::new(),
    ));
    OnlineSession::new(
        lichess,
        Arc::clone(tokens) as Arc<dyn TokenSource>,
        http_client(),
    )
}

fn rig(accounts: &[(&str, bool)]) -> (Arc<Recording>, Arc<MemoryStore>, Arc<ScriptedTokens>) {
    (
        Arc::new(Recording::default()),
        MemoryStore::with_accounts(accounts),
        Arc::new(ScriptedTokens::default()),
    )
}

/// Polls until `check` holds, or fails the test after a few seconds.
async fn until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met: {what}");
}

/// Every payload the host received for `event`, in order.
fn emitted(host: &Recording, event: &str) -> Vec<Value> {
    host.events
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == event)
        .map(|(_, payload)| payload.clone())
        .collect()
}

fn states(host: &Recording) -> Vec<Value> {
    emitted(host, "online:state")
}

fn has_state(host: &Recording, phase: &str, game: &str) -> bool {
    states(host)
        .iter()
        .any(|state| state["phase"] == phase && state["gameId"] == game)
}

async fn wait_requests(fixture: &fixtures::Fixture, path: &str, count: usize) {
    let path = path.to_string();
    until(&format!("{count} requests to {path}"), || {
        fixture.to(&path).len() >= count
    })
    .await;
}

/// The default answer of the account and game routes: a game of Alice's, and every stream
/// refused (`401`), as the TypeScript suite's mocks answer.
fn refuse(seen: &Seen) -> Reply {
    Reply::Json(
        401,
        json!({ "error": format!("refused {}", seen.path()) }).to_string(),
    )
}

fn incoming(overrides: Value) -> ChallengeInfo {
    let mut info = ChallengeInfo {
        id: "Chal1234".into(),
        account: "Alice".into(),
        direction: "in".into(),
        opponent: PlayerRef {
            name: "Bob".into(),
            rating: None,
            title: None,
            provisional: None,
            online: None,
        },
        variant: "standard".into(),
        variant_name: "Standard".into(),
        rated: false,
        speed: "rapid".into(),
        time_control: TimeControl::Clock {
            limit: Number::from(600),
            increment: Number::from(0),
        },
        color: "random".into(),
        rematch_of: None,
        initial_fen: None,
        playable: true,
        problem: None,
        received_at: 0,
    };
    if let Some(id) = overrides.get("id").and_then(Value::as_str) {
        info.id = id.into();
    }
    if overrides.get("correspondence").is_some() {
        info.time_control = TimeControl::Correspondence {
            days: Number::from(3),
        };
    }
    info
}

fn seek_options(target: Option<&str>) -> OnlineOptions {
    OnlineOptions {
        minutes: 10.0,
        increment: 0.0,
        days: None,
        variant: None,
        fen: None,
        color: "random".into(),
        rated: false,
        target: target.map(str::to_string),
        account: None,
    }
}

/// The form fields of a request body, decoded.
fn form(seen: &Seen) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://fixture.invalid/?{}", seen.body))
        .expect("form body")
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn field(seen: &Seen, name: &str) -> Option<String> {
    form(seen)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/* ── online-session.test.ts ───────────────────────────────────────────── */

#[tokio::test]
async fn keeps_assistance_blocked_after_a_failed_game_stream_and_cancellation_until_the_server_confirms_no_game()
 {
    let playing = Arc::new(Mutex::new(
        json!({ "nowPlaying": [{ "gameId": "AbCd1234" }] }),
    ));
    let shared = Arc::clone(&playing);
    let fixture = serve(move |seen| match seen.path() {
        "/api/account/playing" => Reply::Json(200, shared.lock().unwrap().to_string()),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);

    assert_eq!(
        online.resume().await.unwrap(),
        Some(Resumed {
            id: "AbCd1234".into(),
            account: "Alice".into()
        })
    );
    until("auth-required", || {
        has_state(&host, "auth-required", "AbCd1234")
    })
    .await;
    assert!(online.playing());
    online.cancel();
    assert!(
        online.playing(),
        "a cancelled session keeps the game protected"
    );

    *playing.lock().unwrap() = json!({ "nowPlaying": [] });
    assert_eq!(online.resume().await.unwrap(), None);
    assert!(!online.playing());
    online.close();
}

#[tokio::test]
async fn aborts_an_outstanding_challenge_and_sends_no_more_once_cancelled() {
    let (gate_tx, gate) = watch::channel(false);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/challenge/Bob") => Reply::Gated(
            gate.clone(),
            Box::new(Reply::Json(
                200,
                json!({ "id": "AbCd1234", "url": "https://lichess.org/AbCd1234" }).to_string(),
            )),
        ),
        ("POST", _) => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let starting = online.clone();
    let result = tokio::spawn(async move { starting.start(&seek_options(Some("Bob"))).await });
    wait_requests(&fixture, "/api/challenge/Bob", 1).await;
    online.cancel();
    // Lichess answers after the cancel: the late challenge is still cancelled, once.
    let _ = gate_tx.send(true);
    let outcome = result.await.unwrap();
    assert!(outcome.as_ref().is_err_and(|cause| cause.is_aborted()));
    wait_requests(&fixture, "/api/challenge/AbCd1234/cancel", 1).await;
    assert_eq!(fixture.to("/api/challenge/Bob").len(), 1);
    assert_eq!(fixture.to("/api/challenge/AbCd1234/cancel").len(), 1);
    online.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.to("/api/challenge/Bob").len(), 1);
    assert_eq!(fixture.to("/api/challenge/AbCd1234/cancel").len(), 1);
    online.close();
}

#[tokio::test]
async fn prevents_streams_and_challenges_when_cancelled_before_credentials_arrive() {
    let fixture = serve(|_| Reply::Json(200, json!({ "ok": true }).to_string())).await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let (login_tx, login) = watch::channel(false);
    tokens.set("Alice", Script::Gate(login, Some("Alice-token".into())));
    let online = session(&fixture.base, &host, &store, &tokens);
    let starting = online.clone();
    let result = tokio::spawn(async move { starting.start(&seek_options(Some("Bob"))).await });
    until("credentials requested", || tokens.calls_for("Alice") >= 1).await;
    online.cancel();
    let _ = login_tx.send(true);
    let outcome = result.await.unwrap();
    assert!(outcome.is_err_and(|cause| cause.is_aborted()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        fixture.requests().is_empty(),
        "nothing was opened or posted"
    );
    online.close();
}

#[tokio::test]
async fn retains_live_game_protection_when_the_owning_credentials_or_account_disappear() {
    let fixture = serve(move |seen| match seen.path() {
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234", "speed": "rapid" }] }).to_string(),
        ),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.resume().await.unwrap();
    until("auth-required", || {
        has_state(&host, "auth-required", "AbCd1234")
    })
    .await;
    online.cancel();
    tokens.set("Alice", Script::Now(None));
    store.accounts.lock().unwrap().clear();
    let before = fixture.requests().len();
    let error = online.resume().await.unwrap_err();
    assert!(error.message().contains("login is unavailable"));
    assert_eq!(fixture.requests().len(), before, "no request was made");
    assert!(online.playing());
    let last = states(&host).pop().unwrap();
    assert_eq!(
        (
            last["phase"].as_str(),
            last["gameId"].as_str(),
            last["account"].as_str()
        ),
        (Some("auth-required"), Some("AbCd1234"), Some("Alice"))
    );
    online.close();
}

#[tokio::test]
async fn ignores_a_cancelled_resume_before_missing_credentials_arrive() {
    let fixture = serve(|_| Reply::Json(200, json!({ "nowPlaying": [] }).to_string())).await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let (token_tx, token) = watch::channel(false);
    tokens.set("Alice", Script::Gate(token, None));
    let online = session(&fixture.base, &host, &store, &tokens);
    let resuming = online.clone();
    let pending = tokio::spawn(async move { resuming.resume().await });
    until("credentials requested", || tokens.calls_for("Alice") >= 1).await;
    online.cancel();
    let count = states(&host).len();
    let _ = token_tx.send(true);
    let outcome = pending.await.unwrap();
    assert!(outcome.is_err_and(|cause| cause.is_aborted()));
    assert_eq!(states(&host).len(), count);
    assert!(fixture.to("/api/account/playing").is_empty());
    online.close();
}

#[tokio::test]
async fn blocks_startup_assistance_through_pending_and_failed_verification_then_clears_after_an_authoritative_retry()
 {
    let (release, gate) = watch::channel(false);
    let answers = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&answers);
    let fixture = serve(move |seen| {
        if seen.path() != "/api/account/playing" {
            return refuse(seen);
        }
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::Gated(
                gate.clone(),
                Box::new(Reply::Json(503, json!({ "error": "down" }).to_string())),
            )
        } else {
            Reply::Json(200, json!({ "nowPlaying": [] }).to_string())
        }
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    assert!(online.assistance_blocked());
    assert!(!online.playing());
    let resuming = online.clone();
    let pending = tokio::spawn(async move { resuming.resume().await });
    until("checking", || has_state(&host, "checking", "")).await;
    assert!(online.assistance_blocked());
    let _ = release.send(true);
    assert!(pending.await.unwrap().is_err());
    assert!(online.assistance_blocked());
    assert_eq!(states(&host).last().unwrap()["phase"], "disconnected");
    assert_eq!(online.resume().await.unwrap(), None);
    assert!(!online.assistance_blocked());
    online.close();
}

#[tokio::test]
async fn clears_startup_protection_immediately_with_no_connected_accounts() {
    let fixture = serve(|_| Reply::Json(200, json!({ "nowPlaying": [] }).to_string())).await;
    let (host, store, tokens) = rig(&[]);
    let online = session(&fixture.base, &host, &store, &tokens);
    assert_eq!(online.resume().await.unwrap(), None);
    assert!(!online.assistance_blocked());
    assert!(fixture.requests().is_empty());
    online.close();
}

#[tokio::test]
async fn keeps_startup_protection_when_any_account_is_unverified_or_the_response_is_malformed() {
    let phase = Arc::new(AtomicUsize::new(0));
    let switch = Arc::clone(&phase);
    let fixture = serve(move |seen| {
        if seen.path() != "/api/account/playing" {
            return refuse(seen);
        }
        let stage = switch.load(Ordering::SeqCst);
        match (stage, seen.header("Authorization")) {
            (0, Some("Bearer Bob-token")) => {
                Reply::Json(503, json!({ "error": "offline" }).to_string())
            }
            (0, _) => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
            _ => Reply::Json(200, json!({}).to_string()),
        }
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true), ("Bob", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    assert!(online.resume().await.is_err());
    assert!(online.assistance_blocked());
    phase.store(1, Ordering::SeqCst);
    let error = online.resume().await.unwrap_err();
    assert!(error.message().contains("invalid ongoing-game response"));
    assert!(online.assistance_blocked());
    online.close();
}

#[tokio::test]
async fn forgets_account_and_game_recovery_on_explicit_logout_and_rejects_a_late_stream() {
    let fixture = serve(move |seen| match seen.path() {
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234" }] }).to_string(),
        ),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.resume().await.unwrap();
    until("auth-required", || {
        has_state(&host, "auth-required", "AbCd1234")
    })
    .await;
    online.logout(None);
    assert!(!online.playing());
    assert!(!online.assistance_blocked());
    let last = states(&host).pop().unwrap();
    assert_eq!(
        (
            last["account"].as_str(),
            last["gameId"].as_str(),
            last["phase"].as_str()
        ),
        (Some(""), Some(""), Some("idle"))
    );
    store.accounts.lock().unwrap().clear();
    assert_eq!(online.resume().await.unwrap(), None);
    assert!(!online.assistance_blocked());
    online.close();
}

#[tokio::test]
async fn preserves_another_accounts_playing_session_when_logging_out_an_unrelated_account() {
    let fixture = serve(move |seen| match seen.path() {
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234" }] }).to_string(),
        ),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.resume().await.unwrap();
    until("auth-required", || {
        has_state(&host, "auth-required", "AbCd1234")
    })
    .await;
    let count = states(&host).len();
    online.logout(Some(&["Bob".to_string()]));
    assert!(online.playing());
    assert!(online.assistance_blocked());
    assert_eq!(states(&host).len(), count);
    online.logout(Some(&["Alice".to_string()]));
    assert!(!online.playing());
    let last = states(&host).pop().unwrap();
    assert_eq!(last["phase"], "idle");
    online.close();
}

/// A small deterministic generator (the seeded runs of the property test).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

#[tokio::test]
async fn retains_protection_through_generated_credential_and_network_failures_until_authoritative_recovery()
 {
    let mut generator = Lcg(7);
    for _run in 0..25 {
        let length = 1 + generator.next(10) as usize;
        let actions: Vec<&str> = (0..length)
            .map(|_| ["cancel", "credentials", "network"][generator.next(3) as usize])
            .collect();
        let network_down = Arc::new(AtomicBool::new(false));
        let down = Arc::clone(&network_down);
        let fixture = serve(move |seen| match seen.path() {
            "/api/account/playing" if down.load(Ordering::SeqCst) => {
                Reply::Json(503, json!({ "error": "offline" }).to_string())
            }
            "/api/account/playing" => Reply::Json(
                200,
                json!({ "nowPlaying": [{ "gameId": "AbCd1234" }] }).to_string(),
            ),
            _ => refuse(seen),
        })
        .await;
        let (host, store, tokens) = rig(&[("Alice", true)]);
        let online = session(&fixture.base, &host, &store, &tokens);
        online.resume().await.unwrap();
        for action in &actions {
            online.cancel();
            if *action != "cancel" {
                if *action == "credentials" {
                    tokens.set("Alice", Script::Now(None));
                } else {
                    tokens.set("Alice", Script::Now(Some("Alice-token".into())));
                    network_down.store(true, Ordering::SeqCst);
                }
                assert!(online.resume().await.is_err());
            }
            assert!(online.playing(), "protected after {actions:?}");
            assert!(online.assistance_blocked(), "blocked after {actions:?}");
        }
        tokens.set("Alice", Script::Now(Some("Alice-token".into())));
        network_down.store(false, Ordering::SeqCst);
        // Authoritative recovery: the server now reports no game.
        let empty = serve(|_| Reply::Json(200, json!({ "nowPlaying": [] }).to_string())).await;
        let online = session(&empty.base, &host, &store, &tokens);
        assert_eq!(online.resume().await.unwrap(), None);
        assert!(!online.playing());
        assert!(!online.assistance_blocked());
        online.close();
        drop(online);
    }
}

#[tokio::test]
async fn rejects_an_attachment_whose_credentials_arrive_after_cancel_close_or_logout() {
    for action in ["cancel", "close", "logout"] {
        let fixture = serve(|_| Reply::Hang).await;
        let (host, store, tokens) = rig(&[("Alice", true)]);
        let (token_tx, token) = watch::channel(false);
        tokens.set("Alice", Script::Gate(token, Some("Alice-token".into())));
        let online = session(&fixture.base, &host, &store, &tokens);
        let attaching = online.clone();
        let pending = tokio::spawn(async move { attaching.attach("Alice", "AbCd1234").await });
        until("credentials requested", || tokens.calls_for("Alice") >= 1).await;
        match action {
            "cancel" => online.cancel(),
            "close" => online.close(),
            _ => online.logout(Some(&["Alice".to_string()])),
        }
        let _ = token_tx.send(true);
        let outcome = pending.await.unwrap();
        assert!(outcome.is_err_and(|cause| cause.is_aborted()), "{action}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            fixture.to("/api/board/game/stream/AbCd1234").is_empty(),
            "{action}"
        );
        assert!(!online.playing(), "{action}");
        online.close();
    }
}

#[tokio::test]
async fn keeps_the_newer_attachment_when_an_older_account_login_arrives_late() {
    let fixture = serve(|_| Reply::Hang).await;
    let (host, store, tokens) = rig(&[("Alice", true), ("Bob", true)]);
    let (alice_tx, alice) = watch::channel(false);
    tokens.set("Alice", Script::Gate(alice, Some("Alice-token".into())));
    let online = session(&fixture.base, &host, &store, &tokens);
    let older = online.clone();
    let pending = tokio::spawn(async move { older.attach("Alice", "AbCd1234").await });
    until("alice requested", || tokens.calls_for("Alice") >= 1).await;
    online.attach("Bob", "Other123").await.unwrap();
    wait_requests(&fixture, "/api/board/game/stream/Other123", 1).await;
    let games = fixture.to("/api/board/game/stream/Other123");
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].header("Authorization"), Some("Bearer Bob-token"));
    let _ = alice_tx.send(true);
    assert!(
        pending
            .await
            .unwrap()
            .is_err_and(|cause| cause.is_aborted())
    );
    assert_eq!(
        fixture.to("/api/board/game/stream/AbCd1234").len(),
        0,
        "the superseded attachment opened nothing"
    );
    assert!(online.playing());
    online.close();
}

#[tokio::test]
async fn supersedes_a_pending_attachment_with_authoritative_recovery() {
    let fixture = serve(|seen| match seen.path() {
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let (token_tx, token) = watch::channel(false);
    tokens.queue(
        "Alice",
        vec![
            Script::Gate(token, Some("Alice-token".into())),
            Script::Now(Some("Alice-token".into())),
        ],
    );
    let online = session(&fixture.base, &host, &store, &tokens);
    let attaching = online.clone();
    let pending = tokio::spawn(async move { attaching.attach("Alice", "AbCd1234").await });
    until("credentials requested", || tokens.calls_for("Alice") >= 1).await;
    assert_eq!(online.resume().await.unwrap(), None);
    let _ = token_tx.send(true);
    assert!(
        pending
            .await
            .unwrap()
            .is_err_and(|cause| cause.is_aborted())
    );
    assert_eq!(fixture.to("/api/account/playing").len(), 1);
    assert!(!online.playing());
    online.close();
}

#[tokio::test]
async fn treats_a_late_credential_failure_for_a_cancelled_attachment_as_cancellation() {
    let fixture = serve(|_| Reply::Hang).await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let (token_tx, token) = watch::channel(false);
    tokens.set("Alice", Script::Gate(token, None));
    let online = session(&fixture.base, &host, &store, &tokens);
    let attaching = online.clone();
    let pending = tokio::spawn(async move { attaching.attach("Alice", "AbCd1234").await });
    until("credentials requested", || tokens.calls_for("Alice") >= 1).await;
    online.cancel();
    // A failing store answers only after the cancellation: it is not an error to report.
    tokens.set("Alice", Script::Fail("Old login failure".into()));
    let _ = token_tx.send(true);
    assert!(
        pending
            .await
            .unwrap()
            .is_err_and(|cause| cause.is_aborted())
    );
    assert!(fixture.to("/api/board/game/stream/AbCd1234").is_empty());
    online.close();
}

/* ── online-actions.test.ts ───────────────────────────────────────────── */

/// A session attached to game AbCd1234 of Alice (a correspondence game of the same account is
/// listed first and must not be picked).
async fn playing(
    extra: impl Fn(&Seen) -> Option<Reply> + Send + 'static,
) -> (Arc<Recording>, fixtures::Fixture, OnlineSession) {
    let fixture = serve(move |seen| {
        if let Some(reply) = extra(seen) {
            return reply;
        }
        match seen.path() {
            "/api/account/playing" => Reply::Json(
                200,
                json!({
                    "nowPlaying": [
                        { "gameId": "Corr1234", "speed": "correspondence" },
                        { "gameId": "AbCd1234", "speed": "rapid" }
                    ]
                })
                .to_string(),
            ),
            _ => Reply::Hang,
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let store = MemoryStore::with_accounts(&[("Alice", true)]);
    let tokens = Arc::new(ScriptedTokens::default());
    let online = session(&fixture.base, &host, &store, &tokens);
    assert_eq!(
        online.resume().await.unwrap(),
        Some(Resumed {
            id: "AbCd1234".into(),
            account: "Alice".into()
        })
    );
    assert!(online.playing());
    (host, fixture, online)
}

#[tokio::test]
async fn reattaches_to_the_live_game_not_a_correspondence_one() {
    let (_host, fixture, online) = playing(|_| None).await;
    assert!(online.playing());
    wait_requests(&fixture, "/api/board/game/stream/AbCd1234", 1).await;
    assert!(fixture.to("/api/board/game/stream/Corr1234").is_empty());
    online.cancel();
    online.close();
}

#[tokio::test]
async fn game_actions_call_their_lichess_endpoints() {
    let cases: [(OnlineAction, &str, Option<&str>); 6] = [
        (
            OnlineAction::OfferDraw,
            "/api/board/game/AbCd1234/draw/yes",
            None,
        ),
        (
            OnlineAction::AcceptDraw,
            "/api/board/game/AbCd1234/draw/yes",
            None,
        ),
        (
            OnlineAction::DeclineDraw,
            "/api/board/game/AbCd1234/draw/false",
            None,
        ),
        (
            OnlineAction::ClaimVictory,
            "/api/board/game/AbCd1234/claim-victory",
            None,
        ),
        (
            OnlineAction::ClaimDraw,
            "/api/board/game/AbCd1234/claim-draw",
            None,
        ),
        (
            OnlineAction::Berserk,
            "/api/board/game/AbCd1234/berserk",
            None,
        ),
    ];
    for (action, path, _) in cases {
        let (_host, fixture, online) = playing(move |seen| {
            (seen.method == "POST").then(|| Reply::Json(200, json!({ "ok": true }).to_string()))
        })
        .await;
        online.action("AbCd1234", action).await.unwrap();
        let posts = fixture.to(path);
        assert_eq!(posts.len(), 1, "{action:?} calls {path}");
        assert_eq!(posts[0].header("Authorization"), Some("Bearer Alice-token"));
        online.cancel();
        online.close();
    }
}

#[tokio::test]
async fn refuses_actions_and_chat_for_a_game_not_on_the_board() {
    let (_host, _fixture, online) = playing(|seen| {
        (seen.method == "POST").then(|| Reply::Json(200, json!({ "ok": true }).to_string()))
    })
    .await;
    let action = online.action("Other123", OnlineAction::OfferDraw).await;
    assert!(action.unwrap_err().message().contains("Reconnect"));
    let chat = online.send_chat("Other123", "player", "hi").await;
    assert!(chat.unwrap_err().message().contains("Reconnect"));
    online.cancel();
    online.close();
}

#[tokio::test]
async fn sends_and_reads_the_player_chat_of_the_live_game() {
    let (_host, fixture, online) = playing(|seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/board/game/AbCd1234/chat") => {
            Some(Reply::Json(200, json!({ "ok": true }).to_string()))
        }
        ("GET", "/api/board/game/AbCd1234/chat") => Some(Reply::Json(
            200,
            json!([{ "user": "Bob", "text": "You too" }, { "bogus": 1 }]).to_string(),
        )),
        _ => None,
    })
    .await;
    online
        .send_chat("AbCd1234", "player", "Good luck")
        .await
        .unwrap();
    let sent = fixture.to("/api/board/game/AbCd1234/chat");
    let post = sent.iter().find(|seen| seen.method == "POST").unwrap();
    assert_eq!(field(post, "room").as_deref(), Some("player"));
    assert_eq!(field(post, "text").as_deref(), Some("Good luck"));
    let lines = online.chat("AbCd1234").await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].user, "Bob");
    assert_eq!(lines[0].text, "You too");
    assert_eq!(lines[0].room, "player");
    online.cancel();
    online.close();
}

#[tokio::test]
async fn keeps_the_idle_event_stream_open_for_challenges_and_closes_it_on_request() {
    let streams: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&streams);
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    wait_requests(&fixture, "/api/stream/event", 1).await;
    let call = fixture.to("/api/stream/event").remove(0);
    assert_eq!(call.header("Authorization"), Some("Bearer Alice-token"));
    let sender = streams.lock().unwrap().clone().unwrap();
    online.stay_connected("");
    // Writes to the stream find it gone once the client has closed its end.
    for _ in 0..200 {
        if sender.is_closed() {
            break;
        }
        let _ = sender.send("\n".into());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sender.is_closed(), "the stream was closed by the client");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.to("/api/stream/event").len(), 1, "no reconnect");
    online.close();
}

#[tokio::test]
async fn hands_challenge_events_on_the_idle_stream_to_the_inbox_and_opens_accepted_games() {
    let streams: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&streams);
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    wait_requests(&fixture, "/api/stream/event", 1).await;
    let sender = streams.lock().unwrap().clone().unwrap();
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
    until("challenge received", || {
        !emitted(&host, "challenge:received").is_empty()
    })
    .await;
    assert_eq!(emitted(&host, "challenge:received")[0]["id"], "Chal1234");
    sender
        .send(format!(
            "{}\n",
            json!({ "type": "gameStart", "game": { "gameId": "Chal1234", "speed": "rapid" } })
        ))
        .unwrap();
    wait_requests(&fixture, "/api/board/game/stream/Chal1234", 1).await;
    assert!(online.playing());
    online.close();
}

#[tokio::test]
async fn accepts_a_challenge_as_its_account_and_opens_the_game_at_once() {
    let fixture = serve(|seen| match seen.path() {
        "/api/challenge/Chal1234/accept" => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.accept_challenge(&incoming(json!({}))).await.unwrap();
    assert_eq!(fixture.to("/api/challenge/Chal1234/accept").len(), 1);
    assert!(online.playing());
    online.cancel();
    online.close();
}

#[tokio::test]
async fn accepting_a_correspondence_challenge_leaves_the_board_alone() {
    let fixture = serve(|seen| match seen.path() {
        "/api/challenge/Chal1234/accept" => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online
        .accept_challenge(&incoming(json!({ "correspondence": true })))
        .await
        .unwrap();
    assert!(!online.playing());
    until("ongoing changed", || {
        !emitted(&host, "online:ongoing-changed").is_empty()
    })
    .await;
    online.close();
}

#[tokio::test]
async fn declines_with_a_reason_and_refuses_a_second_live_game() {
    let (_host, fixture, online) = playing(|seen| {
        (seen.method == "POST").then(|| Reply::Json(200, json!({ "ok": true }).to_string()))
    })
    .await;
    let refused = online.accept_challenge(&incoming(json!({}))).await;
    assert!(refused.unwrap_err().message().contains("Finish the game"));
    online
        .decline_challenge(&incoming(json!({})), "later")
        .await
        .unwrap();
    let declines = fixture.to("/api/challenge/Chal1234/decline");
    assert_eq!(declines.len(), 1);
    assert_eq!(field(&declines[0], "reason").as_deref(), Some("later"));
    online.cancel();
    online.close();
}

#[tokio::test]
async fn creates_correspondence_challenges_without_waiting_on_a_stream() {
    let fixture = serve(|seen| match seen.path() {
        "/api/challenge/Bob" => Reply::Json(
            200,
            json!({ "id": "Corr5678", "url": "https://lichess.org/Corr5678" }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let mut options = seek_options(Some("Bob"));
    options.minutes = 0.0;
    options.days = Some(3.0);
    options.variant = Some("chess960".into());
    let result = online.start(&options).await.unwrap();
    assert_eq!(result.id.as_deref(), Some("Corr5678"));
    assert_eq!(result.correspondence, Some(true));
    let posted = fixture.to("/api/challenge/Bob");
    assert_eq!(field(&posted[0], "days").as_deref(), Some("3"));
    assert_eq!(field(&posted[0], "variant").as_deref(), Some("chess960"));
    assert!(fixture.to("/api/stream/event").is_empty());
}

#[tokio::test]
async fn refuses_a_rated_or_public_game_from_a_set_up_position() {
    let fixture = serve(|_| Reply::Hang).await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let fen = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
    let rated = OnlineOptions {
        color: "white".into(),
        rated: true,
        fen: Some(fen.into()),
        ..seek_options(Some("Bob"))
    };
    let casual_public = OnlineOptions {
        color: "white".into(),
        fen: Some(fen.into()),
        ..seek_options(None)
    };
    assert!(
        online
            .start(&rated)
            .await
            .unwrap_err()
            .message()
            .contains("casual challenge")
    );
    assert!(
        online
            .start(&casual_public)
            .await
            .unwrap_err()
            .message()
            .contains("casual challenge")
    );
    online.close();
}

/* ── Scenarios the brief names beyond the TypeScript suites ─────────── */

#[tokio::test]
async fn a_move_is_sent_once_and_a_failure_is_not_retried() {
    let (_host, fixture, online) = playing(|seen| {
        (seen.method == "POST" && seen.path().ends_with("/move/e2e4"))
            .then(|| Reply::Json(503, json!({ "error": "unavailable" }).to_string()))
    })
    .await;
    let outcome = online.move_uci("AbCd1234", "e2e4").await;
    assert!(outcome.is_err());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        fixture.to("/api/board/game/AbCd1234/move/e2e4").len(),
        1,
        "one attempt"
    );
    assert!(
        online.playing(),
        "a failed move keeps the game on the board"
    );
    online.cancel();
    online.close();
}

#[tokio::test]
async fn a_live_game_is_not_replaced_by_opening_another_game() {
    let (_host, _fixture, online) = playing(|_| None).await;
    let other = online.open("Alice", "Other123").await;
    assert!(
        other
            .unwrap_err()
            .message()
            .contains("Finish the live game")
    );
    online.cancel();
    online.close();
}

#[tokio::test]
async fn a_dropped_game_stream_reconnects_and_keeps_the_game() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let fixture = serve(move |seen| match seen.path() {
        "/api/board/game/stream/AbCd1234" => {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                // The first stream ends straight away; the reconnect is the live one.
                Reply::Text(200, "application/x-ndjson", String::new())
            } else {
                Reply::Hang
            }
        }
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234", "speed": "rapid" }] }).to_string(),
        ),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.resume().await.unwrap();
    until("reconnected", || attempts.load(Ordering::SeqCst) >= 2).await;
    assert!(online.playing());
    assert!(has_state(&host, "reconnecting", "AbCd1234"));
    online.cancel();
    online.close();
}

#[tokio::test]
async fn a_tournament_join_and_withdrawal_use_the_accounts_login() {
    let fixture = serve(|seen| match seen.path() {
        "/api/tournament/Abcd1234/join" => Reply::Json(200, json!({ "ok": true }).to_string()),
        "/api/tournament/Abcd1234/withdraw" => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let lichess = {
        let client = LichessClient::with_base(&fixture.base, policy(&host), http_client());
        Arc::new(Lichess::new(
            client,
            capabilities_for(&host),
            Arc::clone(&store) as Arc<dyn kchess_core::lichess::accounts::LichessStore>,
            Arc::clone(&tokens) as Arc<dyn TokenSource>,
            Arc::clone(&host) as Arc<dyn Host>,
            CancellationToken::new(),
        ))
    };
    let joined = kchess_core::lichess::tournaments::join_tournament(
        &lichess,
        "arena",
        "Abcd1234",
        "Alice",
        Some("secret"),
    )
    .await
    .unwrap();
    assert!(matches!(
        joined,
        kchess_core::lichess::accounts::Reply::Done(())
    ));
    let posted = fixture.to("/api/tournament/Abcd1234/join");
    assert_eq!(field(&posted[0], "password").as_deref(), Some("secret"));
    assert_eq!(field(&posted[0], "pairMeAsap").as_deref(), Some("true"));
    assert_eq!(
        posted[0].header("Authorization"),
        Some("Bearer Alice-token")
    );
    let left =
        kchess_core::lichess::tournaments::leave_tournament(&lichess, "arena", "Abcd1234", "Alice")
            .await
            .unwrap();
    assert!(matches!(
        left,
        kchess_core::lichess::accounts::Reply::Done(())
    ));
    assert_eq!(fixture.to("/api/tournament/Abcd1234/withdraw").len(), 1);
}

/// A malformed record on the idle event stream is reported as it arrives: the stream counts as
/// interrupted and the lobby says so while the stream is still open, as the TypeScript event stream
/// did when its line callback threw (`core/src/services/lichess.ts`).
#[tokio::test]
async fn a_malformed_line_on_the_event_stream_is_reported_as_it_arrives() {
    let (lines_tx, lines_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut lines = Some(lines_rx);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("GET", "/api/stream/event") => match lines.take() {
            Some(open) => Reply::Stream(open),
            None => refuse(seen),
        },
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    wait_requests(&fixture, "/api/stream/event", 1).await;
    lines_tx
        .send("{\"type\": \"challenge\", \"challenge\": \n".to_string())
        .expect("the stream is open");
    until("the malformed record interrupts the stream", || {
        emitted(&host, "online:lobby").iter().any(|state| {
            state["phase"] == "reconnecting"
                && state["message"] == "Connection interrupted. Reconnecting…"
        })
    })
    .await;
    drop(lines_tx);
    online.close();
}
