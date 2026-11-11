//! Computer moves. One warm engine process serves consecutive moves; a new search supersedes
//! the one running (`SUPERSEDED`), and the process goes after two idle minutes.
//!
//! `best_move` takes its place in line and supersedes the running search as soon as it is
//! called, before its future is polled; the search itself runs inside the returned future.
//! Dropping that future ends the search.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::{BoxFuture, ready};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::scheduler::Priority;
use super::slot::EngineSlot;
use super::status::{EngineTrust, computer_playing, fairy_variant};
use super::uci::{UciController, error_summary, search_cancelled, truncate_for_log};
use super::{EngineContext, random_unit};
use crate::host::Level;
use crate::tasks::{aborted as signal_aborted, lock};
use crate::time::now_ms;
use kchess_contracts::check::{Moves, check};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::BestMoveOptions;
use kchess_domain::js::number_to_string as js_number;
use kchess_domain::misc::value::J;

/// The message a superseded search rejects with.
pub const SUPERSEDED: &str = "Engine search superseded.";
/// The start position when a request gives none.
const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// How long an idle warm engine is kept.
const WARM_IDLE: Duration = Duration::from_secs(120);

/// The random source for the weakest levels' random moves; tests replace it.
pub type RandomSource = Arc<dyn Fn() -> f64 + Send + Sync>;

/// `EngineLevelInfo` for the level asked for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LevelProfile {
    time: f64,
    #[serde(default)]
    uci_elo: Option<f64>,
    #[serde(default)]
    skill: Option<f64>,
    #[serde(default)]
    random_move: Option<f64>,
}

/// A validated request, ready to search.
struct Request {
    moves: Vec<String>,
    fen: Option<String>,
    movetime: Option<f64>,
    chess960: bool,
    /// Fairy-Stockfish's name of the variant (`UCI_Variant`), for a variant game.
    variant: Option<&'static str>,
    level: String,
    configured: String,
}

/// The search service's state.
#[derive(Default)]
struct State {
    /// Bumped by every `stop_engine`; a search that sees a newer value was superseded.
    generation: u64,
    /// The searches in progress or in line, by the connection that asked (each window plays its
    /// own computer game), with the ticket that identifies each.
    active: HashMap<u64, (u64, CancellationToken)>,
    ticket: u64,
    last_move_at: i64,
    /// Finishes when the search queued last has finished.
    serial: Option<oneshot::Receiver<()>>,
}

struct Shared {
    ctx: EngineContext,
    random: RandomSource,
    trust: EngineTrust,
    state: Mutex<State>,
    warm: Arc<EngineSlot>,
}

/// The computer's search. Cloning shares the same engine and state.
#[derive(Clone)]
pub struct Search {
    shared: Arc<Shared>,
}

/// Runs when a search ends, however it ends, before the next search in line may start.
struct Finish {
    shared: Arc<Shared>,
    connection: u64,
    ticket: u64,
}

impl Drop for Finish {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if state
            .active
            .get(&self.connection)
            .is_some_and(|(ticket, _)| *ticket == self.ticket)
        {
            state.active.remove(&self.connection);
        }
        if state.active.is_empty() {
            self.shared.warm.release();
        }
    }
}

/// The receiving end of the search queue, and the sender that releases the next search.
type Ticket = (
    u64,
    u64,
    CancellationToken,
    Option<oneshot::Receiver<()>>,
    oneshot::Sender<()>,
);

impl Search {
    pub fn new(ctx: EngineContext) -> Search {
        Search::with_random(ctx, Arc::new(random_unit))
    }

