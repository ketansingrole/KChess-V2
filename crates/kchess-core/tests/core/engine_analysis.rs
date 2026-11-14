//! Analysis through the scripted engine: streamed
//! lines, MultiPV, the repetition context, superseded requests, interrupted and completed output,
//! and the payload shape of `engine:analysis`.

use crate::support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::engine::analysis::{Analysis, AnalysisSink, AnalysisUpdate, Reason};
use kchess_domain::replay::replay_positions;
use serde_json::json;
use support::{context, fake_path, log_path, received, scheduler, wait_until};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const ITALIAN: &str = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";
/// White mates in one with Ra8#.
const MATE_IN_ONE: &str = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
const WAIT: Duration = Duration::from_secs(20);

type Updates = Arc<Mutex<Vec<AnalysisUpdate>>>;

fn recorder() -> (Updates, AnalysisSink) {
    let updates: Updates = Arc::new(Mutex::new(Vec::new()));
    let sink_updates = Arc::clone(&updates);
    let sink: AnalysisSink = Arc::new(move |update| {
        sink_updates.lock().expect("updates").push(update);
    });
    (updates, sink)
}

fn find(updates: &Updates, test: impl Fn(&AnalysisUpdate) -> bool) -> Option<AnalysisUpdate> {
    updates
        .lock()
        .expect("updates")
        .iter()
        .find(|update| test(update))
        .cloned()
}

fn analysis_with(mode: &str, log: &std::path::Path) -> (Analysis, Arc<support::Recording>) {
    let host = support::host();
    let ctx = context(&host, scheduler(4, false), mode, log);
    (Analysis::new(ctx), host)
}

#[tokio::test]
async fn streams_scored_lines_for_a_position() {
    let log = log_path("analysis-stream");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": START, "lines": 2, "infinite": true }),
            &fake_path(),
            sink,
        )
        .expect("a valid request");
    let update = wait_until(WAIT, "two scored lines", || {
        find(&updates, |u| {
            u.id == id && u.lines.len() == 2 && u.depth >= 8.0
        })
    })
    .await;
    assert_eq!(update.fen, START);
    assert!(!update.done);
    assert_eq!(
        update
            .lines
            .iter()
            .map(|line| line.rank)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    for line in &update.lines {
        assert!(
            line.pv[0].len() == 4 || line.pv[0].len() == 5,
            "a UCI move: {:?}",
            line.pv
        );
        assert!(line.cp.is_some_and(|cp| cp.abs() < 150.0));
    }
    assert!(
        update.engine.contains(&fake_path()),
        "the engine identity names the executable"
    );
    analysis.stop_analysis(true);
}

#[tokio::test]
async fn the_update_payload_is_the_engine_analysis_shape() {
    let log = log_path("analysis-shape");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": START, "lines": 1, "infinite": false, "clientId": 7 }),
            &fake_path(),
            sink,
        )
        .expect("a valid request");
    let done = wait_until(WAIT, "the completed update", || {
        find(&updates, |u| u.id == id && u.done)
    })
    .await;
    support::wire::assert_wire_steady("engine-analysis.completed", &done, &["context", "engine"]);
    let value = serde_json::to_value(&done).expect("json");
    for key in [
        "id", "fen", "depth", "lines", "done", "context", "clientId", "reason", "engine",
    ] {
        assert!(value.get(key).is_some(), "{key} is in the payload");
    }
    assert_eq!(value["reason"], json!("completed"));
    assert_eq!(value["clientId"], json!(7));
    assert!(value.get("error").is_none(), "no error on success");
    assert!(
        value.get("nps").is_none(),
        "nps is omitted until the engine reports it"
    );
}

#[tokio::test]
async fn castling_history_is_sent_as_the_root_position_with_chess960_rules() {
    let root = "r3k2r/ppp2ppp/8/8/8/8/PPP2PPP/R3K2R w KQkq - 0 1";
    let moves = vec!["e1h1".to_string()];
    let fen = replay_positions(root, &moves)
        .last()
        .expect("a position")
        .fen
        .clone();
    let log = log_path("analysis-castling");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": fen, "rootFen": root, "moves": moves, "lines": 3, "infinite": true }),
            &fake_path(),
            sink,
        )
        .expect("a valid request");
    let update = wait_until(WAIT, "three lines", || {
        find(&updates, |u| u.id == id && u.lines.len() == 3)
    })
    .await;
    assert_eq!(update.context, format!("{root}|e1h1"));
    analysis.stop_analysis(true);
    let lines = received(&log);
    assert!(
        lines
            .iter()
            .any(|line| line == &format!("position fen {root} moves e1h1"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name UCI_Chess960 value true")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name MultiPV value 3")
    );
}

