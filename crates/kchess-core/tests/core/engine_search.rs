//! `bestMove` and `humanMove` through the scripted engine: level options, the random-move path,
//! the lines a human move draws from, invalid input, superseded
//! searches reaching the engine, the warm process, and the computer-playing bookkeeping.

use crate::support;

use std::future::Future;
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
async fn castling_reaches_the_engine_in_the_notation_of_its_mode() {
    // The rules record castling king-takes-rook (e1h1); Stockfish rejects that with
    // `UCI_Chess960` off, never answers, and the computer game times out.
    let line = json!([
        "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1h1", "f8c5"
    ]);
    let log = log_path("castling-standard");
    let (search, _) = search_with("search", false, &log);
    search
        .best_move(
            line.clone(),
            "max",
            &fake_path(),
            json!({ "movetime": 100 }),
        )
        .await
        .expect("a move");
    assert!(
        received(&log)
            .iter()
            .any(|line| line == "position startpos moves e2e4 e7e5 g1f3 b8c6 f1c4 g8f6 e1g1 f8c5")
    );
    let log = log_path("castling-chess960");
    let (search, _) = search_with("search", false, &log);
    let mut typed = line;
    typed[6] = json!("e1g1");
    search
        .best_move(
            typed,
            "max",
            &fake_path(),
            json!({ "movetime": 100, "chess960": true }),
        )
        .await
        .expect("a move");
    assert!(
        received(&log)
            .iter()
            .any(|line| line == "position startpos moves e2e4 e7e5 g1f3 b8c6 f1c4 g8f6 e1h1 f8c5")
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
    assert!(superseded.aborted, "a superseded search is a cancellation");
    assert!(
        received(&log).iter().any(|line| line == "stop"),
        "the engine was told to stop the superseded search"
    );
}

/// Two windows on the shared core (`docs/shared-core.md`) each play the computer: one window's
/// move waits its turn instead of superseding the other's, and a stop reaches only its own.
#[tokio::test]
async fn windows_play_their_own_computer_games_side_by_side() {
    let log = log_path("windows");
    let (search, _) = search_with("search", false, &log);
    // A window's search, asked for in its scope: it has superseded the window's last search and
    // taken its turn by the time this returns.
    let in_window = |connection: u64, movetime: u64| {
        let search = search.clone();
        let mut asked = Box::pin(kchess_core::connection::scope(connection, async move {
            search
                .best_move(
                    json!(["e2e4"]),
                    "max",
                    &fake_path(),
                    json!({ "movetime": movetime }),
                )
                .await
        }));
        let mut first_poll = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(
            asked.as_mut().poll(&mut first_poll).is_pending(),
            "the search is asked for"
        );
        tokio::spawn(asked)
    };
    let first = in_window(2, 400);
    wait_until(
        Duration::from_secs(10),
        "the first window's search to reach the engine",
        || {
            received(&log)
                .iter()
                .any(|line| line == "go movetime 400")
                .then_some(())
        },
    )
    .await;
    let second = in_window(3, 50);
    // The second window takes its move back: only its own search stops.
    search.stop_connection(3);
    assert_eq!(
        first
            .await
            .expect("joined")
            .expect("the first window's move"),
        "e2e4"
    );
    let stopped = second.await.expect("joined").expect_err("stopped");
    assert_eq!(stopped.message, SUPERSEDED);
    assert!(stopped.aborted);
    assert_eq!(
        in_window(3, 50)
            .await
            .expect("joined")
            .expect("the second window's next move"),
        "e2e4"
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

/// `human_move` with the scripted `human` engine, whose five lines are two good moves, a fair
/// one, a pawn's mistake and one that lets the opponent mate, and a random source fixed at `roll`.
fn human_search(roll: f64, log: &Path) -> Search {
    let host = host();
    let ctx = context(&host, scheduler(4, false), "human", log);
    let random: RandomSource = Arc::new(move || roll);
    Search::with_random(ctx, random)
}

/// One search of five lines at full strength, to the rating's depth and for the time asked, and
/// the move drawn among them: the lowest roll plays the best, the highest the worst allowed, and
/// from 1800 that is never the move that lets the opponent mate.
#[tokio::test]
async fn human_move_searches_several_lines_and_plays_one_as_people_would() {
    let cases = [
        (0.0, 1000, "e2e4"),
        (0.0, 2200, "e2e4"),
        (0.9999, 1000, "f2f3"),
        (0.9999, 2000, "a2a3"),
    ];
    for (roll, rating, expected) in cases {
        let log = log_path("human-move");
        let search = human_search(roll, &log);
        let mv = search
            .human_move(
                json!(["e2e4", "e7e5"]),
                rating,
                &fake_path(),
                json!({ "movetime": 50 }),
            )
            .await
            .expect("a move");
        assert_eq!(mv, expected, "roll {roll} at {rating}");
        let lines = received(&log);
        for sent in [
            "setoption name MultiPV value 5",
            "setoption name UCI_LimitStrength value false",
            "setoption name Skill Level value 20",
            "position startpos moves e2e4 e7e5",
        ] {
            assert!(lines.iter().any(|line| line == sent), "{sent}: {lines:?}");
        }
        let depth = kchess_domain::games::human::depth(rating);
        let go = format!("go depth {depth} movetime 50");
        assert!(lines.contains(&go), "{go}: {lines:?}");
    }
}

/// The same warm engine plays a level's move after a human one: it searches one line again.
#[tokio::test]
async fn a_level_move_after_a_human_one_searches_one_line() {
    let log = log_path("human-then-level");
    let search = human_search(0.0, &log);
    search
        .human_move(json!([]), 1500, &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a human move");
    search
        .best_move(json!([]), "max", &fake_path(), json!({ "movetime": 50 }))
        .await
        .expect("a level's move");
    let lines = received(&log);
    let multipv: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("setoption name MultiPV"))
        .collect();
    assert_eq!(
        multipv,
        [
            "setoption name MultiPV value 5",
            "setoption name MultiPV value 1"
        ]
    );
    assert_eq!(lines.iter().filter(|line| *line == "uci").count(), 1);
}

/// A human move is stopped as a level's is: the stop reaches the engine's search, the move
/// rejects as superseded, and the engine is free for the next one.
#[tokio::test]
async fn stopping_a_human_move_stops_its_search() {
    let log = log_path("human-stop");
    let search = human_search(0.0, &log);
    let running = tokio::spawn(search.human_move(
        json!(["e2e4"]),
        1600,
        &fake_path(),
        json!({ "movetime": 5000 }),
    ));
    wait_until(
        Duration::from_secs(10),
        "the human search to reach the engine",
        || {
            received(&log)
                .iter()
                .any(|line| line.ends_with("movetime 5000"))
                .then_some(())
        },
    )
    .await;
    search.stop_connection(kchess_core::connection::current_or_app());
    let stopped = running
        .await
        .expect("the task finishes")
        .expect_err("stopped");
    assert_eq!(stopped.message, SUPERSEDED);
    assert!(stopped.aborted, "a stopped search is a cancellation");
    assert!(
        received(&log).iter().any(|line| line == "stop"),
        "the engine was told to stop the search"
    );
    let next = search
        .human_move(
            json!(["e2e4"]),
            1600,
            &fake_path(),
            json!({ "movetime": 50 }),
        )
        .await
        .expect("the next move");
    assert_eq!(next, "e2e4");
}
