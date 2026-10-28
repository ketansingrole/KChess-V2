//! One visible position's engine search (`usePositionEngine`), for boards that are not the
//! analysis board: a new request replaces the search running (stopping it in the engine, not only
//! dropping its answers), and only updates for the request asked last are kept.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::game::tree::analysis_context;

/// Client ids unique across every position engine in the app, as Electron's start from the clock.
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(0);

fn next_client() -> u64 {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let _ = NEXT_CLIENT.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
    NEXT_CLIENT.fetch_add(1, Ordering::Relaxed) + 1
}

/// The context an update must carry to belong to a request (`analysisContext`).
fn context_of(request: &Value) -> String {
    let fen = request["fen"].as_str().unwrap_or_default();
    let root = request["rootFen"].as_str().unwrap_or(fen);
    let moves: Vec<String> = serde_json::from_value(request["moves"].clone()).unwrap_or_default();
    analysis_context(root, &moves)
}

#[derive(Default)]
pub struct PositionEngine {
    /// The search asked for (`AnalysisRequest`), or none.
    wanted: Option<Value>,
    context: String,
    client_id: u64,
    latest_id: f64,
    /// The latest update for the search asked for (`AnalysisUpdate`).
    pub update: Option<Value>,
    pub error: Option<String>,
    pub busy: bool,
    _start: Option<Task<()>>,
}

impl PositionEngine {
    pub fn wanted(&self) -> Option<&Value> {
        self.wanted.as_ref()
    }

    /// Search `request` (or nothing), replacing the search running; the same request again
    /// changes nothing. `engine` finds this engine in its owner.
    pub fn search<V: 'static>(
        &mut self,
        request: Option<Value>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        if request == self.wanted {
            return;
        }
        self.restart(request, engine, cx);
    }

    /// Search `request` again from the start (a retry, or after the core reconnected).
    pub fn restart<V: 'static>(
        &mut self,
        request: Option<Value>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        let id = next_client();
        self.client_id = id;
        self.latest_id = 0.;
        self.context = request.as_ref().map(context_of).unwrap_or_default();
        self.wanted = request.clone();
        self.update = None;
        self.error = None;
        self.busy = request.is_some();
        // Cancellation reaches the core's search, including when nothing replaces it.
        let stopped = cx.global::<AppCore>().call("stopAnalysis", Vec::new());
        let Some(request) = request else {
            self._start = None;
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
        self._start = Some(cx.spawn(async move |this, cx| {
            let _ = stopped.await;
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let start = this.update(cx, |owner, cx| {
                (engine(owner).client_id == id).then(|| {
                    let mut request = request;
                    request["clientId"] = json!(id);
                    cx.global::<AppCore>().call("startAnalysis", vec![request])
                })
            });
            let Ok(Some(start)) = start else {
                return;
            };
            if let Err(cause) = start.await {
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
            }
        }));
    }

    /// An `engine:analysis` event: kept, and true, when it is for the search asked for.
    pub fn receive(&mut self, update: &Value) -> bool {
        let Some(wanted) = &self.wanted else {
            return false;
        };
        let id = update["id"].as_f64().unwrap_or_default();
        if update["clientId"].as_f64() != Some(self.client_id as f64)
            || update["fen"] != wanted["fen"]
            || update["context"].as_str() != Some(self.context.as_str())
            || id < self.latest_id
        {
            return false;
        }
        self.latest_id = id;
        self.error = update["error"].as_str().map(str::to_string);
        self.update = self.error.is_none().then(|| update.clone());
        self.busy = update["done"] != json!(true);
        true
    }

    /// The best line of the latest update.
    pub fn best(&self) -> Option<&Value> {
        self.update.as_ref()?["lines"].as_array()?.first()
    }

    pub fn lines(&self) -> Vec<Value> {
        self.update
            .as_ref()
            .and_then(|update| update["lines"].as_array().cloned())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{PositionEngine, next_client};
    use serde_json::json;

    #[test]
    fn client_ids_never_repeat() {
        let first = next_client();
        assert!(next_client() > first);
    }

    #[test]
    fn only_the_search_asked_for_is_kept() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let request = json!({ "fen": fen, "moves": [] });
        let mut engine = PositionEngine {
            context: super::context_of(&request),
            wanted: Some(request),
            client_id: 7,
            ..PositionEngine::default()
        };
        let context = engine.context.clone();
        let update = |client: u64, id: f64, fen: &str| {
            json!({
                "clientId": client, "id": id, "fen": fen, "context": context,
                "depth": 10, "lines": [{ "pv": ["e7e5"], "cp": 20 }], "done": false,
            })
        };
        let (stale, other, mine) = (
            update(6, 1., fen),
            update(7, 1., "8/8/8/8/8/8/8/8 w"),
            update(7, 2., fen),
        );
        assert!(!engine.receive(&stale));
        assert!(!engine.receive(&other));
        assert!(engine.receive(&mine));
        assert!(engine.busy);
        assert_eq!(engine.best().unwrap()["cp"], json!(20));
        // An older update of the same search arriving late is dropped.
        assert!(!engine.receive(&update(7, 1., fen)));
    }
}