    /// `new` with the random source for the weakest levels' random moves replaced.
    pub fn with_random(ctx: EngineContext, random: RandomSource) -> Search {
        Search {
            shared: Arc::new(Shared {
                warm: EngineSlot::new(ctx.clone(), WARM_IDLE),
                ctx,
                random,
                trust: EngineTrust::default(),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// The engine budget the scheduler grants searches.
    pub fn engine_threads(&self) -> usize {
        self.shared.ctx.scheduler.search_threads()
    }

    /// An engine path the frontends may persist.
    pub fn trust_engine_path(&self, path: &Path) {
        self.shared.trust.trust(path);
    }

    pub fn is_trusted_engine_path(&self, path: &Path) -> bool {
        self.shared
            .trust
            .is_trusted(path, &self.shared.ctx.locations.managed_dir)
    }

    /// Supersedes every search (settings, a live game, closing); with `close`, also ends the warm
    /// engine and its idle timer.
    pub fn stop_engine(&self, close: bool) {
        let mut state = lock(&self.shared.state);
        state.generation += 1;
        for (_, token) in state.active.values() {
            token.cancel();
        }
        if close {
            self.shared.warm.close();
        }
    }

    /// Stops the search `connection` asked for (a window's takeback or teardown); other windows'
    /// computer games carry on.
    pub fn stop_connection(&self, connection: u64) {
        if let Some((_, token)) = lock(&self.shared.state).active.get(&connection) {
            token.cancel();
        }
    }

    /// Stops and closes the engine, forgets trusted paths and the last move.
    pub fn reset_engine(&self) {
        self.stop_engine(true);
        self.shared.trust.clear();
        lock(&self.shared.state).last_move_at = 0;
    }

    /// A search runs, or a move was computed within `within_ms`.
    pub fn computer_playing(&self, within_ms: i64) -> bool {
        let state = lock(&self.shared.state);
        computer_playing(
            !state.active.is_empty(),
            state.last_move_at,
            within_ms,
            now_ms(),
        )
    }

    /// The engine's move from the position the
    /// moves reach, at the level's strength. Invalid input is refused at once; otherwise the
    /// search supersedes the one the same connection asked for, and waits its turn in line
    /// behind other connections' searches.
    pub fn best_move(
        &self,
        moves: Value,
        level: &str,
        configured: &str,
        options: Value,
    ) -> BoxFuture<'static, Result<String>> {
        let checked = check::<Moves>(J::from(&moves)).and_then(|Moves(moves)| {
            let options = match options {
                Value::Null => BestMoveOptions::default(),
                options => check::<BestMoveOptions>(J::from(&options))?,
            };
            Ok((moves, options))
        });
        let (moves, options) = match checked {
            Ok(checked) => checked,
            Err(refusal) => return Box::pin(ready(Err(CoreError::new(refusal)))),
        };
        self.best_move_with_settings(moves, level, options, ready(Ok(configured.to_string())))
    }

    /// Register before settings are read, so stop/suspend also cancel that wait.
    pub(crate) fn best_move_with_settings<'a>(
        &self,
        moves: Vec<String>,
        level: &str,
        options: BestMoveOptions,
        configured: impl Future<Output = Result<String>> + Send + 'a,
    ) -> BoxFuture<'a, Result<String>> {
        let mut request = match prepare(moves, options, level, "") {
            Ok(request) => request,
            Err(cause) => return Box::pin(ready(Err(cause))),
        };
        let connection = crate::connection::current_or_app();
        let (epoch, ticket, token, prev, done) = self.register(connection);
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // Declared so that the finish bookkeeping runs first, then the next search may start.
            let _done = done;
            let _finish = Finish {
                shared: Arc::clone(&shared),
                connection,
                ticket,
            };
            let mut key = String::new();
            let outcome = async {
                // Retain the serial chain even when a settings wait is cancelled.
                if let Some(prev) = prev {
                    let _ = prev.await;
                }
                request.configured = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(signal_aborted()),
                    result = configured => result?,
                };
                attempt(&shared, &request, epoch, &token, &mut key).await
            }
            .await;
            match outcome {
                Ok(mv) => Ok(mv),
                Err(cause) => {
                    let fen = match &request.fen {
                        Some(fen) => format!("fen={}", truncate_for_log(fen, 60)),
                        None => "startpos".to_string(),
                    };
                    let shown_key = if key.is_empty() {
                        "unstarted".to_string()
                    } else {
                        key.clone()
                    };
                    let context = format!(
                        "level={} moves={} engine={} {fen}",
                        request.level,
                        request.moves.len(),
                        truncate_for_log(&shown_key, 80),
                    );
                    if token.is_cancelled() || cause.aborted {
                        shared.ctx.host.log(
                            Level::Debug,
                            "engine",
                            &format!(
                                "Engine search superseded: {context} {}",
                                error_summary(&cause)
                            ),
                        );
                        Err(CoreError::new(SUPERSEDED))
                    } else {
                        shared.ctx.host.log(
                            Level::Warn,
                            "engine",
                            &format!("Engine search failed: {context} {}", error_summary(&cause)),
                        );
                        Err(cause)
                    }
                }
            }
        })
    }

    /// The synchronous part of `bestMove`: supersedes `connection`'s search, takes a ticket and
    /// queues behind the last search in line.
    fn register(&self, connection: u64) -> Ticket {
        let mut state = lock(&self.shared.state);
        let epoch = state.generation;
        state.ticket += 1;
        let ticket = state.ticket;
        let token = CancellationToken::new();
        if let Some((_, superseded)) = state.active.insert(connection, (ticket, token.clone())) {
            superseded.cancel();
        }
        self.shared.warm.keep();
        state.last_move_at = now_ms();
        let (done, queued) = oneshot::channel();
        let prev = state.serial.replace(queued);
        (epoch, ticket, token, prev, done)
    }
}

