//! One visible position's engine search, for boards that are not the
//! analysis board: a new request replaces the search running (the core cancels it when the next
//! starts), and only updates for the request asked last are kept.

use kchess_contracts::calls;
use kchess_contracts::types::{AnalysisRequest, AnalysisUpdate, EngineLine};
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::*;
use kchess_contracts::Level;

use crate::core::AppCore;
use crate::game::tree::analysis_context;

/// Client ids unique across every engine search in the app, the analysis board's included.
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(0);

pub fn next_client() -> u64 {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let _ = NEXT_CLIENT.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
    NEXT_CLIENT.fetch_add(1, Ordering::Relaxed) + 1
}

/// The context an update must carry to belong to a request.
fn context_of(request: &AnalysisRequest) -> String {
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    analysis_context(root, request.moves.as_deref().unwrap_or_default())
}

#[derive(Default)]
pub struct PositionEngine {
    /// The search asked for, or none.
    wanted: Option<AnalysisRequest>,
    context: String,
    client_id: u64,
    latest_id: u64,
    /// The latest update for the search asked for.
    pub update: Option<AnalysisUpdate>,
    pub error: Option<String>,
    pub busy: bool,
    _start: Option<Task<()>>,
}

impl PositionEngine {
    /// Search `request` (or nothing), replacing the search running; the same request again
    /// changes nothing. `engine` finds this engine in its owner.
    pub fn search<V: 'static>(
        &mut self,
        request: Option<AnalysisRequest>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        if request == self.wanted {
            return;
        }
        self.restart(request, engine, cx);
    }

    /// Search what was asked for again from the start (a retry, or after the core reconnected).
    pub fn retry<V: 'static>(
        &mut self,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        self.restart(self.wanted.clone(), engine, cx);
    }

    fn restart<V: 'static>(
        &mut self,
        request: Option<AnalysisRequest>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        let id = next_client();
        self.client_id = id;
        self.latest_id = 0;
        self.context = request.as_ref().map(context_of).unwrap_or_default();
        self.wanted = request.clone();
        self.update = None;
        self.error = None;
        self.busy = request.is_some();
        let Some(mut request) = request else {
            // Cancellation reaches the core's search, not only its answers.
            self._start = None;
            let stopped = cx
                .global::<AppCore>()
                .request(calls::StopAnalysis { kill: None });
            cx.background_executor()
                .spawn(async move {
                    if let Err(cause) = stopped.await {
                        crate::log::write(
                            Level::Warn,
                            "position-engine",
                            &format!("stopping analysis failed: {}", cause.message),
                        );
                    }
                })
                .detach();
            return;
        };
        request.client_id = Some(id);
        let start = cx
            .global::<AppCore>()
            .request(calls::StartAnalysis { request });
        self._start = Some(cx.spawn(async move |this, cx| {
            let Err(cause) = start.await else {
                return;
            };
            let _ = this.update(cx, |owner, cx| {
                let engine = engine(owner);
                if engine.client_id != id {
                    return;
                }
                crate::log::write(
                    Level::Warn,
                    "position-engine",
                    &format!("starting analysis failed: {}", cause.message),
                );
                engine.error = Some(cause.message);
                engine.busy = false;
                cx.notify();
            });
        }));
    }

    /// An `engine:analysis` event: kept, and true, when it is for the search asked for.
    pub fn receive(&mut self, update: &AnalysisUpdate) -> bool {
        let Some(wanted) = &self.wanted else {
            return false;
        };
        if update.client_id != i64::try_from(self.client_id).ok()
            || wanted.fen != update.fen
            || update.context != self.context
            || update.id < self.latest_id
        {
            return false;
        }
        self.latest_id = update.id;
        self.error = update.error.clone();
        self.update = self.error.is_none().then(|| update.clone());
        self.busy = !update.done;
        true
    }

    /// The best line of the latest update.
    pub fn best(&self) -> Option<&EngineLine> {
        self.update.as_ref()?.lines.first()
    }

    pub fn lines(&self) -> &[EngineLine] {
        self.update
            .as_ref()
            .map_or(&[], |update| update.lines.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::{PositionEngine, next_client};
    use kchess_contracts::types::AnalysisRequest;
    use serde_json::json;

    #[test]
    fn client_ids_never_repeat() {
        let first = next_client();
        assert!(next_client() > first);
    }

    #[test]
    fn only_the_search_asked_for_is_kept() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let request = AnalysisRequest {
            fen: fen.to_string(),
            lines: 1,
            infinite: None,
            root_fen: None,
            moves: Some(Vec::new()),
            client_id: None,
            variant: None,
        };
        let mut engine = PositionEngine {
            context: super::context_of(&request),
            wanted: Some(request),
            client_id: 7,
            ..PositionEngine::default()
        };
        let context = engine.context.clone();
        let update = |client: u64, id: u64, fen: &str| {
            serde_json::from_value(json!({
                "clientId": client, "id": id, "fen": fen, "context": context,
                "depth": 10.0, "lines": [{ "rank": 1, "depth": 10.0, "pv": ["e7e5"], "cp": 20.0 }],
                "done": false, "engine": "stockfish",
            }))
            .expect("an update")
        };
        let (stale, other, mine) = (
            update(6, 1, fen),
            update(7, 1, "8/8/8/8/8/8/8/8 w"),
            update(7, 2, fen),
        );
        assert!(!engine.receive(&stale));
        assert!(!engine.receive(&other));
        assert!(engine.receive(&mine));
        assert!(engine.busy);
        assert_eq!(engine.best().unwrap().cp, Some(20.));
        // An older update of the same search arriving late is dropped.
        assert!(!engine.receive(&update(7, 1, fen)));
    }
}
