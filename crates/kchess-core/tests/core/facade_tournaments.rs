//! The facade's tournaments for a signed-in account over a local Lichess: joining keeps the
//! tournament (with its name and end) until it is left, finished or the account withdrawn, and
//! invalid arguments are refused before anything is sent.
use crate::fixtures;
use crate::signed_in;

use std::sync::{Arc, Mutex};

use fixtures::{Reply, serve};
use serde_json::{Value, json};
use signed_in::ALICE_LOGIN;

/// The arena as Lichess answers `GET /api/tournament/{id}`, with the account's place when it is in.
fn detail(me: bool, finished: bool) -> Value {
    let mut arena = json!({
        "id": "Abcd1234",
        "fullName": "Hourly Rapid",
        "clock": { "limit": 600, "increment": 0 },
        "variant": "standard",
        "rated": true,
        "nbPlayers": 2,
        "startsAt": "2026-11-01T18:00:00Z",
        "isFinished": finished,
        "standing": { "page": 1, "players": [] }
    });
    if me {
        arena["me"] = json!({ "rank": 1, "withdraw": false });
    }
    arena
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_joined_tournament_is_kept_until_it_is_left_withdrawn_or_finished() {
    // Whether Alice is in the arena, and whether it has finished.
    let state = Arc::new(Mutex::new((false, false)));
    let fixture = serve({
        let state = Arc::clone(&state);
        move |seen| match (seen.method.as_str(), seen.path()) {
            (_, "/api/stream/event") => Reply::Hang,
            ("GET", "/api/tournament") => Reply::Json(
                200,
                json!({ "created": [], "started": [], "finished": [] }).to_string(),
            ),
            ("GET", "/api/team/of/Alice") => Reply::Json(200, "[]".into()),
            ("GET", "/api/tournament/Abcd1234") => {
                let (me, finished) = *state.lock().unwrap();
                Reply::Json(200, detail(me, finished).to_string())
            }
            ("POST", "/api/tournament/Abcd1234/join") => {
                state.lock().unwrap().0 = true;
                Reply::Json(200, json!({ "ok": true }).to_string())
            }
            ("POST", "/api/tournament") => Reply::Json(
                200,
                json!({
                    "id": "Club1234", "fullName": "Club Night Arena", "status": 10,
                    "variant": { "key": "standard", "name": "Standard" }, "rated": true,
                    "clock": { "limit": 600, "increment": 0 }, "minutes": 60, "nbPlayers": 0,
                    "startsAt": 1_793_556_000_000i64
                })
                .to_string(),
            ),
            ("POST", "/api/tournament/Abcd1234/withdraw") => {
                state.lock().unwrap().0 = false;
                Reply::Json(200, json!({ "ok": true }).to_string())
            }
            _ => Reply::Json(404, "{}".into()),
        }
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    let joined = || async {
        core.call("joinedTournaments", vec![])
            .await
            .unwrap()
            .as_array()
            .cloned()
            .unwrap()
    };
    let arena = |method: &'static str, account: Value| {
        let mut args = vec![json!("arena"), json!("Abcd1234"), account];
        if method == "joinTournament" {
            args.push(Value::Null);
        }
        core.call(method, args)
    };

    // Invalid arguments are refused before anything is sent.
    for (method, args) in [
        (
            "tournament",
            vec![json!("ladder"), json!("Abcd1234"), Value::Null, json!(1)],
        ),
        (
            "tournament",
            vec![json!("arena"), json!("../x"), Value::Null, json!(1)],
        ),
        (
            "joinTournament",
            vec![
                json!("arena"),
                json!("Abcd1234"),
                json!("not a name!"),
                Value::Null,
            ],
        ),
        (
            "leaveTournament",
            vec![json!("swiss"), json!("Abcd1234"), json!(3)],
        ),
        (
            "createTournament",
            vec![json!("Alice"), json!({ "minutes": 60 })],
        ),
    ] {
        assert!(core.call(method, args).await.is_err(), "{method}");
    }
    assert!(fixture.requests().is_empty());

    let list = core
        .call("tournaments", vec![json!("Alice")])
        .await
        .unwrap();
    crate::support::wire::assert_wire("tournaments.facade", &list);
    assert_eq!(list["problems"], json!([]));
    assert_eq!(
        fixture.to("/api/team/of/Alice")[0].header("Authorization"),
        Some(ALICE_LOGIN)
    );

    // Joining keeps the tournament under its name, for the joining account.
    let before = kchess_core::time::now_ms();
    assert_eq!(
        arena("joinTournament", json!("Alice")).await.unwrap(),
        json!(true)
    );
    let join = &fixture.to("/api/tournament/Abcd1234/join")[0];
    assert_eq!(join.header("Authorization"), Some(ALICE_LOGIN));
    let entries = joined().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], json!("Hourly Rapid"));
    assert_eq!(entries[0]["account"], json!("Alice"));
    // An arena's detail has no end, so the entry lasts a day.
    assert!(entries[0]["until"].as_i64().unwrap() >= before + 86_000_000);

    // Looking it up while still in it keeps it; once withdrawn on Lichess, it goes.
    let page = core
        .call(
            "tournament",
            vec![
                json!("arena"),
                json!("Abcd1234"),
                json!("Alice"),
                Value::Null,
            ],
        )
        .await
        .unwrap();
    crate::support::wire::assert_wire("tournament.facade", &page);
    assert_eq!(page["standingPage"], json!(1), "no page reads the first");
    assert_eq!(joined().await.len(), 1);
    state.lock().unwrap().0 = false;
    arena("tournament", json!("Alice")).await.ok();
    assert_eq!(joined().await, Vec::<Value>::new());

    // Leaving forgets it.
    arena("joinTournament", json!("Alice")).await.unwrap();
    assert_eq!(joined().await.len(), 1);
    assert_eq!(
        arena("leaveTournament", json!("Alice")).await.unwrap(),
        json!(true)
    );
    assert_eq!(joined().await, Vec::<Value>::new());

    // A finished arena is forgotten for every account, even looked up anonymously.
    arena("joinTournament", json!("Alice")).await.unwrap();
    state.lock().unwrap().1 = true;
    let finished = core
        .call(
            "tournament",
            vec![json!("arena"), json!("Abcd1234"), json!(""), json!(1)],
        )
        .await
        .unwrap();
    crate::support::wire::assert_wire("tournament.finished", &finished);
    assert_eq!(finished["status"], json!("finished"));
    assert_eq!(joined().await, Vec::<Value>::new());

    // An account without a login is asked to reconnect, and nothing is kept.
    assert_eq!(
        arena("joinTournament", json!("Bob")).await.unwrap(),
        json!({ "needsReconnect": true })
    );
    assert_eq!(joined().await, Vec::<Value>::new());

    // An arena is created as the chosen account.
    let created = core
        .call(
            "createTournament",
            vec![
                json!("Alice"),
                json!({ "name": "Club Night", "clockTime": 10, "clockIncrement": 0, "minutes": 60,
                        "variant": "standard", "rated": true }),
            ],
        )
        .await
        .unwrap();
    crate::support::wire::assert_wire("createTournament.facade", &created);
    assert_eq!(created["id"], json!("Club1234"));
    let posted = fixture
        .requests()
        .into_iter()
        .find(|seen| seen.method == "POST" && seen.path() == "/api/tournament")
        .unwrap();
    assert_eq!(posted.header("Authorization"), Some(ALICE_LOGIN));
    assert!(posted.body.contains("name=Club+Night") || posted.body.contains("name=Club%20Night"));
    core.close().await;
}

