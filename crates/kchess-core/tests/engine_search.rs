//! `bestMove` through the scripted engine (`core/tests/unit/engine-levels.test.ts`, and the search
//! parts of `engine.ts`): level options, the random-move path, invalid input, superseded
//! searches reaching the engine, the warm process, and the computer-playing bookkeeping.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kchess_core::engine::search::{RandomSource, SUPERSEDED, Search};
use serde_json::json;
use support::{context, fake_path, host, log_path, received, scheduler, wait_until};

fn search_with(mode: &str, battery: bool, log: &Path) -> (Search, Arc<support::Recording>) {
    let host = host();
    let ctx = context(&host, scheduler(4, battery), mode, log);
    (Search::new(ctx), host)
}

#[tokio::test]
async fn best_move_sends_the_level_options_and_the_position() {
    let log = log_path("best-move");
    let (search, _) = search_with("search", false, &log);
    // Beginner plays random moves some of the time, so its move is not the engine's.
    for level in ["casual", "max"] {
        let mv = search
            .best_move(
                json!(["e2e4"]),
                level,
                &fake_path(),
                json!({ "movetime": 100 }),
            )
            .await
            .expect("a move");
        // The scripted engine answers with the first move of its principal variation.
        assert_eq!(mv, "e2e4", "{level}");
    }
    let lines = received(&log);
    assert!(
        lines
            .iter()
            .any(|line| line == "position startpos moves e2e4")
    );
    assert!(lines.iter().any(|line| line == "go movetime 100"));
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name Threads value 4")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name Hash value 64")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name UCI_Chess960 value false")
    );
    // Casual is rated (UCI_Elo), so strength is limited; full-strength levels are not.
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name UCI_LimitStrength value true")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name UCI_Elo value 1350")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setoption name Skill Level value 20")
    );
}

#[tokio::test]
async fn the_random_move_path_picks_from_the_legal_moves() {
    // The rook on a1 checks the king on h1; its only escape is h2 (the scripted `go perft 1`).
    let log = log_path("random");
    let host = host();
    let ctx = context(&host, scheduler(2, false), "search", &log);
    let random: RandomSource = Arc::new(|| 0.0);
    let search = Search::with_random(ctx, random);
    let mv = search
        .best_move(
            json!([]),
            "beginner",
            &fake_path(),
            json!({ "fen": "7k/8/8/8/8/8/6P1/r6K w - - 0 1" }),
        )
        .await
        .expect("a random move");
    assert_eq!(mv, "h1h2");
    assert!(received(&log).iter().any(|line| line == "go perft 1"));
}

#[tokio::test]
async fn invalid_moves_are_refused_before_any_engine_starts() {
    let log = log_path("invalid");
    let (search, _) = search_with("search", false, &log);
    let error = search
        .best_move(json!(["e2e5"]), "max", &fake_path(), json!({}))
        .await
        .expect_err("an illegal move");
    assert_eq!(error.message, "Invalid computer position or move history.");
    assert!(received(&log).is_empty(), "no engine was started");
}

#[tokio::test]
async fn a_new_search_supersedes_the_running_one_and_stops_its_engine() {
    let log = log_path("superseded");
    let (search, _) = search_with("search", false, &log);
    let first = tokio::spawn(search.best_move(
        json!(["e2e4"]),
        "max",
        &fake_path(),
        json!({ "movetime": 5000 }),
    ));
    wait_until(
        Duration::from_secs(10),
        "the first search to reach the engine",
        || {
            received(&log)
                .iter()
                .any(|line| line == "go movetime 5000")
                .then_some(())
        },
    )
    .await;
    let second = search
        .best_move(
            json!(["e2e4"]),
            "max",
            &fake_path(),
            json!({ "movetime": 50 }),
        )
        .await
        .expect("the newer search completes");
    assert_eq!(second, "e2e4");
    let superseded = first
        .await
        .expect("the task finishes")
        .expect_err("superseded");
    assert_eq!(superseded.message, SUPERSEDED);
    assert!(
        received(&log).iter().any(|line| line == "stop"),
        "the engine was told to stop the superseded search"
    );
}

#[tokio::test]
async fn consecutive_moves_reuse_the_warm_engine_and_send_only_changed_options() {
    let log = log_path("warm");
    let (search, _) = search_with("search", false, &log);
    for _ in 0..2 {
        search
            .best_move(
                json!(["e2e4"]),
                "max",
                &fake_path(),
                json!({ "movetime": 50 }),
            )
            .await
            .expect("a move");
    }
    let lines = received(&log);
    assert_eq!(
        lines.iter().filter(|line| *line == "uci").count(),
        1,
        "one process"
    );
    assert_eq!(
        lines.iter().filter(|line| *line == "ucinewgame").count(),
        2,
        "each game is reset"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("setoption name Hash"))
            .count(),
        1,
        "unchanged options are not sent again"
    );
}

#[tokio::test]
async fn closing_the_engine_makes_the_next_move_start_a_new_process() {
    let log = log_path("close");
    let (search, _) = search_with("search", false, &log);
    search
        .best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a move");
    search.stop_engine(true);
    search
        .best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a move after closing");
    let starts = received(&log).iter().filter(|line| *line == "uci").count();
    assert_eq!(starts, 2);
}

#[tokio::test]
async fn computer_playing_follows_the_search_until_reset() {
    let log = log_path("computer-playing");
    let (search, _) = search_with("search", false, &log);
    assert!(!search.computer_playing(60_000), "no move yet");
    search
        .best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a move");
    assert!(search.computer_playing(60_000), "a move was just computed");
    search.reset_engine();
    assert!(
        !search.computer_playing(60_000),
        "reset forgets the last move"
    );
}

#[tokio::test]
async fn trusted_engine_paths_are_forgotten_on_reset() {
    let log = log_path("trust");
    let (search, _) = search_with("search", false, &log);
    let path = Path::new("/opt/engines/stockfish");
    assert!(!search.is_trusted_engine_path(path));
    search.trust_engine_path(path);
    assert!(search.is_trusted_engine_path(path));
    search.reset_engine();
    assert!(!search.is_trusted_engine_path(path));
}

#[tokio::test]
async fn a_missing_engine_says_how_to_choose_one() {
    let log = log_path("missing");
    let (search, _) = search_with("search", false, &log);
    let error = search
        .best_move(json!([]), "max", "/nonexistent/kchess-engine", json!({}))
        .await
        .expect_err("no engine");
    assert_eq!(
        error.message,
        "Stockfish could not be found. Choose an engine in Settings."
    );
}

#[tokio::test]
async fn battery_power_limits_the_threads_a_move_may_use() {
    let log = log_path("battery");
    let (search, _) = search_with("search", true, &log);
    search
        .best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a move");
    assert!(
        received(&log)
            .iter()
            .any(|line| line == "setoption name Threads value 2")
    );
    assert_eq!(search.engine_threads(), 2);
}