#[tokio::test]
async fn a_superseded_request_is_interrupted_and_the_newer_one_wins() {
    let log = log_path("analysis-superseded");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let first = analysis
        .start_analysis(
            json!({ "fen": ITALIAN, "lines": 1, "infinite": true }),
            &fake_path(),
            Arc::clone(&sink),
        )
        .expect("first");
    // The first search is running in the engine when the second one supersedes it.
    wait_until(WAIT, "the first search to stream", || {
        find(&updates, |u| u.id == first && !u.lines.is_empty())
    })
    .await;
    let second = analysis
        .start_analysis(
            json!({ "fen": MATE_IN_ONE, "lines": 1, "infinite": true }),
            &fake_path(),
            sink,
        )
        .expect("second");
    let mate = wait_until(WAIT, "the mate line", || {
        find(&updates, |u| {
            u.id == second && u.lines.first().and_then(|l| l.mate) == Some(1.0)
        })
    })
    .await;
    assert_eq!(mate.lines[0].pv[0], "a1a8");
    let interrupted = wait_until(WAIT, "the superseded request's final update", || {
        find(&updates, |u| u.id == first && u.done)
    })
    .await;
    assert_eq!(
        interrupted.reason,
        Some(Reason::Interrupted),
        "superseded output is never completed"
    );
    let after_switch: Vec<AnalysisUpdate> = updates
        .lock()
        .expect("updates")
        .iter()
        .filter(|u| u.id == second)
        .cloned()
        .collect();
    assert!(after_switch.iter().all(|u| u.fen == MATE_IN_ONE));
    assert!(
        received(&log).iter().any(|line| line == "stop"),
        "the engine was told to stop"
    );
    analysis.stop_analysis(true);
}

#[tokio::test]
async fn a_stopped_analysis_ends_with_an_interrupted_final_update() {
    let log = log_path("analysis-stopped");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": ITALIAN, "lines": 3, "infinite": true }),
            &fake_path(),
            sink,
        )
        .expect("a valid request");
    wait_until(WAIT, "three lines", || {
        find(&updates, |u| u.id == id && u.lines.len() == 3)
    })
    .await;
    assert!(analysis.analysis_running());
    analysis.stop_analysis(false);
    let last = wait_until(WAIT, "the final update", || {
        find(&updates, |u| u.id == id && u.done)
    })
    .await;
    assert_eq!(last.reason, Some(Reason::Interrupted));
    assert_eq!(last.lines.len(), 3);
    wait_until(WAIT, "the analysis to end", || {
        (!analysis.analysis_running()).then_some(())
    })
    .await;
    analysis.stop_analysis(true);
}

