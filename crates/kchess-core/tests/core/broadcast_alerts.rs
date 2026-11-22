//! Broadcast alerts: the core keeps the rounds the player chose and watches Lichess's schedule
//! for them, telling each one once when it goes live and never a round nobody chose.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::fixtures::{Recording, Reply, serve};
use kchess_contracts::types::{BroadcastAlert, BroadcastRoundRef, BroadcastTourDetail};
use kchess_core::lichess::watch::alerts::{self, LATER, NEAR, SOON};
use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};

const TOUR: &str = "tourAAAA";
const OTHER: &str = "tourBBBB";
const ROUNDS: [&str; 3] = ["roundA01", "roundA02", "roundA03"];
const OTHER_ROUND: &str = "roundB01";

/// A round as Lichess lists it: under way, over, or due at `starts_at`.
#[derive(Clone, Copy, Default)]
struct State {
    ongoing: bool,
    finished: bool,
}

/// The two tournaments' rounds, which the test moves along.
type Schedule = Arc<Mutex<Vec<(&'static str, &'static str, State)>>>;

fn tour(schedule: &Schedule, id: &str) -> Value {
    let hour = 3_600_000i64;
    let now = kchess_core::time::now_ms();
    let rounds: Vec<Value> = schedule
        .lock()
        .unwrap()
        .iter()
        .filter(|(tour, _, _)| *tour == id)
        .enumerate()
        .map(|(index, (_, round, state))| {
            json!({
                "id": round,
                "name": format!("Round {}", index + 1),
                "ongoing": state.ongoing,
                "finished": state.finished,
                "startsAt": now + (index as i64 - 1) * hour,
            })
        })
        .collect();
    json!({ "tour": { "id": id, "name": format!("Open {id}") }, "rounds": rounds })
}

fn set(schedule: &Schedule, round: &str, state: State) {
    for entry in schedule.lock().unwrap().iter_mut() {
        if entry.1 == round {
            entry.2 = state;
        }
    }
}

const LIVE: State = State {
    ongoing: true,
    finished: false,
};
const OVER: State = State {
    ongoing: false,
    finished: true,
};

/// The rounds told so far, in order.
fn told(host: &Recording) -> Vec<String> {
    host.events
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == "broadcast:round-started")
        .map(|(_, payload)| payload["roundId"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn look(core: &Core) {
    tokio::time::timeout(
        Duration::from_secs(20),
        core.look_at_broadcast_alerts_for_tests(),
    )
    .await
    .expect("the alert watch looked");
}

async fn alerts(core: &Core) -> Vec<BroadcastAlert> {
    serde_json::from_value(core.call("broadcastAlerts", vec![]).await.unwrap()).unwrap()
}

#[tokio::test]
async fn an_alert_tells_a_round_once_when_it_goes_live_and_never_one_not_chosen() {
    let schedule: Schedule = Arc::new(Mutex::new(vec![
        (TOUR, ROUNDS[0], LIVE),
        (TOUR, ROUNDS[1], State::default()),
        (TOUR, ROUNDS[2], State::default()),
        (OTHER, OTHER_ROUND, State::default()),
    ]));
    let served = Arc::clone(&schedule);
    let fixture = serve(move |seen| {
        let path = seen.path();
        for id in [TOUR, OTHER] {
            if path == format!("/api/broadcast/{id}") {
                return Reply::Json(200, tour(&served, id).to_string());
            }
        }
        Reply::Json(404, "{}".into())
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(fixture.base.clone()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.path().to_path_buf())
    };
    let host = Arc::new(Recording::default());
    let core = Core::open(config, host.clone()).unwrap();

    // Round 2 alone; round 1 is already under way and is not told.
    let answer = core
        .call(
            "setBroadcastAlert",
            vec![json!(TOUR), json!(true), json!(ROUNDS[1])],
        )
        .await
        .unwrap();
    let chosen: Vec<BroadcastAlert> = serde_json::from_value(answer).unwrap();
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].round_name.as_deref(), Some("Round 2"));
    look(&core).await;
    assert!(
        told(&host).is_empty(),
        "nothing went live: {:?}",
        told(&host)
    );

    // Round 2 goes live, and so do round 3 and the other tournament's, which nobody chose.
    set(&schedule, ROUNDS[1], LIVE);
    set(&schedule, ROUNDS[2], LIVE);
    set(&schedule, OTHER_ROUND, LIVE);
    look(&core).await;
    assert_eq!(told(&host), [ROUNDS[1]]);
    // Told, the round's alert has nothing left to wait for.
    assert!(alerts(&core).await.is_empty());

    // Looking again tells nothing again.
    look(&core).await;
    assert_eq!(told(&host), [ROUNDS[1]]);
    // The other tournament was never asked for.
    assert!(
        !fixture
            .requests()
            .iter()
            .any(|request| request.path().contains(OTHER))
    );

    // Every round of the tournament: those under way are not told; a later one is, once.
    set(&schedule, ROUNDS[2], State::default());
    core.call("setBroadcastAlert", vec![json!(TOUR), json!(true)])
        .await
        .unwrap();
    look(&core).await;
    assert_eq!(told(&host), [ROUNDS[1]]);
    set(&schedule, ROUNDS[2], LIVE);
    look(&core).await;
    look(&core).await;
    assert_eq!(told(&host), [ROUNDS[1], ROUNDS[2]]);
    assert_eq!(alerts(&core).await.len(), 1, "rounds are still to finish");

    // Once every round has ended the tournament's alert ends too.
    for round in ROUNDS {
        set(&schedule, round, OVER);
    }
    look(&core).await;
    assert!(alerts(&core).await.is_empty());
    // Windows read the alerts again each time the watch changed them.
    let changed = host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, payload)| {
            name == "profile:changed" && payload["areas"] == json!(["broadcasts"])
        })
        .count();
    assert!(
        changed >= 2,
        "the watch's changes were announced: {changed}"
    );

    // Closing the core stops the watch with it.
    tokio::time::timeout(Duration::from_secs(20), core.close())
        .await
        .expect("the core closed");
}

