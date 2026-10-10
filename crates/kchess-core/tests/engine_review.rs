//! Game review through the scripted engine (`core/tests/unit/review-engine.test.ts`,
//! `review-queue.test.ts` and `review-associations.test.ts`): the quick and deep passes, automatic
//! reviews that ask Lichess first, pauses, queue order, discarding an account's reviews,
//! cancellation keeping finished work, battery throttling, game links, and a computer move
//! taking the engine from a running review.

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kchess_core::engine::review::{Pause, ReviewEval, ReviewRequest, Reviews, StoredReview};
use kchess_core::engine::scheduler::Scheduler;
use kchess_core::engine::search::Search;
use serde_json::{Map, json};
use support::review_store::{GameRow, MemoryStore, ScriptedHost, settings};
use support::{Recording, context, fake_path, log_path, received, scheduler, wait_until};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// 3. Qh5 Nf6?? 4. Qxf7#: Black's third move loses (the scripted engine sees it as a blunder).
const SCHOLARS: [&str; 7] = ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"];
const WAIT: Duration = Duration::from_secs(30);

struct Rig {
    reviews: Reviews,
    host: Arc<ScriptedHost>,
    store: Arc<MemoryStore>,
    recording: Arc<Recording>,
    log: PathBuf,
}

/// Reviews with the engine `mode` on `scheduler`, storing into a fresh in-memory store.
fn rig_on(scheduler: Arc<Scheduler>, mode: &str, name: &str, auto: &str, on_battery: bool) -> Rig {
    let store = MemoryStore::new();
    let recording = support::host();
    let log = log_path(name);
    let ctx = context(&recording, scheduler, mode, &log);
    let reviews = Reviews::new(ctx, store.clone());
    let host = ScriptedHost::new(store.clone(), settings(auto, on_battery));
    reviews.setup_reviews(host.clone());
    Rig {
        reviews,
        host,
        store,
        recording,
        log,
    }
}

fn rig(mode: &str, name: &str, battery: bool, auto: &str, on_battery: bool) -> Rig {
    rig_on(scheduler(4, battery), mode, name, auto, on_battery)
}

fn moves(list: &[&str]) -> Vec<String> {
    list.iter().map(|mv| (*mv).to_string()).collect()
}

fn request(list: &[&str]) -> ReviewRequest {
    ReviewRequest {
        fen: START.to_string(),
        moves: moves(list),
        game_id: None,
        account: None,
    }
}

/// `reviewKey(fen, moves)`, as the store and the queue key a review.
fn key_of(fen: &str, list: &[&str]) -> String {
    let out =
        kchess_domain::api::call("reviewKey", &json!([fen, list]).to_string()).expect("reviewKey");
    serde_json::from_str::<String>(&out).expect("a key")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(0))
}

fn updates_of(rig: &Rig) -> Vec<StoredReview> {
    rig.host
        .updates
        .lock()
        .expect("updates")
        .iter()
        .map(|update| update.review.clone())
        .collect()
}

