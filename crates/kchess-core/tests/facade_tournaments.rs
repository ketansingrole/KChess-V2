//! The facade's tournaments for a signed-in account over a local Lichess: joining keeps the
//! tournament (with its name and end) until it is left, finished or the account withdrawn, and
//! invalid arguments are refused before anything is sent.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]
#[path = "support/lichess_http.rs"]
mod fixtures;

use std::sync::{Arc, Mutex, OnceLock, Weak};

use fixtures::{Reply, serve};
use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

/// Opens the account's stored login as written (`secrets.decrypt`), as a host's secret store.
#[derive(Default)]
struct SignedIn {
    core: OnceLock<Weak<Core>>,
    logs: Mutex<Vec<String>>,
}

impl Host for SignedIn {
    fn log(&self, _: Level, scope: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("[{scope}] {message}"));
    }

    fn emit(&self, event: &str, payload: Value) {
        if event != "host:request" {
            return;
        }
        let Some(core) = self.core.get().and_then(Weak::upgrade) else {
            return;
        };
        let answer = match payload["kind"].as_str() {
            Some("secrets.decrypt") => json!({ "value": payload["payload"] }),
            _ => json!({ "error": "Not in this test." }),
        };
        tokio::spawn(async move {
            core.call("host.reply", vec![payload["id"].clone(), answer])
                .await
                .unwrap();
        });
    }
}

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
    // SAFETY: this test binary has one test; the fixture origin is set before opening the core.
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", &fixture.base);
    }
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(SignedIn::default());
    let core = Arc::new(
        Core::open(
            Config::new(dir.path().to_path_buf()),
            Arc::clone(&host) as Arc<dyn Host>,
        )
        .unwrap(),
    );
    host.core.set(Arc::downgrade(&core)).unwrap();
    core.call("store.games.addAccount", vec![json!("Alice"), json!(true)])
        .await
        .unwrap();
    core.call(
        "store.games.importTokenCiphertext",
        vec![json!("Alice"), json!("lip_alice")],
    )
    .await
    .unwrap();
    let joined = || async {
        core.call("store.library.joinedTournaments", vec![json!(0)])
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
    assert_eq!(list["problems"], json!([]));
    assert_eq!(
        fixture.to("/api/team/of/Alice")[0].header("Authorization"),
        Some("Bearer lip_alice")
    );

    // Joining keeps the tournament under its name, for the joining account.
    let before = kchess_now();
    assert_eq!(
        arena("joinTournament", json!("Alice")).await.unwrap(),
        json!(true)
    );
    let join = &fixture.to("/api/tournament/Abcd1234/join")[0];
    assert_eq!(join.header("Authorization"), Some("Bearer lip_alice"));
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
    assert_eq!(created["id"], json!("Club1234"));
    let posted = fixture
        .requests()
        .into_iter()
        .find(|seen| seen.method == "POST" && seen.path() == "/api/tournament")
        .unwrap();
    assert_eq!(posted.header("Authorization"), Some("Bearer lip_alice"));
    assert!(posted.body.contains("name=Club+Night") || posted.body.contains("name=Club%20Night"));
    core.close().await;
}

fn kchess_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
