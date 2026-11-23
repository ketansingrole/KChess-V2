//! Engine options. The fake engine logs every command it
//! receives, so the `setoption` lines and `isready` round-trips are read from that log.

use crate::support;

use std::time::Duration;

use kchess_core::engine::uci::{DEFAULT_DEADLINE, UciController};
use tempfile::TempDir;

/// The handshake deadline for an engine that answers: the core's own (`DEFAULT_DEADLINE`). None of
/// these tests is about the deadline, which counts the process's start too: a loaded Windows runner
/// took over 2 s to start the fake engine (CI 2026-11-18), past the 2 s these tests once used.
const HANDSHAKE: Duration = DEFAULT_DEADLINE;

fn setoptions(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.starts_with("setoption"))
        .cloned()
        .collect()
}

fn syncs(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|line| line.as_str() == "isready")
        .count()
}

#[tokio::test]
async fn sends_each_option_once_then_skips_redundant_isready_round_trips() {
    let dir = TempDir::new().expect("temp dir");
    let log = dir.path().join("commands.log");
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("handshake", Some(&log)), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");

    // Only what follows the handshake is measured.
    let base_lines = support::received(&log);
    let base = syncs(&base_lines);
    let since = |start: usize| support::received(&log)[start..].to_vec();
    let mark = base_lines.len();

    // Review's per-position pattern: the same Threads 140 times for one game.
    engine
        .ensure_options(&[("Threads", "1")])
        .await
        .expect("options");
    let after = since(mark);
    assert_eq!(setoptions(&after), vec!["setoption name Threads value 1"]);
    assert_eq!(syncs(&after), 1);

    let mark = support::received(&log).len();
    for _ in 0..20 {
        engine
            .ensure_options(&[("Threads", "1")])
            .await
            .expect("options");
    }
    let after = since(mark);
    assert!(setoptions(&after).is_empty());
    assert_eq!(syncs(&after), 0);

    // Analysis scrubbing with identical options across positions.
    let analysis = [
        ("Threads", "4"),
        ("Hash", "128"),
        ("MultiPV", "3"),
        ("UCI_Chess960", "true"),
    ];
    let mark = support::received(&log).len();
    engine.ensure_options(&analysis).await.expect("options");
    assert_eq!(syncs(&since(mark)), 1);

    let mark = support::received(&log).len();
    for _ in 0..10 {
        engine.ensure_options(&analysis).await.expect("options");
    }
    let after = since(mark);
    assert!(setoptions(&after).is_empty());
    assert_eq!(syncs(&after), 0);

    // Only the changed option is resent (battery saver 4 -> 2 threads).
    let mark = support::received(&log).len();
    engine
        .ensure_options(&[
            ("Threads", "2"),
            ("Hash", "128"),
            ("MultiPV", "3"),
            ("UCI_Chess960", "true"),
        ])
        .await
        .expect("options");
    let after = since(mark);
    assert_eq!(setoptions(&after), vec!["setoption name Threads value 2"]);
    assert_eq!(syncs(&after), 1);
    assert!(syncs(&support::received(&log)) > base);

    engine.close();
    tokio::time::timeout(Duration::from_secs(5), engine.closed())
        .await
        .expect("closed engine exits");
}

#[tokio::test]
async fn forgets_confirmed_options_when_the_engine_is_closed() {
    let dir = TempDir::new().expect("temp dir");
    let log = dir.path().join("commands.log");
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("handshake", Some(&log)), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");
    let mark = support::received(&log).len();
    engine
        .ensure_options(&[("Threads", "1")])
        .await
        .expect("options");
    assert_eq!(syncs(&since_of(&log, mark)), 1);
    engine.close();
    engine.closed().await;

    // A replacement process starts empty: the same options are sent again.
    let next_log = dir.path().join("next-commands.log");
    let next = UciController::start(&hub, support::fake("handshake", Some(&next_log)), HANDSHAKE)
        .expect("spawn the replacement");
    next.ready().await.expect("handshake");
    let mark = support::received(&next_log).len();
    next.ensure_options(&[("Threads", "1")])
        .await
        .expect("options");
    let after = since_of(&next_log, mark);
    assert_eq!(setoptions(&after), vec!["setoption name Threads value 1"]);
    assert_eq!(syncs(&after), 1);
    next.close();
}

/// A change dropped while it waits for `readyok` (its client left mid-search) has reached the
/// engine, so the value it replaced is no longer known: the next search sends it again. The cache
/// once kept the old value, and the next weak search played at the dropped one's full strength.
#[tokio::test]
async fn a_change_dropped_before_its_acknowledgement_is_sent_again() {
    let dir = TempDir::new().expect("temp dir");
    let log = dir.path().join("commands.log");
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("handshake", Some(&log)), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");
    engine
        .ensure_options(&[("Skill Level", "5")])
        .await
        .expect("options");
    let mark = support::received(&log).len();

    // On this one-thread runtime the engine's answer cannot arrive during the single poll.
    let mut stronger = Box::pin(engine.ensure_options(&[("Skill Level", "20")]));
    tokio::select! {
        biased;
        _ = &mut stronger => panic!("acknowledged within one poll"),
        () = std::future::ready(()) => {}
    }
    drop(stronger);

    engine
        .ensure_options(&[("Skill Level", "5")])
        .await
        .expect("options");
    assert_eq!(
        setoptions(&since_of(&log, mark)),
        [
            "setoption name Skill Level value 20",
            "setoption name Skill Level value 5"
        ]
    );
    engine.close();
}

/// An abandoned command's answer is still on the way, and it answered the next command waiting
/// for the same line: the `readyok` of a dropped `isready` confirmed the next options change before
/// the engine had read it (main's CI, 2026-11-23). The engine answers the abandoned `isready`
/// before the `uci` sent after it, so the `uci` waiter, which would take either, must get `uciok`.
#[tokio::test]
async fn an_abandoned_commands_answer_is_not_given_to_the_next() {
    let host = support::host();
    let hub = support::hub(&host);
    let engine = UciController::start(&hub, support::fake("handshake", None), HANDSHAKE)
        .expect("spawn the fake engine");
    engine.ready().await.expect("handshake");

    // On this one-thread runtime the engine's answer cannot arrive during the single poll.
    let mut abandoned = Box::pin(engine.command("isready", |line| line == "readyok", None, None));
    tokio::select! {
        biased;
        _ = &mut abandoned => panic!("answered within one poll"),
        () = std::future::ready(()) => {}
    }
    drop(abandoned);

    let answer = engine
        .command(
            "uci",
            |line| line == "uciok" || line == "readyok",
            None,
            None,
        )
        .await
        .expect("uci");
    assert_eq!(answer, "uciok");
    engine.close();
}

fn since_of(log: &std::path::Path, mark: usize) -> Vec<String> {
    support::received(log)[mark..].to_vec()
}