#[tokio::test]
async fn a_quick_pass_shows_first_then_the_deep_pass_completes_and_finds_the_blunder() {
    let rig = rig("review", "scholar", false, "off", true);
    assert!(
        rig.reviews
            .request_review(request(&SCHOLARS))
            .expect("the request")
            .is_none()
    );
    let quick = wait_until(WAIT, "the quick pass", || {
        updates_of(&rig)
            .into_iter()
            .find(|review| !review.complete && review.evals.iter().all(Option::is_some))
    })
    .await;
    let (last, scored) = quick.evals.split_last().expect("positions");
    assert!(scored.iter().flatten().all(|eval| eval.depth == Some(10.0)));
    assert!(last.is_some());
    let full = wait_until(WAIT, "the deep pass", || {
        rig.host.complete_updates().into_iter().next()
    })
    .await;
    let (last, scored) = full.evals.split_last().expect("positions");
    assert!(scored.iter().flatten().all(|eval| eval.depth == Some(18.0)));
    // Checkmate needs no engine.
    assert_eq!(
        last,
        &Some(ReviewEval {
            mate: Some(0.0),
            ..ReviewEval::default()
        })
    );
    let analysis =
        kchess_domain::review::analyse_review(&serde_json::to_value(&full).expect("json"))
            .expect("an analysis");
    assert_eq!(analysis["black"]["blunder"], json!(1), "Nf6 is the blunder");
    assert_eq!(
        full.evals[5]
            .as_ref()
            .and_then(|eval| eval.best.clone())
            .as_deref(),
        Some("d8e7")
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn a_request_for_a_finished_review_answers_from_the_store() {
    let rig = rig("review", "stored", false, "off", true);
    rig.reviews
        .request_review(request(&SCHOLARS))
        .expect("the request");
    wait_until(WAIT, "the complete review", || {
        rig.host.complete_updates().into_iter().next()
    })
    .await;
    let stored = rig
        .reviews
        .request_review(request(&SCHOLARS))
        .expect("the request")
        .expect("the stored review");
    assert!(stored.complete);
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn automatic_reviews_ask_lichess_first_and_leave_variants_alone() {
    let rig = rig("review", "auto", false, "recent", true);
    rig.store.add_game(GameRow {
        id: "scholar1".into(),
        account: "tester".into(),
        moves: "e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#".into(),
        pgn: None,
        perf: "blitz".into(),
        created_at: now_ms(),
    });
    rig.store.add_game(GameRow {
        id: "variant1".into(),
        account: "tester".into(),
        moves: "e4 e5".into(),
        pgn: None,
        perf: "chess960".into(),
        created_at: now_ms(),
    });
    rig.reviews.reviews_changed();
    let done = wait_until(WAIT, "the automatic review", || {
        rig.host
            .complete_updates()
            .into_iter()
            .find(|review| review.game_id.as_deref() == Some("scholar1"))
    })
    .await;
    assert_eq!(
        *rig.host.fetched.lock().expect("fetched"),
        vec![("tester".to_string(), vec!["scholar1".to_string()])]
    );
    assert!(rig.store.linked("scholar1", &done.key));
    assert!(
        rig.host
            .complete_updates()
            .iter()
            .all(|review| review.game_id.as_deref() != Some("variant1")),
        "variants are not reviewed"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn automatic_reviews_wait_while_the_engine_is_needed_and_stop_when_switched_off() {
    let rig = rig("review", "waiting", false, "all", true);
    rig.store.add_game(GameRow {
        id: "waiting1".into(),
        account: "tester".into(),
        moves: "d4 d5 c4".into(),
        pgn: None,
        perf: "blitz".into(),
        created_at: now_ms(),
    });
    *rig.host.busy.lock().expect("busy") = Some(Pause::Engine);
    rig.reviews.reviews_changed();
    wait_until(WAIT, "the engine pause", || {
        (rig.host.last_status()?.paused == Some(Pause::Engine)).then_some(())
    })
    .await;
    assert!(rig.host.complete_updates().is_empty());
    *rig.host.busy.lock().expect("busy") = None;
    *rig.host.settings.lock().expect("settings") = settings("off", true);
    rig.reviews.reviews_changed();
    wait_until(WAIT, "the off pause", || {
        (rig.host.last_status()?.paused == Some(Pause::Off)).then_some(())
    })
    .await;
    assert!(rig.host.complete_updates().is_empty());
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn battery_power_pauses_automatic_reviews_unless_allowed() {
    let rig = rig("review", "battery", true, "all", false);
    rig.reviews.reviews_changed();
    wait_until(WAIT, "the battery pause", || {
        (rig.host.last_status()?.paused == Some(Pause::Battery)).then_some(())
    })
    .await;
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn the_most_recent_request_is_reviewed_first() {
    let rig = rig("review", "order", false, "off", true);
    // Lichess is busy with a live game: nothing starts, so both requests queue.
    *rig.host.busy.lock().expect("busy") = Some(Pause::Online);
    rig.reviews
        .request_review(request(&SCHOLARS))
        .expect("first");
    let second = ["e2e4", "e7e5", "g1f3"];
    rig.reviews
        .request_review(request(&second))
        .expect("second");
    wait_until(WAIT, "both reviews queued", || {
        (rig.reviews.review_status().waiting == 2).then_some(())
    })
    .await;
    *rig.host.busy.lock().expect("busy") = None;
    rig.reviews.reviews_changed();
    wait_until(WAIT, "both reviews", || {
        (rig.host.complete_updates().len() >= 2).then_some(())
    })
    .await;
    let completed = rig.host.complete_updates();
    assert_eq!(
        completed[0].moves,
        moves(&second),
        "the game asked for last goes first"
    );
    assert_eq!(completed[1].moves, moves(&SCHOLARS));
    rig.reviews.stop_reviews().await;
}

/// Regression: the TypeScript queue yielded to *any* queued request, so a running review and a
/// queued one gave way to each other forever and neither finished. A review now yields only to
/// a request made after it.
#[tokio::test]
async fn two_requested_reviews_both_finish_instead_of_yielding_to_each_other() {
    let rig = rig("slow-review", "no-livelock", false, "off", true);
    rig.reviews
        .request_review(request(&SCHOLARS))
        .expect("first");
    // The quick pass is published while the deep pass still runs: the second request overlaps it.
    wait_until(WAIT, "the first review's quick pass", || {
        updates_of(&rig)
            .iter()
            .any(|review| !review.complete)
            .then_some(())
    })
    .await;
    assert!(
        rig.host.complete_updates().is_empty(),
        "the first review is still running"
    );
    let second = ["e2e4", "e7e5", "g1f3"];
    rig.reviews
        .request_review(request(&second))
        .expect("second");
    wait_until(WAIT, "both reviews", || {
        (rig.host.complete_updates().len() >= 2).then_some(())
    })
    .await;
    let completed = rig.host.complete_updates();
    assert_eq!(
        completed[0].moves,
        moves(&second),
        "the newer request goes first"
    );
    assert_eq!(
        completed[1].moves,
        moves(&SCHOLARS),
        "then the first one finishes"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn discarding_an_account_drops_its_queued_reviews() {
    let rig = rig("review", "discard-queued", false, "off", true);
    rig.store.add_account("tester");
    *rig.host.busy.lock().expect("busy") = Some(Pause::Online);
    let mut queued = request(&SCHOLARS);
    queued.account = Some("tester".into());
    queued.game_id = Some("g1".into());
    rig.reviews.request_review(queued).expect("the request");
    wait_until(WAIT, "the queued review", || {
        (rig.reviews.review_status().waiting == 1).then_some(())
    })
    .await;
    rig.reviews.discard_account_reviews(&["TESTER".to_string()]);
    assert_eq!(rig.reviews.review_status().waiting, 0);
    *rig.host.busy.lock().expect("busy") = None;
    rig.reviews.reviews_changed();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        updates_of(&rig).is_empty(),
        "nothing was reviewed for the discarded account"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn discarding_an_account_while_its_review_runs_stores_nothing_more() {
    let rig = rig("stall", "discard-running", false, "off", true);
    rig.store.add_account("tester");
    let mut running = request(&SCHOLARS);
    running.account = Some("tester".into());
    running.game_id = Some("g1".into());
    rig.reviews.request_review(running).expect("the request");
    // The quick pass finishes; the deep pass stalls in the engine.
    wait_until(WAIT, "the quick pass", || {
        updates_of(&rig)
            .into_iter()
            .find(|review| !review.complete && review.evals.iter().all(Option::is_some))
    })
    .await;
    rig.reviews.discard_account_reviews(&["tester".to_string()]);
    wait_until(WAIT, "the discarded review to end", || {
        (rig.host.last_status()?.current.is_none()).then_some(())
    })
    .await;
    let written = updates_of(&rig).len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        updates_of(&rig).len(),
        written,
        "no output after the discard"
    );
    assert!(
        received(&rig.log).iter().any(|line| line == "stop"),
        "the engine was stopped"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn cancelling_keeps_the_finished_quick_pass_and_is_not_complete() {
    let rig = rig("stall", "cancel", false, "off", true);
    rig.reviews
        .request_review(request(&SCHOLARS))
        .expect("the request");
    let key = key_of(START, &SCHOLARS);
    wait_until(WAIT, "the quick pass", || {
        updates_of(&rig)
            .into_iter()
            .find(|review| !review.complete && review.evals.iter().all(Option::is_some))
    })
    .await;
    rig.reviews.cancel_review(&key);
    wait_until(WAIT, "the cancelled review to end", || {
        (rig.host.last_status()?.current.is_none()).then_some(())
    })
    .await;
    let stored = rig.store.review(&key).expect("the partial review is kept");
    assert!(!stored.complete, "an interrupted review is never complete");
    assert!(stored.evals.iter().all(Option::is_some));
    // The last position is the mate, which needs no engine; the others keep the quick pass.
    let (_, scored) = stored.evals.split_last().expect("positions");
    assert!(scored.iter().flatten().all(|eval| eval.depth == Some(10.0)));
    assert!(received(&rig.log).iter().any(|line| line == "stop"));
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn battery_power_limits_the_threads_a_review_may_use() {
    let rig = rig("review", "battery-threads", true, "off", true);
    rig.reviews
        .request_review(request(&["e2e4", "e7e5"]))
        .expect("the request");
    wait_until(WAIT, "the review", || {
        rig.host.complete_updates().into_iter().next()
    })
    .await;
    assert!(
        received(&rig.log)
            .iter()
            .any(|line| line == "setoption name Threads value 2")
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn a_cached_lichess_review_is_linked_to_the_requesting_game() {
    let rig = rig("review", "links", false, "off", true);
    let key = key_of(START, &["e2e4", "e7e5"]);
    rig.store.put(StoredReview {
        key: key.clone(),
        fen: START.into(),
        moves: moves(&["e2e4", "e7e5"]),
        source: "lichess".into(),
        engine: None,
        evals: vec![None, None, None],
        depth: 0,
        complete: true,
        updated_at: 1,
        game_id: None,
        extra: Map::new(),
    });
    let mut asking = request(&["e2e4", "e7e5"]);
    asking.game_id = Some("Game0002".into());
    let stored = rig
        .reviews
        .request_review(asking)
        .expect("the request")
        .expect("the cached review");
    assert_eq!(stored.game_id.as_deref(), Some("Game0002"));
    assert!(
        rig.store.linked("Game0002", &key),
        "linked before the cached review is returned"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn identical_queued_searches_keep_an_association_for_each_game() {
    let rig = rig("review", "associations", false, "off", true);
    *rig.host.busy.lock().expect("busy") = Some(Pause::Online);
    let key = key_of(START, &SCHOLARS);
    for game in ["Game0001", "Game0002"] {
        let mut asking = request(&SCHOLARS);
        asking.game_id = Some(game.into());
        rig.reviews.request_review(asking).expect("the request");
    }
    assert!(rig.store.linked("Game0001", &key));
    assert!(rig.store.linked("Game0002", &key));
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn a_request_for_an_account_that_is_gone_stores_nothing() {
    let rig = rig("review", "gone", false, "off", true);
    let mut asking = request(&SCHOLARS);
    asking.account = Some("gone".into());
    asking.game_id = Some("g9".into());
    assert!(
        rig.reviews
            .request_review(asking)
            .expect("the request")
            .is_none()
    );
    assert!(rig.store.review(&key_of(START, &SCHOLARS)).is_none());
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn a_finished_review_is_not_queued_again_automatically() {
    let rig = rig("review", "finished", false, "all", true);
    rig.store.add_game(GameRow {
        id: "done1".into(),
        account: "tester".into(),
        moves: "e4 e5".into(),
        pgn: None,
        perf: "blitz".into(),
        created_at: now_ms(),
    });
    // Reviewed by hand first (the analysis board): the game is linked to the finished review.
    let mut asking = request(&["e2e4", "e7e5"]);
    asking.game_id = Some("done1".into());
    rig.reviews.request_review(asking).expect("the request");
    wait_until(WAIT, "the finished review", || {
        rig.host.complete_updates().into_iter().next()
    })
    .await;
    let finished = updates_of(&rig).len();
    rig.reviews.reviews_changed();
    wait_until(WAIT, "the queue to look again", || {
        (rig.host.last_status()?.waiting == 0).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        updates_of(&rig).len(),
        finished,
        "the finished game is not reviewed again"
    );
    assert!(
        rig.host.fetched.lock().expect("fetched").is_empty(),
        "nor looked up on Lichess"
    );
    rig.reviews.stop_reviews().await;
}

#[tokio::test]
async fn a_computer_move_takes_the_engine_from_a_running_review() {
    let shared = scheduler(4, false);
    let rig = rig_on(Arc::clone(&shared), "stall", "preempt", "off", true);
    rig.reviews
        .request_review(request(&SCHOLARS))
        .expect("the request");
    wait_until(WAIT, "the quick pass", || {
        updates_of(&rig)
            .into_iter()
            .find(|review| !review.complete && review.evals.iter().all(Option::is_some))
    })
    .await;
    let search_log = log_path("preempt-search");
    let search = Search::new(context(
        &rig.recording,
        Arc::clone(&shared),
        "search",
        &search_log,
    ));
    let mv = tokio::time::timeout(
        Duration::from_secs(20),
        search.best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 })),
    )
    .await
    .expect("the computer is not kept waiting")
    .expect("a move");
    assert_eq!(mv, "e2e4");
    assert!(
        received(&rig.log).iter().any(|line| line == "stop"),
        "the review was stopped"
    );
    assert!(
        rig.host.complete_updates().is_empty(),
        "the interrupted review did not complete"
    );
    rig.reviews.stop_reviews().await;
}