/// Checks the moves reach a position from the options' start.
fn prepare(
    moves: Vec<String>,
    options: BestMoveOptions,
    level: &str,
    configured: &str,
) -> Result<Request> {
    let chess960 = options.chess960 == Some(true);
    let start = options.fen.as_deref().unwrap_or(INITIAL_FEN);
    let invalid = || CoreError::new("Invalid computer position or move history.");
    let variant = options.variant.as_deref().and_then(fairy_variant);
    let moves = match (&options.variant, variant) {
        // Another variant's moves (drops among them) replay by its rules, and go as they are.
        (Some(key), Some(_)) => {
            let variant = kchess_domain::variant::Variant::from_key(key).ok_or_else(invalid)?;
            kchess_domain::replay::replay_setup_count(variant, start, &moves)
                .filter(|count| *count == moves.len())
                .ok_or_else(invalid)?;
            moves
        }
        // The rules write castling king-takes-rook; with `UCI_Chess960` off the engine rejects
        // that.
        _ => kchess_domain::replay::engine_line(start, &moves, chess960).ok_or_else(invalid)?,
    };
    Ok(Request {
        moves,
        fen: options.fen,
        movetime: options.movetime.map(|ms| ms as f64),
        chess960,
        variant,
        level: level.to_string(),
        configured: configured.to_string(),
    })
}

/// The level's search profile.
fn level_profile(level: &str) -> Result<LevelProfile> {
    let info = kchess_domain::training::engine_levels::info(level)
        .ok_or_else(|| CoreError::new(format!("Unknown engine level {level}.")))?;
    Ok(LevelProfile {
        time: info.time,
        uci_elo: info.uci_elo,
        skill: info.skill,
        random_move: info.random_move,
    })
}

/// One attempt at the search: waits in line, locates the engine, reuses or starts the warm
/// process, and searches under the scheduler's lease.
async fn attempt(
    shared: &Arc<Shared>,
    request: &Request,
    epoch: u64,
    token: &CancellationToken,
    key: &mut String,
) -> Result<String> {
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let located = shared
        .warm
        .locate(&request.configured, request.variant.is_some())
        .await?;
    key.clone_from(&located.key);
    if epoch != lock(&shared.state).generation {
        return Err(search_cancelled());
    }
    let target = shared.warm.open(&located)?.uci;
    let profile = level_profile(&request.level)?;
    let epoch_at_lease = epoch;
    shared
        .ctx
        .lease(Priority::Computer, token, async {
            // Re-check supersede after the queue wait: a `stop_engine` during admission
            // moved the generation, and this search must not start.
            if epoch_at_lease != lock(&shared.state).generation {
                return Err(search_cancelled());
            }
            target.ready().await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            if epoch_at_lease != lock(&shared.state).generation {
                return Err(search_cancelled());
            }
            target.write("ucinewgame")?;
            search_under_lease(shared, request, &profile, &target, token).await
        })
        .await
}

