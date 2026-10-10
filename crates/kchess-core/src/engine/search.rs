//! Computer moves (`core/src/services/engine.ts`'s `bestMove`, `stopEngine`, `resetEngine` and
//! `computerPlaying`). One warm engine process serves consecutive moves; a new search supersedes
//! the one running (`SUPERSEDED`), and the process goes after two idle minutes.
//!
//! The TypeScript module state lives in `Search`'s `State`, one per core instance. As in
//! TypeScript, `best_move` takes its place in line and supersedes the running search as soon as
//! it is called, before its future is polled; the search itself runs inside the returned future.
//! Dropping that future ends the search.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::{BoxFuture, ready};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use super::status::{EngineStatus, EngineTrust, computer_playing, engine_identity, engine_status};
use super::uci::{
    UciController, error_summary, lock, search_cancelled, signal_aborted, truncate_for_log,
};
use super::{
    EngineContext, after, clear_timer, domain, js_number, now_ms, random_unit, replace_timer,
    validate,
};
use crate::error::{CoreError, Result};
use crate::host::Level;

/// The message a superseded search rejects with (`SUPERSEDED`).
pub const SUPERSEDED: &str = "Engine search superseded.";
/// The start position when a request gives none.
const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// How long an idle warm engine is kept.
const WARM_IDLE_MS: u64 = 120_000;

/// The random source `Math.random()` is read from; tests replace it.
pub type RandomSource = Arc<dyn Fn() -> f64 + Send + Sync>;

/// `BestMoveOptions` after validation.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveOptions {
    #[serde(default)]
    fen: Option<String>,
    #[serde(default)]
    movetime: Option<f64>,
    #[serde(default)]
    chess960: bool,
}

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
    level: String,
    configured: String,
}

#[derive(Clone)]
struct Warm {
    key: String,
    uci: Arc<UciController>,
}

/// `serviceState` of `engine.ts`.
#[derive(Default)]
struct State {
    /// Bumped by every `stopEngine`; a search that sees a newer value was superseded.
    generation: u64,
    /// The search in progress, with the ticket that identifies it.
    active: Option<(u64, CancellationToken)>,
    ticket: u64,
    last_move_at: i64,
    warm: Option<Warm>,
    /// Finishes when the search queued last has finished (the promise chain `serial`).
    serial: Option<oneshot::Receiver<()>>,
    idle: Option<AbortHandle>,
}

struct Shared {
    ctx: EngineContext,
    random: RandomSource,
    trust: EngineTrust,
    state: Mutex<State>,
}

/// The computer's search. Cloning shares the same engine and state.
#[derive(Clone)]
pub struct Search {
    shared: Arc<Shared>,
}

/// Runs when a search ends (`finally`), before the next search in line may start.
struct Finish {
    shared: Arc<Shared>,
    ticket: u64,
}

impl Drop for Finish {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if state
            .active
            .as_ref()
            .is_some_and(|(ticket, _)| *ticket == self.ticket)
        {
            state.active = None;
            let weak = Arc::downgrade(&self.shared);
            let idle = after(WARM_IDLE_MS, move || {
                if let Some(shared) = weak.upgrade() {
                    close_warm(&shared);
                }
            });
            replace_timer(&mut state.idle, idle);
        }
    }
}

fn close_warm(shared: &Shared) {
    let mut state = lock(&shared.state);
    if let Some(warm) = state.warm.take() {
        warm.uci.close();
    }
}

