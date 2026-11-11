//! Infinite and bounded analysis. A request streams
//! `engine:analysis` updates as the engine's lines change (at most every 120 ms), and ends with
//! exactly one final update: `completed` when the search finished, `interrupted` when it was
//! superseded or stopped (never completed), or `failed` with the error.
//!
//! Analysis runs on its own task: `start_analysis` returns its id at once, and `stop_analysis`
//! (or a newer request) ends it.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::EngineContext;
use super::job::{self, Line, Place};
use super::scheduler::Priority;
use super::slot::EngineSlot;
use super::status::fairy_variant;
use super::uci::truncate_for_log;
use crate::tasks::{aborted as signal_aborted, lock};
use kchess_contracts::check::check;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::AnalysisRequest;
pub use kchess_contracts::types::{AnalysisReason as Reason, AnalysisUpdate, EngineLine};
use kchess_domain::misc::value::J;
use kchess_domain::training::uci_info::parse_info;

/// Depth a bounded analysis searches to.
pub const DEPTH_LIMIT: u32 = 26;
/// Time a bounded analysis may take.
pub const TIME_LIMIT_MS: u64 = 60_000;
/// How long an idle analysis engine is kept.
const IDLE: Duration = Duration::from_secs(120);
/// Updates are sent at most this often while lines change.
const EMIT_EVERY: Duration = Duration::from_millis(120);

/// Where updates go: the host's `engine:analysis` event, or a test's recorder.
pub type AnalysisSink = Arc<dyn Fn(AnalysisUpdate) + Send + Sync>;

/// The analysis service's state.
#[derive(Default)]
struct State {
    /// The request in progress, with its id.
    current: Option<(u64, CancellationToken)>,
    next_id: u64,
    /// The analyses in the order they were asked for.
    line: Line,
}

