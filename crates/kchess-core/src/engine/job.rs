//! What the engine services' jobs share: their place in line, the wait for settings that a stop
//! also ends, the engine's rules for the game's variant, the position command, and how a job
//! that went wrong is told apart and logged.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::uci::{error_summary, search_cancelled};
use crate::host::{Host, Level};
use crate::tasks::aborted as signal_aborted;
use kchess_contracts::error::{CoreError, Result};

/// A service's jobs in the order they were asked for: each starts once the one before it ended.
///
/// Places take numbered tickets. A place dropped once its turn has come passes the turn to the
/// next place still in line; one dropped while it waits only leaves the line, so the place after
/// it still waits for the job running now.
#[derive(Default)]
pub(crate) struct Line {
    queue: Arc<watch::Sender<Queue>>,
    issued: u64,
}

/// Whose turn it is, and the places that left the line before their turn came.
#[derive(Default)]
struct Queue {
    serving: u64,
    left: BTreeSet<u64>,
}

impl Line {
    /// The next place in line. Dropping it ends its job's turn, or its wait for one.
    pub fn join(&mut self) -> Place {
        let ticket = self.issued;
        self.issued += 1;
        Place {
            queue: Arc::clone(&self.queue),
            ticket,
        }
    }
}

/// One job's place in a `Line`.
pub(crate) struct Place {
    queue: Arc<watch::Sender<Queue>>,
    ticket: u64,
}

impl Place {
    /// Waits until every job before has ended, however it ended.
    pub async fn turn(&mut self) {
        let ticket = self.ticket;
        // The sender lives in this place, so the wait ends only when the turn comes.
        let _ = self
            .queue
            .subscribe()
            .wait_for(|queue| queue.serving == ticket)
            .await;
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let ticket = self.ticket;
        self.queue.send_modify(|queue| {
            if queue.serving != ticket {
                queue.left.insert(ticket);
                return;
            }
            queue.serving += 1;
            while queue.left.remove(&queue.serving) {
                queue.serving += 1;
            }
        });
    }
}

/// The engine the settings name, unless the job is stopped first: a stop must also end a job
/// still waiting for its settings.
pub(crate) async fn configured(
    token: &CancellationToken,
    configured: impl Future<Output = Result<String>>,
) -> Result<String> {
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(signal_aborted()),
        result = configured => result,
    }
}

/// Stops when `token` was cancelled, and when `epoch` is no longer `current` (a stop that
/// superseded every job).
pub(crate) fn still_wanted(token: &CancellationToken, epoch: u64, current: u64) -> Result<()> {
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    if epoch != current {
        return Err(search_cancelled());
    }
    Ok(())
}

/// The option that sets the rules: Fairy-Stockfish's variant, or else Chess960, since the moves
/// write castling king-takes-rook and an engine in standard mode reads that as an illegal move.
pub(crate) fn rules_option(fairy: Option<&'static str>) -> (&'static str, &'static str) {
    match fairy {
        Some(variant) => ("UCI_Variant", variant),
        None => ("UCI_Chess960", "true"),
    }
}

/// The `position` command: the start (`startpos` without a FEN) and the moves played from it.
pub(crate) fn position_command(fen: Option<&str>, moves: &[String]) -> String {
    let mut position = match fen {
        Some(fen) => format!("position fen {fen}"),
        None => "position startpos".to_string(),
    };
    if !moves.is_empty() {
        position.push_str(" moves ");
        position.push_str(&moves.join(" "));
    }
    position
}

