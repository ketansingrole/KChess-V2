//! Port of `core/tests/unit/engine-options.test.ts`. The fake engine logs every command it
//! receives, so the `setoption` lines and `isready` round-trips are read from that log.

mod support;

use std::time::Duration;

use kchess_core::engine::uci::UciController;
use tempfile::TempDir;

const HANDSHAKE: Duration = Duration::from_secs(2);

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

fn since_of(log: &std::path::Path, mark: usize) -> Vec<String> {
    support::received(log)[mark..].to_vec()
}