#[tokio::test]
async fn turning_alerts_off_needs_no_lichess_and_a_missing_round_is_refused() {
    let fixture = serve(|seen| {
        if seen.path() == format!("/api/broadcast/{TOUR}") {
            let schedule: Schedule = Arc::new(Mutex::new(vec![(TOUR, ROUNDS[0], LIVE)]));
            return Reply::Json(200, tour(&schedule, TOUR).to_string());
        }
        Reply::Json(404, "{}".into())
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(fixture.base.clone()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.path().to_path_buf())
    };
    let core = Core::open(config, Arc::new(Recording::default())).unwrap();
    let error = core
        .call(
            "setBroadcastAlert",
            vec![json!(TOUR), json!(true), json!("roundZZZ")],
        )
        .await
        .unwrap_err();
    assert_eq!(error.message, "This round is no longer in the broadcast.");
    let error = core
        .call("setBroadcastAlert", vec![json!("not an id"), json!(true)])
        .await
        .unwrap_err();
    assert!(!error.message.is_empty());
    let before = fixture.requests().len();
    let answer = core
        .call("setBroadcastAlert", vec![json!(OTHER), json!(false)])
        .await
        .unwrap();
    assert_eq!(answer, json!([]));
    assert_eq!(fixture.requests().len(), before);
    core.close().await;
}

fn detail(rounds: &[(&str, State, Option<i64>)]) -> BroadcastTourDetail {
    BroadcastTourDetail {
        id: TOUR.into(),
        name: "Open".into(),
        description: None,
        rounds: rounds
            .iter()
            .enumerate()
            .map(|(index, (id, state, starts_at))| BroadcastRoundRef {
                id: (*id).into(),
                name: format!("Round {}", index + 1),
                ongoing: state.ongoing,
                finished: state.finished,
                starts_at: *starts_at,
            })
            .collect(),
        default_round_id: None,
    }
}

/// A round due soon is looked for every minute; one far off at most every 15 minutes, and just
/// before it is due; a failure waits longer each time, up to half an hour.
#[test]
fn the_watch_looks_often_only_near_a_chosen_rounds_start() {
    let now = 1_800_000_000_000i64;
    let minute = 60_000;
    let mut alert = Vec::new();
    alerts::set(
        &mut alert,
        TOUR,
        None,
        true,
        Some(&detail(&[(ROUNDS[0], State::default(), None)])),
    )
    .unwrap();
    let wait = |starts_at: Option<i64>| {
        alerts::look(
            &alert[0],
            &detail(&[(ROUNDS[0], State::default(), starts_at)]),
            now,
        )
        .next
    };
    assert_eq!(wait(Some(now + 5 * minute)), SOON);
    assert_eq!(wait(Some(now - 30 * minute)), SOON, "a round running late");
    assert_eq!(wait(Some(now + 12 * minute)), Duration::from_secs(2 * 60));
    assert_eq!(wait(Some(now + 10 * 24 * 60 * minute)), LATER);
    assert_eq!(wait(None), LATER);
    assert_eq!(wait(Some(now - 5 * 60 * minute)), LATER, "long overdue");
    assert!(NEAR > SOON);
    assert_eq!(alerts::retry_after(1), alerts::RETRY);
    assert_eq!(alerts::retry_after(2), alerts::RETRY * 2);
    assert_eq!(alerts::retry_after(30), LATER * 2);
}

/// Turning one round off under a whole tournament's alert keeps the rounds still to come.
#[test]
fn one_round_off_under_a_tournaments_alert_keeps_the_others() {
    let tour = detail(&[
        (ROUNDS[0], LIVE, None),
        (ROUNDS[1], State::default(), None),
        (ROUNDS[2], State::default(), None),
    ]);
    let mut alerts_set = Vec::new();
    alerts::set(&mut alerts_set, TOUR, None, true, Some(&tour)).unwrap();
    assert_eq!(alerts_set[0].told, [ROUNDS[0]]);
    // A round's alert under the tournament's changes nothing.
    alerts::set(&mut alerts_set, TOUR, Some(ROUNDS[1]), true, Some(&tour)).unwrap();
    assert_eq!(alerts_set.len(), 1);
    alerts::set(&mut alerts_set, TOUR, Some(ROUNDS[1]), false, Some(&tour)).unwrap();
    let rounds: Vec<_> = alerts_set
        .iter()
        .map(|alert| alert.round_id.as_deref())
        .collect();
    assert_eq!(rounds, [Some(ROUNDS[2])]);
    alerts::set(&mut alerts_set, TOUR, None, false, None).unwrap();
    assert!(alerts_set.is_empty());
}
