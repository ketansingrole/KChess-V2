//! Infinite and bounded analysis. A request streams
//! `engine:analysis` updates as the engine's lines change (at most every 120 ms), and ends with
//! exactly one final update: `completed` when the search finished, `interrupted` when it was
//! superseded or stopped (never completed), or `failed` with the error.
//!
//! The TypeScript module state lives in `Analysis`'s `State`, one per core instance. Analysis
//! runs on its own task: `start_analysis` returns its id at once, and `stop_analysis` (or a newer
//! request) ends it.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use super::status::{EngineStatus, engine_identity, engine_status};
use super::uci::{UciController, error_summary, lock, signal_aborted, truncate_for_log};
use super::{EngineContext, after, clear_timer, domain, parse_info, replace_timer, validate};
use crate::error::{CoreError, Result};
use crate::host::Level;

/// The event analysis updates are delivered as (`engine:analysis`).
pub const ANALYSIS_EVENT: &str = "engine:analysis";
/// Depth a bounded analysis searches to.
pub const DEPTH_LIMIT: u32 = 26;
/// Time a bounded analysis may take.
pub const TIME_LIMIT_MS: u64 = 60_000;
/// How long an idle analysis engine is kept.
const IDLE_MS: u64 = 120_000;
/// Updates are sent at most this often while lines change.
const EMIT_EVERY: Duration = Duration::from_millis(120);

/// Where updates go: the host's `engine:analysis` event, or a test's recorder.
pub type AnalysisSink = Arc<dyn Fn(AnalysisUpdate) + Send + Sync>;

/// Why an analysis ended (`reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Completed,
    Interrupted,
    Failed,
}

/// One line of a search's principal variations (`EngineLine`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineLine {
    /// 1 for the best line, 2 for the next best...
    pub rank: u32,
    pub depth: f64,
    /// Centipawns, when there is no forced mate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    /// Moves to mate: positive when White mates, negative when Black does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// Principal variation as UCI moves.
    pub pv: Vec<String>,
}

/// The payload of `engine:analysis` (`AnalysisUpdate`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUpdate {
    /// The request this belongs to; the frontends drop updates of superseded ones.
    pub id: u64,
    pub fen: String,
    pub depth: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nps: Option<f64>,
    pub lines: Vec<EngineLine>,
    pub done: bool,
    pub context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    pub engine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `AnalysisRequest` after validation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    fen: String,
    #[serde(default)]
    root_fen: Option<String>,
    #[serde(default)]
    moves: Option<Vec<String>>,
    /// Numbers arrive as floats from the validator.
    #[serde(default)]
    client_id: Option<f64>,
    lines: f64,
    #[serde(default)]
    infinite: bool,
}

/// The whole number a validated request carries, as the integer the update reports.
fn whole(value: f64) -> i64 {
    value as i64
}

#[derive(Clone)]
struct Engine {
    key: String,
    uci: Arc<UciController>,
}

/// The analysis service's state.
#[derive(Default)]
struct State {
    engine: Option<Engine>,
    /// The request in progress, with its id.
    current: Option<(u64, CancellationToken)>,
    next_id: u64,
    /// Finishes when the analysis queued last has finished (the promise chain `serial`).
    serial: Option<oneshot::Receiver<()>>,
    idle: Option<AbortHandle>,
}

struct Shared {
    ctx: EngineContext,
    state: Mutex<State>,
}

/// One client's analysis: its engine and the request in progress. Cloning shares them.
#[derive(Clone)]
pub struct AnalysisEngine {
    shared: Arc<Shared>,
}

/// The analysis service: one `AnalysisEngine` per connection (`crate::connection`). The
/// scheduler gives the whole thread budget to one lease at a time (`EngineContext::lease`), so
/// the newest request wins: starting an analysis interrupts every other connection's, whose
/// window restarts it when the person returns to it. An idle engine closes after two minutes as
/// before. Cloning shares them.
#[derive(Clone)]
pub struct Analysis {
    ctx: EngineContext,
    engines: Arc<Mutex<BTreeMap<u64, AnalysisEngine>>>,
}