/// A lookup of the arena that left before another window joined it, and answers after the join
/// was kept, keeps the membership: its answer from before the join erased it, and the pairings
/// stopped keeping the account's stream open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lookup_from_before_a_join_keeps_the_membership() {
    let (release, gate) = tokio::sync::watch::channel(false);
    let looked_up = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let joined_in = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Told once the held lookup reached the handler: the fixture records a request before its
    // handler runs, so the join's own lookup could otherwise be the one held.
    let held = Arc::new(tokio::sync::Notify::new());
    let streams: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>> = Arc::default();
    let fixture = serve({
        let (looked_up, joined_in) = (Arc::clone(&looked_up), Arc::clone(&joined_in));
        let held = Arc::clone(&held);
        let streams = Arc::clone(&streams);
        move |seen| match (seen.method.as_str(), seen.path()) {
            // An open stream: one that never answers would hold the second of the two requests
            // Lichess is asked at once, and the join's own lookup waited behind the held one.
            (_, "/api/stream/event") => {
                let (sender, events) = tokio::sync::mpsc::unbounded_channel();
                streams.lock().unwrap().push(sender);
                Reply::Stream(events)
            }
            ("GET", "/api/tournament/Abcd1234") => {
                let first = looked_up.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                let me = joined_in.load(std::sync::atomic::Ordering::SeqCst);
                let answer = Reply::Json(200, detail(me && !first, false).to_string());
                if first {
                    held.notify_one();
                    Reply::Gated(gate.clone(), Box::new(answer))
                } else {
                    answer
                }
            }
            ("POST", "/api/tournament/Abcd1234/join") => {
                joined_in.store(true, std::sync::atomic::Ordering::SeqCst);
                Reply::Json(200, json!({ "ok": true }).to_string())
            }
            _ => Reply::Json(404, "{}".into()),
        }
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = Arc::new(profile.core.clone());
    let lookup = tokio::spawn({
        let core = Arc::clone(&core);
        async move {
            core.call(
                "tournament",
                vec![json!("arena"), json!("Abcd1234"), json!("Alice"), json!(1)],
            )
            .await
        }
    });
    held.notified().await;
    assert_eq!(
        core.call(
            "joinTournament",
            vec![
                json!("arena"),
                json!("Abcd1234"),
                json!("Alice"),
                Value::Null
            ],
        )
        .await
        .unwrap(),
        json!(true)
    );
    let _ = release.send(true);
    lookup.await.unwrap().expect("the older lookup");
    let joined = core.call("joinedTournaments", vec![]).await.unwrap();
    assert_eq!(joined.as_array().map(Vec::len), Some(1), "{joined}");
    core.close().await;
}

/// A lookup begun while joined that answers "withdrawn" after the account left and joined again
/// keeps the new membership: the answer is about the membership that was left, and it erased the
/// new one, so pairings stopped keeping the account's stream open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lookup_from_before_a_leave_and_rejoin_keeps_the_membership() {
    let (release, gate) = tokio::sync::watch::channel(false);
    let looked_up = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let held = Arc::new(tokio::sync::Notify::new());
    let streams: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>> = Arc::default();
    let fixture = serve({
        let looked_up = Arc::clone(&looked_up);
        let held = Arc::clone(&held);
        let streams = Arc::clone(&streams);
        move |seen| match (seen.method.as_str(), seen.path()) {
            (_, "/api/stream/event") => {
                let (sender, events) = tokio::sync::mpsc::unbounded_channel();
                streams.lock().unwrap().push(sender);
                Reply::Stream(events)
            }
            // The join's lookups find Alice in; the second lookup, held, saw her withdrawn.
            ("GET", "/api/tournament/Abcd1234") => {
                if looked_up.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                    held.notify_one();
                    let mut withdrawn = detail(true, false);
                    withdrawn["me"]["withdraw"] = json!(true);
                    Reply::Gated(
                        gate.clone(),
                        Box::new(Reply::Json(200, withdrawn.to_string())),
                    )
                } else {
                    Reply::Json(200, detail(true, false).to_string())
                }
            }
            ("POST", "/api/tournament/Abcd1234/join" | "/api/tournament/Abcd1234/withdraw") => {
                Reply::Json(200, json!({ "ok": true }).to_string())
            }
            _ => Reply::Json(404, "{}".into()),
        }
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = Arc::new(profile.core.clone());
    let join = || {
        core.call(
            "joinTournament",
            vec![
                json!("arena"),
                json!("Abcd1234"),
                json!("Alice"),
                Value::Null,
            ],
        )
    };
    assert_eq!(join().await.unwrap(), json!(true));
    let lookup = tokio::spawn({
        let core = Arc::clone(&core);
        async move {
            core.call(
                "tournament",
                vec![json!("arena"), json!("Abcd1234"), json!("Alice"), json!(1)],
            )
            .await
        }
    });
    held.notified().await;
    assert_eq!(
        core.call(
            "leaveTournament",
            vec![json!("arena"), json!("Abcd1234"), json!("Alice")],
        )
        .await
        .unwrap(),
        json!(true)
    );
    assert_eq!(join().await.unwrap(), json!(true));
    let _ = release.send(true);
    lookup.await.unwrap().expect("the older lookup");
    let joined = core.call("joinedTournaments", vec![]).await.unwrap();
    assert_eq!(joined.as_array().map(Vec::len), Some(1), "{joined}");
    core.close().await;
}

/// 2026-11-15: a join waiting for the arena's details while its account logged out still
/// remembered the tournament afterwards, so the lobby reopened for an account that was gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_join_answered_after_a_logout_remembers_nothing() {
    let (details, held) = tokio::sync::mpsc::unbounded_channel::<String>();
    let held = Mutex::new(Some(held));
    let fixture = serve(move |seen| match (seen.method.as_str(), seen.path()) {
        (_, "/api/stream/event") => Reply::Hang,
        ("POST", "/api/tournament/Abcd1234/join") => {
            Reply::Json(200, json!({ "ok": true }).to_string())
        }
        ("GET", "/api/tournament/Abcd1234") => match held.lock().unwrap().take() {
            Some(held) => Reply::Stream(held),
            None => Reply::Json(200, detail(true, false).to_string()),
        },
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = &profile.core;
    let join = core.call(
        "joinTournament",
        vec![
            json!("arena"),
            json!("Abcd1234"),
            json!("Alice"),
            Value::Null,
        ],
    );
    let logout = async {
        crate::support::wait_until(
            std::time::Duration::from_secs(10),
            "the join to ask for the details",
            || (!fixture.to("/api/tournament/Abcd1234").is_empty()).then_some(()),
        )
        .await;
        core.call("logout", vec![json!("Alice")]).await.unwrap();
        details.send(detail(true, false).to_string()).unwrap();
        drop(details);
    };
    let (joined, ()) = tokio::join!(join, logout);
    assert_eq!(joined.unwrap(), json!(true));
    let remembered = core.call("joinedTournaments", vec![]).await.unwrap();
    assert_eq!(
        remembered,
        json!([]),
        "the logged-out account's join was kept"
    );
}