/// The part of a search that holds the engine budget: options, position, `go`, and the move.
async fn search_under_lease(
    shared: &Arc<Shared>,
    request: &Request,
    profile: &LevelProfile,
    target: &UciController,
    token: &CancellationToken,
) -> Result<String> {
    // The warm process reuses the previous game's options; only changed values are sent.
    // Chess960 is set every time: the warm engine may have played a Chess960 game before.
    let threads = shared.ctx.scheduler.search_threads().to_string();
    let elo = profile.uci_elo.filter(|elo| *elo != 0.0);
    let strength = match elo {
        Some(elo) => ("UCI_Elo", js_number(elo)),
        None => ("Skill Level", js_number(profile.skill.unwrap_or(20.0))),
    };
    let chess960 = request.chess960.to_string();
    let limit = elo.is_some().to_string();
    let mut options = vec![
        ("Threads", threads.as_str()),
        ("Hash", "64"),
        ("UCI_Chess960", chess960.as_str()),
        ("UCI_LimitStrength", limit.as_str()),
        (strength.0, strength.1.as_str()),
    ];
    // The variant first: a warm Fairy-Stockfish keeps the one it played last.
    if let Some(variant) = request.variant {
        options.insert(0, ("UCI_Variant", variant));
    }
    target.ensure_options(&options).await?;
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let movetime = request.movetime.unwrap_or(profile.time);
    let mut position = match &request.fen {
        Some(fen) => format!("position fen {fen}"),
        None => "position startpos".to_string(),
    };
    if !request.moves.is_empty() {
        position.push_str(&format!(" moves {}", request.moves.join(" ")));
    }
    target.write(&position)?;
    let random = (shared.random)() < profile.random_move.unwrap_or(0.0);
    let legal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&legal);
    let command = if random {
        "go perft 1".to_string()
    } else {
        format!("go movetime {}", js_number(movetime))
    };
    let timeout = Duration::from_millis(15_000f64.max(movetime + 10_000.0) as u64);
    let response = target
        .search(
            &command,
            move |line: &str| {
                if let Some(mv) = perft_move(line) {
                    lock(&sink).push(mv);
                }
            },
            Some(timeout),
            Some(token),
        )
        .await?;
    let mv = if random {
        let moves = lock(&legal).clone();
        if moves.is_empty() {
            None
        } else {
            let index = ((shared.random)() * moves.len() as f64).floor() as usize;
            moves.get(index.min(moves.len() - 1)).cloned()
        }
    } else {
        response.split_whitespace().nth(1).map(str::to_string)
    };
    match mv {
        Some(mv) if !mv.is_empty() && mv != "(none)" && mv != "0000" => Ok(mv),
        _ => Err(CoreError::new("Stockfish found no legal move.")),
    }
}

/// A `go perft 1` line `e2e4: 1` names one legal move.
fn perft_move(line: &str) -> Option<String> {
    let (mv, count) = line.split_once(": ")?;
    let bytes = mv.as_bytes();
    let square =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    // A move, a promotion (to a king in Antichess), or a Crazyhouse drop (`P@e3`).
    let shaped = match bytes {
        [piece, b'@', file, rank] => b"PNBRQ".contains(piece) && square(*file, *rank),
        [from_file, from_rank, to_file, to_rank, rest @ ..] => {
            square(*from_file, *from_rank)
                && square(*to_file, *to_rank)
                && matches!(rest, [] | [b'n' | b'b' | b'r' | b'q' | b'k'])
        }
        _ => false,
    };
    let counted = !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit());
    (shaped && counted).then(|| mv.to_string())
}

#[cfg(test)]
mod tests {
    use super::perft_move;

    /// The weakest levels pick from `go perft 1`'s moves: drops and Antichess's king promotions
    /// are among them.
    #[test]
    fn perft_lists_moves_drops_and_promotions() {
        assert_eq!(perft_move("e2e4: 1").as_deref(), Some("e2e4"));
        assert_eq!(perft_move("P@e3: 1").as_deref(), Some("P@e3"));
        assert_eq!(perft_move("a7a8q: 1").as_deref(), Some("a7a8q"));
        assert_eq!(perft_move("a7a8k: 1").as_deref(), Some("a7a8k"));
        for line in [
            "K@e3: 1",
            "e2e9: 1",
            "a7a8x: 1",
            "Nodes searched: 20",
            "e2e4: x",
        ] {
            assert_eq!(perft_move(line), None, "{line}");
        }
    }
}
