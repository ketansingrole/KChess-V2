//! Stockfish at full strength for practice positions: its replies and hints
//! from a start FEN and the moves played since. `cancel` (a restart, a takeback, another position)
//! stops the search this helper started and drops its move.

use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::AppCore;

#[derive(Default)]
pub struct EngineMove {
    /// A reply is being searched (hints search quietly).
    pub thinking: bool,
    pub error: Option<String>,
    /// Whether Stockfish is installed: unknown until asked.
    pub ready: Option<bool>,
    /// Bumped by each search and by `cancel`, so a late answer from an old search is dropped.
    generation: u64,
    /// This helper's search is still running in the core.
    running: bool,
    search: Option<Task<()>>,
}

/// The page that owns an `EngineMove`, so a finished search can find it again.
pub trait HasEngine: 'static + Sized {
    fn engine(&mut self) -> &mut EngineMove;
}

impl EngineMove {
    /// Ask whether Stockfish is installed (`engineReady`).
    pub fn check<V: HasEngine>(cx: &mut Context<V>) {
        let status = cx.global::<AppCore>().call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let ready =
                matches!(status.await, Ok(status) if status["ready"].as_bool() == Some(true));
            let _ = this.update(cx, |view, cx| {
                view.engine().ready = Some(ready);
                cx.notify();
            });
        })
        .detach();
    }

    /// The best move after `moves` from `fen`, handed to `done` unless cancelled or failed.
    pub fn ask<V: HasEngine>(
        view: &mut V,
        fen: &str,
        moves: &[String],
        movetime: u32,
        busy: bool,
        done: fn(&mut V, String, &mut Context<V>),
        cx: &mut Context<V>,
    ) {
        let engine = view.engine();
        engine.generation += 1;
        let mine = engine.generation;
        engine.running = true;
        if busy {
            engine.thinking = true;
        }
        let search = cx.global::<AppCore>().call(
            "bestMove",
            vec![
                json!(moves),
                json!("max"),
                json!({ "fen": fen, "movetime": movetime }),
            ],
        );
        let task = cx.spawn(async move |this, cx| {
            let reply = search.await;
            let _ = this.update(cx, |view, cx| {
                let engine = view.engine();
                if engine.generation != mine {
                    return;
                }
                engine.running = false;
                if busy {
                    engine.thinking = false;
                }
                match reply {
                    Ok(Value::String(best)) if !best.is_empty() => done(view, best, cx),
                    Ok(_) => {}
                    Err(cause) if !cause.aborted => {
                        crate::log::write(
                            Level::Warn,
                            "practice",
                            &format!("Engine move failed: {}", cause.message),
                        );
                        engine.error = Some(cause.message);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        });
        view.engine().search = Some(task);
    }

    /// Stop this helper's search, if it is still running, and forget its answer.
    pub fn cancel(&mut self, cx: &mut App) {
        self.generation += 1;
        self.search = None;
        self.thinking = false;
        self.error = None;
        if !std::mem::take(&mut self.running) {
            return;
        }
        let stop = cx.global::<AppCore>().call("stopEngine", Vec::new());
        cx.background_executor()
            .spawn(async move {
                if let Err(cause) = stop.await {
                    crate::log::write(
                        Level::Warn,
                        "practice",
                        &format!("Stopping the engine failed: {}", cause.message),
                    );
                }
            })
            .detach();
    }
}