/// `stopEngine`'s body, under the state lock.
fn stop_locked(state: &mut State, close: bool) {
    state.generation += 1;
    if let Some((_, token)) = &state.active {
        token.cancel();
    }
    if close {
        if let Some(warm) = state.warm.take() {
            warm.uci.close();
        }
        clear_timer(&mut state.idle);
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
                ctx,
                random,
                trust: EngineTrust::default(),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// `engineThreads`: the engine budget the scheduler grants searches.
    pub fn engine_threads(&self) -> usize {
        self.shared.ctx.scheduler.search_threads()
    }

    /// `trustEnginePath`: an engine path the renderer may persist.
    pub fn trust_engine_path(&self, path: &Path) {
        self.shared.trust.trust(path);
    }

    /// `isTrustedEnginePath`.
    pub fn is_trusted_engine_path(&self, path: &Path) -> bool {
        self.shared
            .trust
            .is_trusted(path, &self.shared.ctx.locations.managed_dir)
    }

    /// `stopEngine(close)`: supersedes the search running; with `close`, also ends the warm
    /// engine and its idle timer.
    pub fn stop_engine(&self, close: bool) {
        stop_locked(&mut lock(&self.shared.state), close);
    }

    /// `resetEngine`: stops and closes the engine, forgets trusted paths and the last move.
    pub fn reset_engine(&self) {
        self.stop_engine(true);
        self.shared.trust.clear();
        lock(&self.shared.state).last_move_at = 0;
    }

    /// `computerPlaying(withinMs)`: a search runs, or a move was computed within `within_ms`.
    pub fn computer_playing(&self, within_ms: i64) -> bool {
        let state = lock(&self.shared.state);
        computer_playing(
            state.active.is_some(),
            state.last_move_at,
            within_ms,
            now_ms(),
        )
    }

    /// `bestMove(moves, level, configured, options)`: the engine's move from the position the
    /// moves reach, at the level's strength. Invalid input is refused at once; otherwise the
    /// search supersedes the one running and waits its turn in line.
    pub fn best_move(
        &self,
        moves: Value,
        level: &str,
        configured: &str,
        options: Value,
    ) -> BoxFuture<'static, Result<String>> {
        let request = match prepare(moves, options, level, configured) {
            Ok(request) => request,
            Err(cause) => return Box::pin(ready(Err(cause))),
        };
        let (epoch, ticket, token, prev, done) = self.register();
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // Declared so that the finish bookkeeping runs first, then the next search may start.
            let _done = done;
            let _finish = Finish {
                shared: Arc::clone(&shared),
                ticket,
            };
            let mut key = String::new();
            let outcome = attempt(&shared, &request, epoch, &token, &mut key, prev).await;
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

    /// The synchronous part of `bestMove`: supersedes the search running, takes a ticket and
    /// queues behind the last search in line.
    fn register(&self) -> Ticket {
        let mut state = lock(&self.shared.state);
        stop_locked(&mut state, false);
        let epoch = state.generation;
        state.ticket += 1;
        let ticket = state.ticket;
        let token = CancellationToken::new();
        state.active = Some((ticket, token.clone()));
        clear_timer(&mut state.idle);
        state.last_move_at = now_ms();
        let (done, queued) = oneshot::channel();
        let prev = state.serial.replace(queued);
        (epoch, ticket, token, prev, done)
    }
}

/// Validates the untrusted arguments the way `bestMove` does, then checks the moves reach a
/// position.
fn prepare(moves: Value, options: Value, level: &str, configured: &str) -> Result<Request> {
    let moves: Vec<String> = serde_json::from_value(validate("assertMoves", &moves)?)
        .map_err(|cause| CoreError::new(cause.to_string()))?;
    let options = if options.is_null() {
        json!({})
    } else {
        options
    };
    let options: MoveOptions = serde_json::from_value(validate("assertBestMoveOptions", &options)?)
        .map_err(|cause| CoreError::new(cause.to_string()))?;
    let start = options.fen.as_deref().unwrap_or(INITIAL_FEN);
    if kchess_domain::replay::replay_positions_count(start, &moves) != moves.len() + 1 {
        return Err(CoreError::new("Invalid computer position or move history."));
    }
    Ok(Request {
        moves,
        fen: options.fen,
        movetime: options.movetime,
        chess960: options.chess960,
        level: level.to_string(),
        configured: configured.to_string(),
    })
}

/// The level's search profile (`engineLevelInfo`).
fn level_profile(level: &str) -> Result<LevelProfile> {
    let info = domain("engineLevelInfo", vec![json!(level)])?;
    if info.is_null() {
        return Err(CoreError::new(format!("Unknown engine level {level}.")));
    }
    serde_json::from_value(info).map_err(|cause| CoreError::new(cause.to_string()))
}

/// The warm engine for `status`: kept when it is the same executable and still healthy,
/// otherwise replaced.
fn warm_engine(shared: &Shared, key: &str, status: &EngineStatus) -> Result<Arc<UciController>> {
    let mut state = lock(&shared.state);
    let current = state
        .warm
        .as_ref()
        .is_some_and(|warm| warm.key == key && warm.uci.failed().is_none());
    if !current {
        if let Some(stale) = state.warm.take() {
            stale.uci.close();
        }
        let uci = Arc::new(shared.ctx.spawn(status)?);
        state.warm = Some(Warm {
            key: key.to_string(),
            uci,
        });
    }
    state
        .warm
        .as_ref()
        .map(|warm| Arc::clone(&warm.uci))
        .ok_or_else(|| CoreError::new("The engine is unavailable."))
}

/// One attempt at the search: waits in line, locates the engine, reuses or starts the warm
/// process, and searches under the scheduler's lease.
async fn attempt(
    shared: &Arc<Shared>,
    request: &Request,
    epoch: u64,
    token: &CancellationToken,
    key: &mut String,
    prev: Option<oneshot::Receiver<()>>,
) -> Result<String> {
    if let Some(prev) = prev {
        // A search that failed still lets the next one start.
        let _ = prev.await;
    }
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let status = engine_status(
        &*shared.ctx.host,
        &shared.ctx.locations,
        &request.configured,
    )
    .await;
    if !status.ready {
        return Err(CoreError::new(
            "Stockfish could not be found. Choose an engine in Settings.",
        ));
    }
    *key = engine_identity(&status).await?;
    if epoch != lock(&shared.state).generation {
        return Err(search_cancelled());
    }
    let target = warm_engine(shared, key, &status)?;
    let profile = level_profile(&request.level)?;
    let stop_token = token.clone();
    shared
        .ctx
        .lease(3, move || stop_token.cancel(), token, async {
            target.ready().await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
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
    target
        .ensure_options(&[
            ("Threads", threads.as_str()),
            ("Hash", "64"),
            ("UCI_Chess960", chess960.as_str()),
            ("UCI_LimitStrength", limit.as_str()),
            (strength.0, strength.1.as_str()),
        ])
        .await?;
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

/// A `go perft 1` line `e2e4: 1` names one legal move (`/^([a-h][1-8][a-h][1-8][nbrq]?): \d+$/`).
fn perft_move(line: &str) -> Option<String> {
    let (mv, count) = line.split_once(": ")?;
    let bytes = mv.as_bytes();
    let square =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    let shaped = (bytes.len() == 4 || (bytes.len() == 5 && b"nbrq".contains(&bytes[4])))
        && square(bytes[0], bytes[1])
        && square(bytes[2], bytes[3]);
    let counted = !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit());
    (shaped && counted).then(|| mv.to_string())
}