/// Whether a job that failed was interrupted (stopped, superseded, aborted) rather than failed,
/// logged as such: an interruption at Debug, a failure at Warn.
pub(crate) fn interrupted(
    host: &dyn Host,
    area: &str,
    (stopped, failed): (&str, &str),
    context: &str,
    cause: &CoreError,
    token: &CancellationToken,
) -> bool {
    let interrupted = token.is_cancelled() || cause.aborted;
    let (level, label) = if interrupted {
        (Level::Debug, stopped)
    } else {
        (Level::Warn, failed)
    };
    host.log(
        level,
        area,
        &format!("{label}: {context} {}", error_summary(cause)),
    );
    interrupted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// How long a turn that should come may take: a stuck line fails here, named, rather than
    /// hanging the run.
    const UNTIL_LIMIT: Duration = Duration::from_secs(5);

    async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
        tokio::time::timeout(UNTIL_LIMIT, wait)
            .await
            .unwrap_or_else(|_| panic!("{what} did not come within {UNTIL_LIMIT:?}"))
    }

    #[test]
    fn the_position_names_its_start_and_moves() {
        assert_eq!(position_command(None, &[]), "position startpos");
        assert_eq!(
            position_command(
                Some("8/8/8/8/8/8/8/K6k w - - 0 1"),
                &["a1b1".into(), "h1g1".into()]
            ),
            "position fen 8/8/8/8/8/8/8/K6k w - - 0 1 moves a1b1 h1g1"
        );
    }

    #[test]
    fn a_variant_sets_its_rules_and_chess_plays_chess960() {
        assert_eq!(
            rules_option(Some("crazyhouse")),
            ("UCI_Variant", "crazyhouse")
        );
        assert_eq!(rules_option(None), ("UCI_Chess960", "true"));
    }

    #[tokio::test]
    async fn a_job_waits_for_the_one_before() {
        let mut line = Line::default();
        let first = line.join();
        let mut second = line.join();
        let waiting = tokio::spawn(async move { second.turn().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(first);
        within("the second job's turn", waiting).await.unwrap();
    }

    /// A place dropped while it waits (a cancelled `settled()`, a search stopped in line) must
    /// not let the place after it start while the job before it still runs.
    #[tokio::test]
    async fn a_place_dropped_in_line_does_not_let_the_next_start_early() {
        let mut line = Line::default();
        let mut running = line.join();
        within("the first job's turn", running.turn()).await;
        let waiting = line.join();
        let mut last = line.join();
        let after = tokio::spawn(async move { last.turn().await });
        drop(waiting);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            !after.is_finished(),
            "started while the running job had not ended"
        );
        drop(running);
        within("the last job's turn", after).await.unwrap();
    }

    /// A place dropped once its own turn has come passes the turn on, also past places dropped
    /// earlier.
    #[tokio::test]
    async fn the_turn_passes_over_every_place_dropped_in_line() {
        let mut line = Line::default();
        let running = line.join();
        let (gone, also_gone) = (line.join(), line.join());
        let mut last = line.join();
        drop(also_gone);
        drop(gone);
        drop(running);
        within("the turn past the dropped places", last.turn()).await;
        let mut next = line.join();
        let waits = tokio::spawn(async move { next.turn().await });
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(!waits.is_finished());
        drop(last);
        within("the next job's turn", waits).await.unwrap();
    }

    #[tokio::test]
    async fn a_stop_ends_the_wait_for_settings() {
        let token = CancellationToken::new();
        token.cancel();
        let error = configured(&token, std::future::pending())
            .await
            .unwrap_err();
        assert!(error.aborted);
        let open = CancellationToken::new();
        assert_eq!(
            configured(&open, async { Ok("sf".to_string()) })
                .await
                .unwrap(),
            "sf"
        );
    }

    #[test]
    fn a_stop_or_a_newer_epoch_ends_a_job() {
        let (open, stopped) = (CancellationToken::new(), CancellationToken::new());
        stopped.cancel();
        assert!(still_wanted(&open, 3, 3).is_ok());
        assert!(still_wanted(&stopped, 3, 3).unwrap_err().aborted);
        assert_eq!(still_wanted(&open, 2, 3).unwrap_err(), search_cancelled());
    }

    #[test]
    fn a_stop_or_an_abort_is_an_interruption_and_anything_else_a_failure() {
        let (open, stopped) = (CancellationToken::new(), CancellationToken::new());
        stopped.cancel();
        let failure = CoreError::new("broken");
        for (token, cause, expected, level) in [
            (&stopped, &failure, true, "debug"),
            (&open, &signal_aborted(), true, "debug"),
            (&open, &failure, false, "warn"),
        ] {
            let host = crate::test_host::TestHost::new();
            let interrupted = interrupted(
                &*host,
                "engine",
                ("Stopped", "Failed"),
                "review",
                cause,
                token,
            );
            assert_eq!(interrupted, expected, "{}", cause.message);
            let label = if expected { "Stopped" } else { "Failed" };
            assert!(
                host.logs()[0].starts_with(&format!("{level} [engine] {label}: review")),
                "{:?}",
                host.logs()
            );
        }
    }
}