struct Shared {
    ctx: EngineContext,
    state: Mutex<State>,
    engine: Arc<EngineSlot>,
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
        request: AnalysisRequest,
        configured: impl Future<Output = Result<String>>,
        send: AnalysisSink,
    ) -> Result<u64> {
        self.interrupt_others();
        self.mine()
            .start_analysis_with_settings(request, configured, send)
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

    /// Finishes once every connection's analysis queued so far has ended (`AnalysisEngine::settled`).
    pub async fn settled(&self) {
        for engine in self.all() {
            engine.settled().await;
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
        // A rank the engine did not write as a number is no line.
        if !rank.is_finite() || rank > f64::from(wanted) {
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

/// Runs when an analysis ends, however it ends: forgets it as current and starts the idle countdown.
struct Finish {
    shared: Arc<Shared>,
    id: u64,
}

/// An admitted request owns cancellation and its place in line, including while settings load.
struct PendingAnalysis {
    request: AnalysisRequest,
    id: u64,
    token: CancellationToken,
    place: Place,
    finish: Finish,
}

impl Drop for Finish {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if state.current.as_ref().is_some_and(|(id, _)| *id == self.id) {
            state.current = None;
            self.shared.engine.release();
        }
    }
}

impl AnalysisEngine {
    pub fn new(ctx: EngineContext) -> AnalysisEngine {
        AnalysisEngine {
            shared: Arc::new(Shared {
                engine: EngineSlot::new(ctx.clone(), IDLE),
                ctx,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Validates the request, supersedes the analysis
    /// running, and starts this one on its own task. Returns the request's id.
    pub fn start_analysis(&self, raw: Value, configured: &str, send: AnalysisSink) -> Result<u64> {
        let request = check::<AnalysisRequest>(J::from(&raw)).map_err(CoreError::new)?;
        Ok(self.start_pending(self.reserve(request), configured.to_string(), send))
    }

    /// Admit before the first await; stopping the service cancels pending settings reads too.
    pub(crate) async fn start_analysis_with_settings(
        &self,
        request: AnalysisRequest,
        configured: impl Future<Output = Result<String>>,
        send: AnalysisSink,
    ) -> Result<u64> {
        let mut pending = self.reserve(request);
        pending.place.turn().await;
        let configured = job::configured(&pending.token, configured).await?;
        Ok(self.start_pending(pending, configured, send))
    }

    fn reserve(&self, request: AnalysisRequest) -> PendingAnalysis {
        let (id, token, place) = {
            let mut state = lock(&self.shared.state);
            state.next_id += 1;
            let id = state.next_id;
            if let Some((_, running)) = &state.current {
                running.cancel();
            }
            let token = CancellationToken::new();
            state.current = Some((id, token.clone()));
            self.shared.engine.keep();
            (id, token, state.line.join())
        };
        PendingAnalysis {
            request,
            id,
            token,
            place,
            finish: Finish {
                shared: Arc::clone(&self.shared),
                id,
            },
        }
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
            place,
            finish,
        } = pending;
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            // Declared before the finish bookkeeping, which so runs first: then the next
            // analysis may start.
            let mut place = place;
            let _finish = finish;
            place.turn().await;
            let root = request
                .root_fen
                .clone()
                .unwrap_or_else(|| request.fen.clone());
            let moves = request.moves.clone().unwrap_or_default();
            let context = kchess_domain::records::context::analysis_context(&root, &moves);
            let session = Arc::new(Session {
                id,
                fen: request.fen.clone(),
                context,
                client_id: request.client_id.map(|id| id as i64),
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
            if let Err(cause) = outcome {
                let context = format!(
                    "fen={} client={}",
                    truncate_for_log(&request.fen, 60),
                    truncate_for_log(
                        &request
                            .client_id
                            .map_or("none".to_string(), |id| id.to_string()),
                        40
                    ),
                );
                let labels = ("Analysis interrupted", "Analysis failed");
                if job::interrupted(
                    &*shared.ctx.host,
                    "analysis",
                    labels,
                    &context,
                    &cause,
                    &token,
                ) {
                    session.emit(Some(Reason::Interrupted), None);
                } else {
                    session.emit(Some(Reason::Failed), Some(cause.message));
                }
            }
        });
        id
    }

    pub fn analysis_running(&self) -> bool {
        lock(&self.shared.state).current.is_some()
    }

    /// Ends the analysis in progress (its final update is `interrupted`); with `kill`, also
    /// closes the engine.
    pub fn stop_analysis(&self, kill: bool) {
        let state = lock(&self.shared.state);
        if let Some((_, token)) = &state.current {
            token.cancel();
        }
        if kill {
            self.shared.engine.close();
        }
    }

    /// Finishes once every analysis queued so far has ended: its final update sent and it no
    /// longer current. It takes its own place in the `serial` line, so an analysis queued after
    /// it still waits its turn.
    pub async fn settled(&self) {
        let mut place = lock(&self.shared.state).line.join();
        place.turn().await;
    }
}

/// The analysis itself: the engine for the request's identity, under the scheduler's lease.
async fn analyse(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    request: &AnalysisRequest,
    root: &str,
    moves: &[String],
    configured: &str,
    token: &CancellationToken,
) -> Result<()> {
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    // Variants other than Chess960 go to the bundled Fairy-Stockfish, whatever engine is chosen.
    let fairy = request.variant.as_deref().and_then(fairy_variant);
    let located = shared.engine.locate(configured, fairy.is_some()).await;
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    let located = located?;
    lock(&session.progress).engine = located.key.clone();
    let target = shared.engine.open(&located)?.uci;
    let white_to_move = request.fen.split(' ').nth(1).unwrap_or("w") == "w";
    shared
        .ctx
        .lease(Priority::Analysis, token, async {
            target.ready().await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            // Stepping through a game reuses this process with identical options; only changed
            // values are sent.
            let threads = shared.ctx.scheduler.search_threads().to_string();
            let lines = request.lines.to_string();
            let options = [
                ("Threads", threads.as_str()),
                ("Hash", "128"),
                ("MultiPV", lines.as_str()),
                job::rules_option(fairy),
            ];
            target.ensure_options(&options).await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            target.write(&job::position_command(Some(root), moves))?;
            let (command, timeout) = if request.infinite == Some(true) {
                ("go infinite".to_string(), None)
            } else {
                (
                    format!("go depth {DEPTH_LIMIT} movetime {TIME_LIMIT_MS}"),
                    Some(Duration::from_millis(TIME_LIMIT_MS + 10_000)),
                )
            };
            let listener = Arc::clone(session);
            let listener_token = token.clone();
            let wanted = request.lines;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the ranks asked for become lines.
    #[test]
    fn lines_past_the_ranks_asked_for_are_ignored() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink_sent = Arc::clone(&sent);
        let session = Session {
            id: 1,
            fen: String::new(),
            context: String::new(),
            client_id: None,
            sink: Arc::new(move |update| lock(&sink_sent).push(update)),
            progress: Mutex::new(Progress::default()),
        };
        let token = CancellationToken::new();
        for rank in [1, 3, 2] {
            session.on_info(
                &format!("info depth 9 multipv {rank} score cp 20 pv e2e4"),
                true,
                2,
                &token,
            );
        }
        let ranks: Vec<u32> = lock(&session.progress).lines.keys().copied().collect();
        assert_eq!(ranks, vec![1, 2]);
    }
}
