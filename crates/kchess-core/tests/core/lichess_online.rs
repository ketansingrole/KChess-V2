//! The online-play session over local fixtures: recovery, protection, attachments and
//! cancellation; actions, chat, lobby, challenges and seeks; and that a move is sent once,
//! a dropped game stream reconnects, and a tournament join and withdrawal use the account's login.

use crate::fixtures;
use crate::support::until;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{MemoryStore, Recording, Reply, Request, capabilities_for, http_client, serve};
use futures_util::future::BoxFuture;
use kchess_contracts::Variant;
use kchess_contracts::error::{CoreError, Result as CoreResult};
use kchess_contracts::types::{ChallengeColor, ChatRoom, TournamentSystem};
use kchess_core::host::Host;
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::challenges::{ChallengeInfo, PlayerRef, TimeControl};
use kchess_core::lichess::client::{LichessClient, TokenSource};
use kchess_core::lichess::online::{OnlineAction, OnlineOptions, OnlineSession, Resumed};
use kchess_core::lichess::policy::Policy;
use kchess_core::lichess::stream::Backoff;
use serde_json::{Number, Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// One scripted answer of the token source.
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
        Arc::clone(host) as Arc<dyn kchess_core::usage::Meter>,
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
    session_with(base, host, store, tokens, Backoff::default())
}

/// `session`, with dropped streams reconnecting after `backoff`.
fn session_with(
    base: &str,
    host: &Arc<Recording>,
    store: &Arc<MemoryStore>,
    tokens: &Arc<ScriptedTokens>,
    backoff: Backoff,
) -> OnlineSession {
    let client = LichessClient::with_base(base, policy(host));
    let lichess = Arc::new(Lichess::new(
        client,
        capabilities_for(host),
        Arc::clone(store) as Arc<dyn kchess_core::lichess::accounts::LichessStore>,
        Arc::clone(tokens) as Arc<dyn TokenSource>,
        Arc::clone(host) as Arc<dyn Host>,
    ));
    OnlineSession::with_backoff(lichess, backoff)
}

fn rig(accounts: &[(&str, bool)]) -> (Arc<Recording>, Arc<MemoryStore>, Arc<ScriptedTokens>) {
    (
        Arc::new(Recording::default()),
        MemoryStore::with_accounts(accounts),
        Arc::new(ScriptedTokens::default()),
    )
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

/// The default answer of the account and game routes: a game of Alice's, and every stream
/// refused (`401`).
fn refuse(seen: &Request) -> Reply {
    Reply::Json(
        401,
        json!({ "error": format!("refused {}", seen.path()) }).to_string(),
    )
}

fn incoming(overrides: Value) -> ChallengeInfo {
    let mut info = ChallengeInfo {
        id: "Chal1234".into(),
        account: "Alice".into(),
        direction: kchess_contracts::types::ChallengeDirection::In,
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
        minutes: 10,
        increment: 0,
        days: None,
        variant: None,
        fen: None,
        color: ChallengeColor::Random,
        rated: false,
        target: target.map(str::to_string),
        account: None,
    }
}

/// The form fields of a request body, decoded.
fn form(seen: &Request) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://fixture.invalid/?{}", seen.body))
        .expect("form body")
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn field(seen: &Request, name: &str) -> Option<String> {
    form(seen)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/* ── session ───────────────────────────────────────────────── */

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
    fixture.requested("/api/challenge/Bob", 1).await;
    online.cancel();
    // Lichess answers after the cancel: the late challenge is still cancelled, once.
    let _ = gate_tx.send(true);
    let outcome = result.await.unwrap();
    assert!(outcome.as_ref().is_err_and(|cause| cause.aborted));
    fixture.requested("/api/challenge/AbCd1234/cancel", 1).await;
    assert_eq!(fixture.to("/api/challenge/Bob").len(), 1);
    assert_eq!(fixture.to("/api/challenge/AbCd1234/cancel").len(), 1);
    online.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.to("/api/challenge/Bob").len(), 1);
    assert_eq!(fixture.to("/api/challenge/AbCd1234/cancel").len(), 1);
    online.close();
}

/// A challenge whose creation fails after the player cancelled it ends as cancelled, with nothing
/// to show.
#[tokio::test]
async fn a_challenge_that_fails_after_the_cancel_ends_as_cancelled() {
    let (gate_tx, gate) = watch::channel(false);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/challenge/Bob") => Reply::Gated(
            gate.clone(),
            Box::new(Reply::Json(400, json!({ "error": "No" }).to_string())),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let starting = online.clone();
    let result = tokio::spawn(async move { starting.start(&seek_options(Some("Bob"))).await });
    fixture.requested("/api/challenge/Bob", 1).await;
    online.cancel();
    let _ = gate_tx.send(true);
    let outcome = result.await.unwrap();
    assert!(
        outcome.as_ref().is_err_and(|cause| cause.aborted),
        "{outcome:?}"
    );
    online.close();
}