/// `settled` (what closing waits for) ends only once the running analysis has sent its final
/// update.
#[tokio::test]
async fn settling_waits_for_the_running_analysis_to_end() {
    let log = log_path("analysis-settled");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": START, "lines": 1, "infinite": true }),
            &fake_path(),
            sink,
        )
        .expect("a valid request");
    wait_until(WAIT, "a line", || {
        find(&updates, |u| u.id == id && !u.lines.is_empty())
    })
    .await;
    let settling = tokio::spawn({
        let analysis = analysis.clone();
        async move { analysis.settled().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!settling.is_finished(), "settled while the analysis runs");
    analysis.stop_analysis(false);
    tokio::time::timeout(WAIT, settling)
        .await
        .expect("settled once stopped")
        .expect("the wait");
    assert!(find(&updates, |u| u.id == id && u.done).is_some());
    analysis.stop_analysis(true);
}

#[tokio::test]
async fn a_bounded_analysis_that_finishes_is_completed() {
    let log = log_path("analysis-bounded");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(json!({ "fen": START, "lines": 1 }), &fake_path(), sink)
        .expect("a valid request");
    let last = wait_until(WAIT, "the final update", || {
        find(&updates, |u| u.id == id && u.done)
    })
    .await;
    assert_eq!(last.reason, Some(Reason::Completed));
    assert!(
        received(&log)
            .iter()
            .any(|line| line == "go depth 26 movetime 60000")
    );
}

#[tokio::test]
async fn an_engine_that_cannot_be_found_fails_the_analysis_with_its_message() {
    let log = log_path("analysis-failed");
    let (analysis, _) = analysis_with("search", &log);
    let (updates, sink) = recorder();
    let id = analysis
        .start_analysis(
            json!({ "fen": START, "lines": 1 }),
            "/nonexistent/kchess-engine",
            sink,
        )
        .expect("a valid request");
    let failed = wait_until(WAIT, "the failed update", || {
        find(&updates, |u| u.id == id && u.done)
    })
    .await;
    assert_eq!(failed.reason, Some(Reason::Failed));
    assert_eq!(
        failed.error.as_deref(),
        Some("Stockfish could not be found. Choose an engine in Settings.")
    );
}

#[tokio::test]
async fn requests_that_are_not_positions_are_refused() {
    let log = log_path("analysis-refused");
    let (analysis, _) = analysis_with("search", &log);
    let (_updates, sink) = recorder();
    assert!(
        analysis
            .start_analysis(json!({ "fen": "startpos", "lines": 1 }), &fake_path(), sink)
            .is_err()
    );
    assert!(!analysis.analysis_running());
}

/// Two windows on one core (`docs/shared-core.md`): each connection has its own analysis and
/// its own updates. The engine budget goes to the newest request, so starting one interrupts the
/// other window's; a client's stop ends only its own, and a connection going away ends its own.
#[tokio::test]
async fn each_connection_has_its_own_analysis() {
    use kchess_core::connection::scope;
    let log = log_path("analysis-connections");
    let (analysis, _) = analysis_with("search", &log);
    let (first, first_sink) = recorder();
    let (second, second_sink) = recorder();
    let request = |fen: &str| json!({ "fen": fen, "lines": 1, "infinite": true });
    let first_id = scope(1, async {
        analysis.start_analysis(request(START), &fake_path(), first_sink)
    })
    .await
    .expect("a valid request");
    wait_until(WAIT, "the first window's lines", || {
        find(&first, |u| {
            u.id == first_id && u.fen == START && !u.lines.is_empty()
        })
    })
    .await;

    let second_id = scope(2, async {
        analysis.start_analysis(request(ITALIAN), &fake_path(), second_sink)
    })
    .await
    .expect("a valid request");
    wait_until(WAIT, "the first window's analysis interrupted", || {
        find(&first, |u| {
            u.id == first_id && u.reason == Some(Reason::Interrupted)
        })
    })
    .await;
    wait_until(WAIT, "the second window's lines", || {
        find(&second, |u| {
            u.id == second_id && u.fen == ITALIAN && !u.lines.is_empty()
        })
    })
    .await;
    assert!(
        find(&first, |u| u.fen == ITALIAN).is_none(),
        "updates stay with their window"
    );

    // The first window's stop leaves the second window's analysis running.
    scope(1, async { analysis.stop_mine(false) }).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(analysis.analysis_running());
    assert!(find(&second, |u| u.done).is_none());

    analysis.forget(2);
    wait_until(WAIT, "the second analysis interrupted", || {
        find(&second, |u| {
            u.id == second_id && u.reason == Some(Reason::Interrupted)
        })
    })
    .await;
    wait_until(WAIT, "no analysis left", || {
        (!analysis.analysis_running()).then_some(())
    })
    .await;
}

/// A Crazyhouse position goes to the variant engine, whatever engine is chosen, with Fairy-
/// Stockfish's name for the variant and the FEN with its pockets as it is.
#[tokio::test]
async fn a_variant_position_goes_to_the_variant_engine_with_its_variant() {
    let dir = tempfile::tempdir().expect("a folder");
    let variant = std::path::PathBuf::from(support::engine_copy(dir.path(), "search"));
    let log = log_path("analysis-variant");
    let host = support::host();
    let mut locations = support::locations();
    locations.variant = variant.clone();
    let mut ctx = kchess_core::engine::EngineContext::new(
        host.clone(),
        Arc::new(support::hub(&host)),
        scheduler(4, false),
        locations,
    );
    let started: Arc<Mutex<Vec<std::path::PathBuf>>> = Arc::default();
    let seen = Arc::clone(&started);
    let fake_log = log.clone();
    ctx.command = Arc::new(move |status| {
        seen.lock().expect("paths").push(status.path.clone().into());
        support::fake("search", Some(&fake_log))
    });
    let analysis = Analysis::new(ctx);
    let (updates, sink) = recorder();
    // 1. e4 d5 2. exd5 Qxd5: a pawn each in hand.
    let fen = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
    let id = analysis
        .start_analysis(
            json!({ "fen": fen, "lines": 1, "infinite": true, "variant": "crazyhouse" }),
            &fake_path(),
            sink,
        )
        .expect("a valid Crazyhouse request");
    wait_until(WAIT, "the variant engine to be asked", || {
        received(&log)
            .iter()
            .any(|line| line.starts_with("position fen"))
            .then_some(())
    })
    .await;
    analysis.stop_analysis(true);
    let _ = find(&updates, |u| u.id == id);
    assert_eq!(*started.lock().expect("paths"), vec![variant]);
    let lines = received(&log);
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name UCI_Variant value crazyhouse"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == &format!("position fen {fen}"))
    );
    assert!(!lines.iter().any(|line| line.contains("UCI_Chess960")));
}

/// A drop that is no legal move there, or a variant KChess does not know, is refused.
#[tokio::test]
async fn a_variant_request_is_checked_under_its_rules() {
    let log = log_path("analysis-variant-invalid");
    let (analysis, _) = analysis_with("search", &log);
    let (_, sink) = recorder();
    let root = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
    for request in [
        json!({ "fen": root, "rootFen": root, "moves": ["P@e4"], "lines": 1, "variant": "crazyhouse" }),
        json!({ "fen": root, "lines": 1, "variant": "shogi" }),
    ] {
        assert!(
            analysis
                .start_analysis(request.clone(), &fake_path(), Arc::clone(&sink))
                .is_err(),
            "{request}"
        );
    }
}