impl Analysis {
    pub fn new(ctx: EngineContext) -> Analysis {
        Analysis {
            ctx,
            engines: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// The analysis of the calling connection, made on first use.
    fn mine(&self) -> AnalysisEngine {
        let connection = crate::connection::current_or_app();
        lock(&self.engines)
            .entry(connection)
            .or_insert_with(|| AnalysisEngine::new(self.ctx.clone()))
            .clone()
    }

    fn all(&self) -> Vec<AnalysisEngine> {
        lock(&self.engines).values().cloned().collect()
    }

    /// Interrupts every other connection's analysis: the engine goes to the newest request.
    fn interrupt_others(&self) {
        let connection = crate::connection::current_or_app();
        let others: Vec<AnalysisEngine> = lock(&self.engines)
            .iter()
            .filter(|(id, _)| **id != connection)
            .map(|(_, engine)| engine.clone())
            .collect();
        for engine in others {
            engine.stop_analysis(false);
        }
    }

    /// `startAnalysis` for the calling connection: supersedes only its own analysis.
    pub fn start_analysis(&self, raw: Value, configured: &str, send: AnalysisSink) -> Result<u64> {
        self.interrupt_others();
        self.mine().start_analysis(raw, configured, send)
    }

    pub(crate) async fn start_analysis_with_settings(
        &self,
        raw: Value,
        configured: impl Future<Output = Result<String>>,
        send: AnalysisSink,
    ) -> Result<u64> {
        self.interrupt_others();
        self.mine()
            .start_analysis_with_settings(raw, configured, send)
            .await
    }

    /// Any connection's analysis is in progress (the review queue waits for them).
    pub fn analysis_running(&self) -> bool {
        self.all().iter().any(AnalysisEngine::analysis_running)
    }

    /// Stops every connection's analysis: a live game, an engine being replaced, closing.
    pub fn stop_analysis(&self, kill: bool) {
        for engine in self.all() {
            engine.stop_analysis(kill);
        }
    }

    /// `stopAnalysis` from a client: stops the calling connection's analysis only.
    pub fn stop_mine(&self, kill: bool) {
        let connection = crate::connection::current_or_app();
        let engine = lock(&self.engines).get(&connection).cloned();
        if let Some(engine) = engine {
            engine.stop_analysis(kill);
        }
    }

    /// A connection went away: its analysis stops and its engine closes.
    pub fn forget(&self, connection: u64) {
        let engine = lock(&self.engines).remove(&connection);
        if let Some(engine) = engine {
            engine.stop_analysis(true);
        }
    }
}

/// What one analysis reports as it goes (`lines`, `nps`, the engine key), under its own lock.
struct Session {
    id: u64,
    fen: String,
    context: String,
    client_id: Option<i64>,
    sink: AnalysisSink,
    progress: Mutex<Progress>,
}

#[derive(Default)]
struct Progress {
    lines: BTreeMap<u32, EngineLine>,
    nps: Option<f64>,
    last_emit: Option<Instant>,
    engine: String,
}

impl Session {
    /// The update as it stands. `reason` marks the final update.
    fn snapshot(&self, reason: Option<Reason>, error: Option<String>) -> AnalysisUpdate {
        let progress = lock(&self.progress);
        let lines: Vec<EngineLine> = progress.lines.values().cloned().collect();
        AnalysisUpdate {
            id: self.id,
            fen: self.fen.clone(),
            depth: lines.first().map_or(0.0, |line| line.depth),
            nps: progress.nps,
            lines,
            done: reason.is_some(),
            context: self.context.clone(),
            client_id: self.client_id,
            reason,
            engine: progress.engine.clone(),
            error,
        }
    }

    /// Sends the final or a progress update; `send` runs without the progress lock.
    fn emit(&self, reason: Option<Reason>, error: Option<String>) {
        let update = self.snapshot(reason, error);
        (self.sink)(update);
    }

    /// One `info` line of the search. Lines past the requested count, and lines once the request
    /// is superseded, are ignored.
    fn on_info(&self, text: &str, white_to_move: bool, wanted: u32, token: &CancellationToken) {
        if token.is_cancelled() {
            return;
        }
        let Some(parsed) = parse_info(text, white_to_move) else {
            return;
        };
        let rank = parsed.line.rank;
        if rank > f64::from(wanted) {
            return;
        }
        let rank = rank as u32;
        let due = {
            let mut progress = lock(&self.progress);
            progress.lines.insert(
                rank,
                EngineLine {
                    rank,
                    depth: parsed.line.depth,
                    cp: parsed.line.cp,
                    mate: parsed.line.mate,
                    pv: parsed.line.pv,
                },
            );
            if let Some(nps) = parsed.nps {
                progress.nps = Some(nps);
            }
            let due = progress
                .last_emit
                .is_none_or(|last| last.elapsed() >= EMIT_EVERY);
            if due {
                progress.last_emit = Some(Instant::now());
            }
            due
        };
        if due {
            self.emit(None, None);
        }
    }
}

/// Runs when an analysis ends (`finally`): forgets it as current and starts the idle countdown.
struct Finish {
    shared: Arc<Shared>,
    id: u64,
}

/// An admitted request owns cancellation and its place in line, including while settings load.
struct PendingAnalysis {
    request: Request,
    id: u64,
    token: CancellationToken,
    prev: Option<oneshot::Receiver<()>>,
    finish: Finish,
    done: oneshot::Sender<()>,
}

impl Drop for Finish {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if state.current.as_ref().is_some_and(|(id, _)| *id == self.id) {
            state.current = None;
            let weak = Arc::downgrade(&self.shared);
            let idle = after(IDLE_MS, move || {
                if let Some(shared) = weak.upgrade() {
                    stop_locked(&mut lock(&shared.state), true);
                }
            });
            replace_timer(&mut state.idle, idle);
        }
    }
}

/// `stopAnalysis`' body, under the state lock.
fn stop_locked(state: &mut State, kill: bool) {
    if let Some((_, token)) = &state.current {
        token.cancel();
    }
    clear_timer(&mut state.idle);
    if kill && let Some(engine) = state.engine.take() {
        engine.uci.close();
    }
}

impl AnalysisEngine {
    pub fn new(ctx: EngineContext) -> AnalysisEngine {
        AnalysisEngine {
            shared: Arc::new(Shared {
                ctx,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// `startAnalysis(request, configured, send)`: validates the request, supersedes the analysis
    /// running, and starts this one on its own task. Returns the request's id.
    pub fn start_analysis(&self, raw: Value, configured: &str, send: AnalysisSink) -> Result<u64> {
        Ok(self.start_pending(self.reserve(raw)?, configured.to_string(), send))
    }

    /// Admit before the first await; stopping the service cancels pending settings reads too.
    pub(crate) async fn start_analysis_with_settings(
        &self,
        raw: Value,
        configured: impl Future<Output = Result<String>>,
        send: AnalysisSink,
    ) -> Result<u64> {
        let mut pending = self.reserve(raw)?;
        if let Some(prev) = pending.prev.take() {
            let _ = prev.await;
        }
        let configured = tokio::select! {
            biased;
            _ = pending.token.cancelled() => return Err(signal_aborted()),
            result = configured => result?,
        };
        Ok(self.start_pending(pending, configured, send))
    }

    fn reserve(&self, raw: Value) -> Result<PendingAnalysis> {
        let request: Request = serde_json::from_value(validate("assertAnalysisRequest", &raw)?)
            .map_err(|cause| CoreError::new(cause.to_string()))?;
        let (id, token, prev, done) = {
            let mut state = lock(&self.shared.state);
            state.next_id += 1;
            let id = state.next_id;
            if let Some((_, running)) = &state.current {
                running.cancel();
            }
            let token = CancellationToken::new();
            state.current = Some((id, token.clone()));
            clear_timer(&mut state.idle);
            let (done, queued) = oneshot::channel();
            let prev = state.serial.replace(queued);
            (id, token, prev, done)
        };
        Ok(PendingAnalysis {
            request,
            id,
            token,
            prev,
            done,
            finish: Finish {
                shared: Arc::clone(&self.shared),
                id,
            },
        })
    }

    fn start_pending(
        &self,
        pending: PendingAnalysis,
        configured: String,
        send: AnalysisSink,
    ) -> u64 {
        let PendingAnalysis {
            request,
            id,
            token,
            prev,
            done,
            finish,
        } = pending;
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            // Declared so that the finish bookkeeping runs first, then the next analysis may start.
            let _done = done;
            let _finish = finish;
            if let Some(prev) = prev {
                // A failed analysis still lets the next one start.
                let _ = prev.await;
            }
            let root = request
                .root_fen
                .clone()
                .unwrap_or_else(|| request.fen.clone());
            let moves = request.moves.clone().unwrap_or_default();
            let context = domain(
                "analysisContext",
                vec![Value::String(root.clone()), serde_json::json!(moves)],
            )
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
            let session = Arc::new(Session {
                id,
                fen: request.fen.clone(),
                context,
                client_id: request.client_id.map(whole),
                sink: send,
                progress: Mutex::new(Progress::default()),
            });
            let outcome = analyse(
                &shared,
                &session,
                &request,
                &root,
                &moves,
                &configured,
                &token,
            )
            .await;
            match outcome {
                Ok(()) => {}
                Err(cause) => {
                    let context = format!(
                        "fen={} client={}",
                        truncate_for_log(&request.fen, 60),
                        truncate_for_log(
                            &request
                                .client_id
                                .map_or("none".to_string(), |id| whole(id).to_string()),
                            40
                        ),
                    );
                    if token.is_cancelled() || cause.aborted {
                        shared.ctx.host.log(
                            Level::Debug,
                            "analysis",
                            &format!("Analysis interrupted: {context} {}", error_summary(&cause)),
                        );
                        session.emit(Some(Reason::Interrupted), None);
                    } else {
                        shared.ctx.host.log(
                            Level::Warn,
                            "analysis",
                            &format!("Analysis failed: {context} {}", error_summary(&cause)),
                        );
                        session.emit(Some(Reason::Failed), Some(cause.message));
                    }
                }
            }
        });
        id
    }

    /// `analysisRunning`: an analysis is in progress.
    pub fn analysis_running(&self) -> bool {
        lock(&self.shared.state).current.is_some()
    }

    /// `stopAnalysis(kill)`: ends the analysis in progress (its final update is `interrupted`);
    /// with `kill`, also closes the engine.
    pub fn stop_analysis(&self, kill: bool) {
        stop_locked(&mut lock(&self.shared.state), kill);
    }
}

/// The analysis itself: the engine for the request's identity, under the scheduler's lease.
async fn analyse(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    request: &Request,
    root: &str,
    moves: &[String],
    configured: &str,
    token: &CancellationToken,
) -> Result<()> {
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let status: EngineStatus =
        engine_status(&*shared.ctx.host, &shared.ctx.locations, configured).await;
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    if !status.ready {
        return Err(CoreError::new(
            "Stockfish could not be found. Choose an engine in Settings.",
        ));
    }
    let key = engine_identity(&status).await?;
    lock(&session.progress).engine = key.clone();
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let target = {
        let mut state = lock(&shared.state);
        let current = state
            .engine
            .as_ref()
            .is_some_and(|engine| engine.key == key && engine.uci.failed().is_none());
        if !current {
            if let Some(stale) = state.engine.take() {
                stale.uci.close();
            }
            let uci = Arc::new(shared.ctx.spawn(&status)?);
            state.engine = Some(Engine {
                key: key.clone(),
                uci,
            });
        }
        state
            .engine
            .as_ref()
            .map(|engine| Arc::clone(&engine.uci))
            .ok_or_else(|| CoreError::new("The engine is unavailable."))?
    };
    let white_to_move = request.fen.split(' ').nth(1).unwrap_or("w") == "w";
    let stop_token = token.clone();
    shared
        .ctx
        .lease(2, move || stop_token.cancel(), token, async {
            target.ready().await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            // Stepping through a game reuses this process with identical options; only changed
            // values are sent. Chess960 is always on: castling is king-takes-rook in the history.
            let threads = shared.ctx.scheduler.search_threads().to_string();
            let lines = whole(request.lines).to_string();
            target
                .ensure_options(&[
                    ("Threads", threads.as_str()),
                    ("Hash", "128"),
                    ("MultiPV", lines.as_str()),
                    ("UCI_Chess960", "true"),
                ])
                .await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            let mut position = format!("position fen {root}");
            if !moves.is_empty() {
                position.push_str(&format!(" moves {}", moves.join(" ")));
            }
            target.write(&position)?;
            let (command, timeout) = if request.infinite {
                ("go infinite".to_string(), None)
            } else {
                (
                    format!("go depth {DEPTH_LIMIT} movetime {TIME_LIMIT_MS}"),
                    Some(Duration::from_millis(TIME_LIMIT_MS + 10_000)),
                )
            };
            let listener = Arc::clone(session);
            let listener_token = token.clone();
            let wanted = whole(request.lines) as u32;
            target
                .search(
                    &command,
                    move |line: &str| {
                        listener.on_info(line, white_to_move, wanted, &listener_token);
                    },
                    timeout,
                    Some(token),
                )
                .await?;
            session.emit(Some(Reason::Completed), None);
            Ok(())
        })
        .await
}