/// What a game may be is checked before anything is asked of Lichess: a position only for a
/// casual challenge, Blitz and up for a challenge, Rapid and up for a seek, any clock by
/// correspondence. What passes is asked for as a challenge, a seek or a correspondence game.
#[tokio::test]
async fn a_game_is_checked_before_it_is_asked_for() {
    let fixture = serve(|seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/challenge/Bob") => Reply::Json(
            200,
            json!({ "id": "Chal1234", "url": "https://lichess.org/Chal1234" }).to_string(),
        ),
        ("POST", "/api/board/seek") if field(seen, "days").is_some() => Reply::Json(
            200,
            json!({ "id": "Corr1234", "url": "https://lichess.org/Corr1234" }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    const FEN: &str = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
    let clock = |minutes: u32, target: Option<&str>| OnlineOptions {
        minutes,
        ..seek_options(target)
    };
    let by_post = |options: OnlineOptions| OnlineOptions {
        days: Some(3),
        ..options
    };
    // What was asked for (the challenge's id, a seek, a correspondence game), or the refusal.
    let cases: Vec<(OnlineOptions, Result<&str, &str>)> = vec![
        (
            OnlineOptions {
                fen: Some(String::new()),
                ..clock(10, None)
            },
            Ok("seek"),
        ),
        (
            OnlineOptions {
                fen: Some(FEN.into()),
                ..clock(10, Some("Bob"))
            },
            Ok("Chal1234"),
        ),
        (clock(1, Some("Bob")), Err("even for direct challenges")),
        (clock(3, Some("Bob")), Ok("Chal1234")),
        (by_post(clock(1, Some("Bob"))), Ok("correspondence")),
        (clock(1, None), Err("for public seeks")),
        (clock(3, None), Err("for public seeks")),
        (by_post(clock(1, None)), Ok("correspondence")),
        (clock(10, None), Ok("seek")),
    ];
    for (options, expected) in cases {
        let (host, store, tokens) = rig(&[("Alice", true)]);
        let online = session(&fixture.base, &host, &store, &tokens);
        let asked = fixture.to("/api/challenge/Bob").len() + fixture.to("/api/board/seek").len();
        let started = online.start(&options).await;
        let answer = match &started {
            Ok(result) if result.correspondence == Some(true) => Ok("correspondence"),
            Ok(result) if result.seeking == Some(true) => Ok("seek"),
            Ok(result) => Ok(result.id.as_deref().unwrap_or("")),
            Err(cause) => Err(cause.message.as_str()),
        };
        match expected {
            Ok(wanted) => assert_eq!(answer, Ok(wanted), "{options:?}"),
            Err(part) => {
                assert!(
                    answer.is_err_and(|message| message.contains(part)),
                    "{options:?}: {answer:?}"
                );
                let now =
                    fixture.to("/api/challenge/Bob").len() + fixture.to("/api/board/seek").len();
                assert_eq!(now, asked, "a refused game asks nothing: {options:?}");
            }
        }
        online.cancel();
        online.close();
    }
}

/// A refused challenge explains the Board API's limits for challenges with Lichess's reason.
#[tokio::test]
async fn a_refused_challenge_explains_the_board_api_limits() {
    let fixture = serve(|seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/challenge/Bob") => Reply::Json(
            400,
            json!({ "error": "Bob takes no challenges" }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let options = OnlineOptions {
        minutes: 3,
        ..seek_options(Some("Bob"))
    };
    let error = online.start(&options).await.unwrap_err();
    assert!(!error.aborted);
    assert!(
        error
            .message
            .starts_with("Lichess rejected this Blitz (3+0) challenge: Bob takes no challenges."),
        "{}",
        error.message
    );
    online.close();
}

/// A correspondence game asked for leaves nothing waiting: the account's idle stream opens again
/// to hear when it is accepted.
#[tokio::test]
async fn a_correspondence_game_asked_for_reopens_the_idle_stream() {
    let fixture = serve(|seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/board/seek") => Reply::Json(
            200,
            json!({ "id": "Corr1234", "url": "https://lichess.org/Corr1234" }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    fixture.requested("/api/stream/event", 1).await;
    let options = OnlineOptions {
        days: Some(3),
        ..seek_options(None)
    };
    let started = online.start(&options).await.unwrap();
    assert_eq!(started.correspondence, Some(true));
    fixture.requested("/api/stream/event", 2).await;
    online.close();
}

/// Leaving a correspondence game on the board lets go of its protection: unlike a real-time
/// game, nothing is lost by stepping away from it, so it keeps nothing else off the board.
#[tokio::test]
async fn leaving_a_correspondence_game_lets_go_of_its_protection() {
    let game: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&game);
    let fixture = serve(move |seen| match seen.path() {
        "/api/board/game/stream/Corr1234" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online
        .open("Alice", "Corr1234")
        .await
        .expect("the game opens");
    fixture
        .requested("/api/board/game/stream/Corr1234", 1)
        .await;
    let sender = game.lock().unwrap().clone().unwrap();
    let full = json!({
        "type": "gameFull",
        "id": "Corr1234",
        "white": {},
        "black": {},
        "speed": "correspondence",
        "state": { "type": "gameState", "moves": "", "status": "started" },
    });
    sender.send(format!("{full}\n")).unwrap();
    until("the game to reach the window", || {
        emitted(&host, "online:event")
            .iter()
            .any(|event| event["type"] == "gameFull")
    })
    .await;
    assert!(online.playing());
    // A real-time game may be looked for while it is on the board.
    let started = online
        .start(&seek_options(None))
        .await
        .expect("a seek beside a correspondence game");
    assert_eq!(started.seeking, Some(true));
    online.cancel();
    assert!(!online.playing());
    online.close();
}

/// Each cancel moves the session on: a state reported after it says so, and a stream from before
/// it is rejected as stale.
#[tokio::test]
async fn each_cancel_moves_the_session_on() {
    let fixture = serve(|_| Reply::Hang).await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    // Recovery is unverified at first, so each cancel reports the session it leaves behind.
    online.cancel();
    online.cancel();
    let sessions: Vec<Value> = states(&host)
        .iter()
        .map(|state| state["session"].clone())
        .collect();
    assert!(sessions.len() >= 2, "{sessions:?}");
    assert_ne!(sessions[sessions.len() - 1], sessions[sessions.len() - 2]);
    online.close();
}

/// A challenge Lichess answered is the session's: cancelling the session withdraws it, and
/// withdrawing another challenge leaves it the session's.
#[tokio::test]
async fn an_answered_challenge_is_withdrawn_with_the_session_but_not_with_another() {
    let fixture = serve(|seen| match (seen.method.as_str(), seen.path()) {
        ("POST", "/api/challenge/Bob") => Reply::Json(
            200,
            json!({ "id": "AbCd1234", "url": "https://lichess.org/AbCd1234" }).to_string(),
        ),
        ("POST", _) => Reply::Json(200, json!({ "ok": true }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online
        .start(&seek_options(Some("Bob")))
        .await
        .expect("the challenge is sent");
    online
        .withdraw_challenge(&incoming(json!({ "id": "Other999" })))
        .await
        .expect("another challenge is withdrawn");
    online.cancel();
    fixture.requested("/api/challenge/AbCd1234/cancel", 1).await;
    online.close();
}

/// A real-time game on the board, or one left protected by a dropped connection, keeps any other
/// game from starting or opening until it is verified over; the protected game itself opens.
#[tokio::test]
async fn a_protected_game_keeps_other_games_out_but_opens_itself() {
    const LIMIT: Duration = Duration::from_secs(5);
    const REFUSED: &str = "Reconnect to your current game";
    let (_host, _fixture, online) = playing(|_| None).await;
    let started = tokio::time::timeout(LIMIT, online.start(&seek_options(None)))
        .await
        .expect("refused at once");
    assert!(started.is_err_and(|cause| cause.message.contains(REFUSED)));
    // The connection drops: the board is empty, the game stays protected.
    online.cancel();
    assert!(online.playing());
    let started = tokio::time::timeout(LIMIT, online.start(&seek_options(None)))
        .await
        .expect("refused at once");
    assert!(started.is_err_and(|cause| cause.message.contains(REFUSED)));
    let other = tokio::time::timeout(LIMIT, online.open("Alice", "Other123"))
        .await
        .expect("refused at once");
    assert!(other.is_err_and(|cause| cause.message.contains(REFUSED)));
    // The protected game itself is not refused: it attaches (here, to a stream that never answers).
    if let Ok(itself) =
        tokio::time::timeout(Duration::from_millis(300), online.open("Alice", "AbCd1234")).await
    {
        assert!(itself.is_ok() || itself.is_err_and(|cause| !cause.message.contains(REFUSED)));
    }
    online.close();
}

/// A refused public seek explains the Board API's matchmaking limits. It was matched by its
/// endpoint's text, so the explanation depended on how the error happened to name the request.
#[tokio::test]
async fn a_refused_public_seek_explains_the_board_api_limits() {
    let fixture = serve(|seen| match seen.path() {
        "/api/board/seek" => Reply::Json(400, json!({ "error": "Bad request" }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let started = online.start(&seek_options(None)).await.unwrap();
    assert_eq!(started.seeking, Some(true));
    crate::support::wire::assert_wire("startOnline.seeking", &started);
    until("the refusal is explained", || {
        emitted(&host, "online:error").iter().any(|message| {
            message
                .as_str()
                .is_some_and(|text| text.starts_with("Lichess rejected this public seek."))
        })
    })
    .await;
    online.close();
}

/// A seek that fails for another reason than a refusal shows that reason, not the matchmaking
/// limits.
#[tokio::test]
async fn a_seek_that_fails_otherwise_shows_its_own_reason() {
    let fixture = serve(|seen| match seen.path() {
        "/api/board/seek" => Reply::Json(404, json!({ "error": "No seeks today" }).to_string()),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.start(&seek_options(None)).await.unwrap();
    until("the failure is shown", || {
        !emitted(&host, "online:error").is_empty()
    })
    .await;
    assert_eq!(
        emitted(&host, "online:error"),
        [json!("No seeks today.")],
        "shown once, as Lichess said it"
    );
    online.close();
}

/// A game stream that keeps failing gives up after its reconnects: the game's page offers to
/// reconnect, and the player is told the connection is lost.
#[tokio::test]
async fn a_game_stream_that_keeps_failing_is_reported_lost() {
    let fixture = serve(|seen| match seen.path() {
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234", "speed": "rapid" }] }).to_string(),
        ),
        "/api/board/game/stream/AbCd1234" => {
            Reply::Json(500, json!({ "error": "Down" }).to_string())
        }
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let backoff = Backoff {
        first: Duration::from_millis(1),
        most: Duration::from_millis(1),
    };
    let online = session_with(&fixture.base, &host, &store, &tokens, backoff);
    online.resume().await.unwrap();
    until("the connection is reported lost", || {
        !emitted(&host, "online:error").is_empty()
    })
    .await;
    assert_eq!(
        emitted(&host, "online:error"),
        [json!(
            "Lost connection to Lichess. Check your network and reopen the game."
        )]
    );
    assert!(states(&host).iter().any(|state| {
        state["phase"] == "disconnected"
            && state["gameId"] == "AbCd1234"
            && state["message"] == "Lost connection to Lichess. Reconnect to recover the game."
    }));
    online.close();
}

/// Every 403 asked to reconnect, though one that is not about the login (Lichess refusing the
/// request itself) is not mended by reconnecting.
#[tokio::test]
async fn a_403_that_is_not_about_the_login_does_not_ask_to_reconnect() {
    let fixture = serve(|seen| match seen.path() {
        "/api/account/playing" => Reply::Json(403, json!({ "error": "Forbidden" }).to_string()),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    assert!(online.resume().await.is_err());
    assert_eq!(states(&host).last().unwrap()["phase"], "disconnected");
    online.close();

    // A refused login still does.
    let fixture = serve(|seen| match seen.path() {
        "/api/account/playing" => Reply::Json(401, json!({ "error": "No such token" }).to_string()),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    assert!(online.resume().await.is_err());
    assert_eq!(states(&host).last().unwrap()["phase"], "auth-required");
    online.close();
}

#[tokio::test]
async fn a_refused_start_leaves_the_pending_challenge_running() {
    // Validation ran after the running challenge was cancelled, so a bad request ended it.
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
    let (_host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &_host, &store, &tokens);
    let starting = online.clone();
    let result = tokio::spawn(async move { starting.start(&seek_options(Some("Bob"))).await });
    fixture.requested("/api/challenge/Bob", 1).await;
    let mut bullet = seek_options(Some("Carol"));
    bullet.minutes = 1;
    let refused = online.start(&bullet).await;
    assert!(refused.is_err_and(|cause| !cause.aborted));
    let _ = gate_tx.send(true);
    assert!(
        result.await.unwrap().is_ok(),
        "the first challenge was not aborted"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(fixture.to("/api/challenge/AbCd1234/cancel").is_empty());
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
    assert!(outcome.is_err_and(|cause| cause.aborted));
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
    assert!(error.message.contains("login is unavailable"));
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
    assert!(outcome.is_err_and(|cause| cause.aborted));
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
    assert!(error.message.contains("invalid ongoing-game response"));
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
        assert!(outcome.is_err_and(|cause| cause.aborted), "{action}");
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
    fixture
        .requested("/api/board/game/stream/Other123", 1)
        .await;
    let games = fixture.to("/api/board/game/stream/Other123");
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].header("Authorization"), Some("Bearer Bob-token"));
    let _ = alice_tx.send(true);
    assert!(pending.await.unwrap().is_err_and(|cause| cause.aborted));
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
    assert!(pending.await.unwrap().is_err_and(|cause| cause.aborted));
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
    assert!(pending.await.unwrap().is_err_and(|cause| cause.aborted));
    assert!(fixture.to("/api/board/game/stream/AbCd1234").is_empty());
    online.close();
}

/* ── actions ───────────────────────────────────────────────── */

/// A session attached to game AbCd1234 of Alice (a correspondence game of the same account is
/// listed first and must not be picked).
async fn playing(
    extra: impl Fn(&Request) -> Option<Reply> + Send + 'static,
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
    let resumed = online.resume().await.unwrap();
    crate::support::wire::assert_wire("resumeOnline.live", &resumed);
    assert_eq!(
        resumed,
        Some(Resumed {
            id: "AbCd1234".into(),
            account: "Alice".into()
        })
    );
    assert!(online.playing());
    (host, fixture, online)
}

/// One account's failed check does not end recovery: the next account's live game is found.
#[tokio::test]
async fn recovery_goes_on_past_an_account_whose_check_failed() {
    let fixture = serve(|seen| match seen.path() {
        "/api/account/playing" if seen.header("Authorization") == Some("Bearer Alice-token") => {
            Reply::Json(404, json!({ "error": "Gone" }).to_string())
        }
        "/api/account/playing" => Reply::Json(
            200,
            json!({ "nowPlaying": [{ "gameId": "AbCd1234", "speed": "rapid" }] }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true), ("Bob", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let resumed = online.resume().await.unwrap();
    assert_eq!(
        resumed,
        Some(Resumed {
            id: "AbCd1234".into(),
            account: "Bob".into()
        })
    );
    assert_eq!(fixture.to("/api/account/playing").len(), 2);
    assert!(host.logged().contains("Game recovery check failed: Alice"));
    online.cancel();
    online.close();
}

#[tokio::test]
async fn reattaches_to_the_live_game_not_a_correspondence_one() {
    let (_host, fixture, online) = playing(|_| None).await;
    assert!(online.playing());
    fixture
        .requested("/api/board/game/stream/AbCd1234", 1)
        .await;
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
    assert!(action.unwrap_err().message.contains("Reconnect"));
    let chat = online.send_chat("Other123", "player", "hi").await;
    assert!(chat.unwrap_err().message.contains("Reconnect"));
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
    crate::support::wire::assert_wire("onlineChat.lines", &lines);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].user, "Bob");
    assert_eq!(lines[0].text, "You too");
    assert_eq!(lines[0].room, ChatRoom::Player);
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
    fixture.requested("/api/stream/event", 1).await;
    let call = fixture.to("/api/stream/event").remove(0);
    assert_eq!(call.header("Authorization"), Some("Bearer Alice-token"));
    let sender = streams.lock().unwrap().clone().unwrap();
    online.stay_connected("");
    // Writes to the stream find it gone once the client has closed its end.
    until("the client to close the stream", || {
        let _ = sender.send("\n".into());
        sender.is_closed()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.to("/api/stream/event").len(), 1, "no reconnect");
    online.close();
}

#[tokio::test]
async fn reopens_the_idle_event_stream_after_it_ended_on_its_own() {
    // An ended stream stayed registered as running, so challenges stopped for the session.
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    tokens.queue(
        "Alice",
        vec![Script::Now(None), Script::Now(Some("Alice-token".into()))],
    );
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    until("auth-required", || {
        emitted(&host, "online:lobby")
            .iter()
            .any(|state| state["phase"] == "auth-required")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    online.stay_connected("Alice");
    fixture.requested("/api/stream/event", 1).await;
    assert_eq!(tokens.calls_for("Alice"), 2);
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
    fixture.requested("/api/stream/event", 1).await;
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
    assert_eq!(online.challenges().list().len(), 1);
    sender
        .send(format!(
            "{}\n",
            json!({ "type": "gameStart", "game": { "gameId": "Chal1234", "speed": "rapid" } })
        ))
        .unwrap();
    fixture
        .requested("/api/board/game/stream/Chal1234", 1)
        .await;
    assert!(online.playing());
    // Accepted elsewhere, the challenge became this game: Lichess sends nothing else for it.
    assert!(online.challenges().list().is_empty());
    online.close();
}

/// The start of the game a challenge asked for reaches the window once, however often the
/// event stream repeats it.
#[tokio::test]
async fn an_accepted_challenge_is_announced_once() {
    let streams: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&streams);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("GET", "/api/stream/event") => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Reply::Stream(rx)
        }
        ("POST", "/api/challenge/Bob") => Reply::Json(
            200,
            json!({ "id": "Chal1234", "url": "https://lichess.org/Chal1234" }).to_string(),
        ),
        _ => Reply::Hang,
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    let options = OnlineOptions {
        minutes: 3,
        ..seek_options(Some("Bob"))
    };
    online.start(&options).await.unwrap();
    fixture.requested("/api/stream/event", 1).await;
    let sender = streams.lock().unwrap().clone().unwrap();
    let start = json!({ "type": "gameStart", "game": { "gameId": "Chal1234", "speed": "blitz" } });
    let starts = |host: &Recording| {
        emitted(host, "online:event")
            .iter()
            .filter(|event| event["type"] == "gameStart")
            .count()
    };
    sender.send(format!("{start}\n")).unwrap();
    until("the start reaches the window", || starts(&host) == 1).await;
    fixture
        .requested("/api/board/game/stream/Chal1234", 1)
        .await;
    // Repeated, then a finish that arrives after it.
    sender.send(format!("{start}\n")).unwrap();
    let finish = json!({
        "type": "gameFinish",
        "game": { "gameId": "Chal1234", "status": { "name": "resign" }, "winner": "black" }
    });
    sender.send(format!("{finish}\n")).unwrap();
    until("the finish reaches the window", || {
        emitted(&host, "online:event")
            .iter()
            .any(|event| event["type"] == "gameFinish")
    })
    .await;
    assert_eq!(starts(&host), 1);
    online.cancel();
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
    assert!(refused.unwrap_err().message.contains("Finish the game"));
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
    options.minutes = 0;
    options.days = Some(3);
    options.variant = Some(Variant::Chess960);
    let result = online.start(&options).await.unwrap();
    crate::support::wire::assert_wire("startOnline.correspondence", &result);
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
        color: ChallengeColor::White,
        rated: true,
        fen: Some(fen.into()),
        ..seek_options(Some("Bob"))
    };
    let casual_public = OnlineOptions {
        color: ChallengeColor::White,
        fen: Some(fen.into()),
        ..seek_options(None)
    };
    assert!(
        online
            .start(&rated)
            .await
            .unwrap_err()
            .message
            .contains("casual challenge")
    );
    assert!(
        online
            .start(&casual_public)
            .await
            .unwrap_err()
            .message
            .contains("casual challenge")
    );
    online.close();
}

/* ── Recovery, ownership and duplicate scenarios ─────────── */

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

/// Two windows send a move for the same position: while the first is unanswered the second is
/// refused, never sent, so it cannot land on the position after the opponent replies.
#[tokio::test]
async fn a_second_move_is_refused_while_the_first_is_unanswered() {
    let (_host, fixture, online) = playing(|seen| match seen.method.as_str() {
        "POST" if seen.path().ends_with("/move/e2e4") => Some(Reply::Hang),
        "POST" if seen.path().contains("/move/") => {
            Some(Reply::Json(200, json!({ "ok": true }).to_string()))
        }
        _ => None,
    })
    .await;
    let first = tokio::spawn({
        let online = online.clone();
        async move { online.move_uci("AbCd1234", "e2e4").await }
    });
    fixture
        .requested("/api/board/game/AbCd1234/move/e2e4", 1)
        .await;
    let second = online.move_uci("AbCd1234", "d2d4").await;
    assert!(second.unwrap_err().message.contains("already being sent"));
    assert!(
        fixture.to("/api/board/game/AbCd1234/move/d2d4").is_empty(),
        "the second move never reaches Lichess"
    );
    first.abort();
    let _ = first.await;
    let third = online.move_uci("AbCd1234", "g1f3").await;
    assert!(
        third.is_ok(),
        "a dropped call releases the guard: {third:?}"
    );
    online.cancel();
    online.close();
}

#[tokio::test]
async fn a_live_game_is_not_replaced_by_opening_another_game() {
    let (_host, _fixture, online) = playing(|_| None).await;
    let other = online.open("Alice", "Other123").await;
    assert!(other.unwrap_err().message.contains("Finish the live game"));
    online.cancel();
    online.close();
}

/// A real-time game left by a cancelled session stays protected; opening another game must not
/// take its protection over, or that game finishing would allow assistance in the first.
#[tokio::test]
async fn a_protected_game_is_not_replaced_by_opening_another_game() {
    let (_host, _fixture, online) = playing(|_| None).await;
    online.cancel();
    assert!(online.playing(), "the left game stays protected");
    let other = online.open("Alice", "Corr1234").await;
    assert!(
        other
            .unwrap_err()
            .message
            .contains("Reconnect to your current game")
    );
    assert!(online.assistance_blocked());
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
        let client = LichessClient::with_base(&fixture.base, policy(&host));
        Arc::new(Lichess::new(
            client,
            capabilities_for(&host),
            Arc::clone(&store) as Arc<dyn kchess_core::lichess::accounts::LichessStore>,
            Arc::clone(&tokens) as Arc<dyn TokenSource>,
            Arc::clone(&host) as Arc<dyn Host>,
        ))
    };
    let joined = kchess_core::lichess::tournaments::join_tournament(
        &lichess,
        TournamentSystem::Arena,
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
    let left = kchess_core::lichess::tournaments::leave_tournament(
        &lichess,
        TournamentSystem::Arena,
        "Abcd1234",
        "Alice",
    )
    .await
    .unwrap();
    assert!(matches!(
        left,
        kchess_core::lichess::accounts::Reply::Done(())
    ));
    assert_eq!(fixture.to("/api/tournament/Abcd1234/withdraw").len(), 1);
}

/// A malformed record on the idle event stream is reported as it arrives: the stream counts as
/// interrupted and the lobby says so while the stream is still open, as the event stream
/// did when its line callback failed.
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
    fixture.requested("/api/stream/event", 1).await;
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

/// A record the checks refuse, replayed by Lichess on every connect, used to count as progress:
/// the stream reconnected about once a second forever instead of backing off and giving up.
#[tokio::test]
async fn a_refused_record_on_every_connect_backs_off_instead_of_reconnecting_each_second() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        ("GET", "/api/stream/event") => {
            counter.fetch_add(1, Ordering::SeqCst);
            Reply::Text(200, "application/x-ndjson", "not a record\n".to_string())
        }
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true)]);
    let backoff = Backoff {
        first: Duration::from_millis(50),
        most: Duration::from_secs(15),
    };
    let online = session_with(&fixture.base, &host, &store, &tokens, backoff);
    online.stay_connected("Alice");
    fixture.requested("/api/stream/event", 1).await;
    // Backing off: connects at 0, 100 and 300 ms, then 700 ms. Every 100 ms would be five by now.
    tokio::time::sleep(Duration::from_millis(450)).await;
    let seen = attempts.load(Ordering::SeqCst);
    assert!(seen <= 3, "reconnected {seen} times in 450 ms");
    online.close();
}

/// Challenges sent to a connected account whose event stream is not open reached no inbox: the
/// ongoing refresh lists them (`GET /api/challenge`) for every account but the one streaming, and a
/// later listing without one removes it.
#[tokio::test]
async fn lists_the_challenges_of_accounts_without_an_open_event_stream() {
    let pending = Arc::new(Mutex::new(true));
    let listing = Arc::clone(&pending);
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/api/challenge" => {
            let challenge = json!({
                "id": "BobChal1",
                "url": "https://lichess.org/BobChal1",
                "status": "created",
                "challenger": { "id": "carol", "name": "Carol", "rating": 1800 },
                "destUser": { "id": "bob", "name": "Bob", "rating": 1700 },
                "variant": { "key": "standard", "name": "Standard" },
                "rated": true,
                "speed": "bullet",
                "timeControl": { "type": "clock", "limit": 60, "increment": 0 },
                "color": "random",
                "perf": { "name": "Bullet" },
                "direction": "in",
            });
            let incoming = if *listing.lock().unwrap() {
                vec![challenge]
            } else {
                vec![]
            };
            Reply::Json(200, json!({ "in": incoming, "out": [] }).to_string())
        }
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true), ("Bob", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    fixture.requested("/api/stream/event", 1).await;

    online.ongoing().await.unwrap();
    let calls = fixture.to("/api/challenge");
    assert_eq!(
        calls.len(),
        1,
        "only the account without a stream is listed"
    );
    assert_eq!(calls[0].header("Authorization"), Some("Bearer Bob-token"));
    let inbox = online.challenges().list();
    // The time it arrived is the clock's: an integer, recorded as 0.
    let mut shown = serde_json::to_value(&inbox).unwrap();
    assert!(shown[0]["receivedAt"].is_i64());
    shown[0]["receivedAt"] = json!(0);
    crate::support::wire::assert_wire("challenges.listed", &shown);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].account, "Bob");
    assert_eq!(inbox[0].opponent.name, "Carol");
    assert!(!inbox[0].playable, "bullet is off the Board API");
    assert_eq!(emitted(&host, "challenge:received").len(), 1);

    // Listed again: not announced again.
    online.ongoing().await.unwrap();
    assert_eq!(emitted(&host, "challenge:received").len(), 1);

    // Answered elsewhere: gone from the next listing, gone from the inbox.
    *pending.lock().unwrap() = false;
    online.ongoing().await.unwrap();
    assert!(online.challenges().list().is_empty());
    online.close();
}

/// A challenge listing Lichess refuses leaves the inbox as it was and is logged with its account.
#[tokio::test]
async fn a_refused_challenge_listing_is_logged() {
    let fixture = serve(move |seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/api/challenge" => Reply::Json(400, json!({ "error": "Not now" }).to_string()),
        _ => refuse(seen),
    })
    .await;
    let (host, store, tokens) = rig(&[("Alice", true), ("Bob", true)]);
    let online = session(&fixture.base, &host, &store, &tokens);
    online.stay_connected("Alice");
    fixture.requested("/api/stream/event", 1).await;
    online.ongoing().await.unwrap();
    assert_eq!(fixture.to("/api/challenge").len(), 1);
    assert!(online.challenges().list().is_empty());
    assert!(
        host.logged().contains("Could not load challenges: Bob"),
        "{}",
        host.logged()
    );
    online.close();
}

/// The game's own stream ending it (its final state) frees the board and the engines then, not
/// at some later report.
#[tokio::test]
async fn a_game_ended_on_its_own_stream_frees_the_board_and_engines() {
    let game: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&game);
    let (host, fixture, online) = playing(move |seen| match seen.path() {
        "/api/board/game/stream/AbCd1234" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Some(Reply::Stream(rx))
        }
        _ => None,
    })
    .await;
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let seen = Arc::clone(&busy);
    online.set_state_hook(Arc::new(move |_, blocked| {
        seen.store(blocked, std::sync::atomic::Ordering::SeqCst);
    }));
    fixture
        .requested("/api/board/game/stream/AbCd1234", 1)
        .await;
    let sender = game.lock().unwrap().clone().unwrap();
    let over =
        json!({ "type": "gameState", "moves": "e2e4", "status": "resign", "winner": "black" });
    sender.send(format!("{over}\n")).unwrap();
    until("the final state to reach the window", || {
        emitted(&host, "online:event")
            .iter()
            .any(|event| event["type"] == "gameState" && event["status"] == "resign")
    })
    .await;
    assert!(!online.playing());
    assert!(!busy.load(std::sync::atomic::Ordering::SeqCst));
    online.cancel();
    online.close();
}

/// The engines' busy state (which pauses reviews during a live game) was taken only from the
/// next state report, and a game that ended reported none: reviews stayed paused after it.
#[tokio::test]
async fn a_finished_game_frees_the_engines_at_once() {
    let events: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&events);
    let (host, fixture, online) = playing(move |seen| match seen.path() {
        "/api/stream/event" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Some(Reply::Stream(rx))
        }
        _ => None,
    })
    .await;
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let seen = Arc::clone(&busy);
    online.set_state_hook(Arc::new(move |_, blocked| {
        seen.store(blocked, std::sync::atomic::Ordering::SeqCst);
    }));
    fixture.requested("/api/stream/event", 1).await;
    let sender = events.lock().unwrap().clone().unwrap();
    let finish = json!({
        "type": "gameFinish",
        "game": { "gameId": "AbCd1234", "status": { "name": "resign" }, "winner": "black" }
    });
    sender.send(format!("{finish}\n")).unwrap();
    // Free by the time the window hears of the finish, not at some later report (the silent
    // game stream reports reconnecting only seconds later).
    until("the finish to reach the window", || {
        emitted(&host, "online:event")
            .iter()
            .any(|event| event["type"] == "gameFinish")
    })
    .await;
    assert!(!busy.load(std::sync::atomic::Ordering::SeqCst));
    online.cancel();
    online.close();
}

/// Lichess's account stream can say a game finished before the game's own stream does. The
/// window shows Rematch and New opponent on that `gameFinish`, so the session must end the game
/// then too: a New opponent clicked at once was refused with "Reconnect to your current game
/// before finding another" (the app's `e2e::live_game::new_opponent_seeks_the_same_game_against_anyone`
/// failed this way in CI).
#[tokio::test]
async fn a_game_finished_on_the_account_stream_ends_the_live_game() {
    let events: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>> =
        Arc::new(Mutex::new(None));
    let slot = Arc::clone(&events);
    let (host, fixture, online) = playing(move |seen| match seen.path() {
        "/api/stream/event" => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            *slot.lock().unwrap() = Some(tx);
            Some(Reply::Stream(rx))
        }
        // The game's own stream says nothing more: its final state is late.
        _ => None,
    })
    .await;
    fixture.requested("/api/stream/event", 1).await;
    let sender = events.lock().unwrap().clone().unwrap();
    let finish = json!({
        "type": "gameFinish",
        "game": { "gameId": "AbCd1234", "status": { "name": "resign" }, "winner": "black" }
    });
    sender.send(format!("{finish}\n")).unwrap();
    until("the finish to reach the window", || {
        emitted(&host, "online:event")
            .iter()
            .any(|event| event["type"] == "gameFinish")
    })
    .await;
    assert!(
        !online.playing(),
        "the finished game no longer blocks a new one"
    );
    online.cancel();
    online.close();
}
