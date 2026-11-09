//! Stockfish at full strength for practice positions: its replies and hints
//! from a start FEN and the moves played since. `cancel` (a restart, a takeback, another position)
//! stops the search this helper started and drops its move.

use gpui_kit::*;
use kchess_contracts::Level;
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
        // This search replaces any before it: whether one is thinking, and an earlier failure,
        // are its own. A quiet search after a busy one left `thinking` on for good.
        engine.thinking = busy;
        engine.error = None;
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

#[cfg(test)]
mod tests {
    use super::{EngineMove, HasEngine};
    use gpui_kit::{AppContext as _, Context};

    #[derive(Default)]
    struct Page {
        engine: EngineMove,
    }

    impl HasEngine for Page {
        fn engine(&mut self) -> &mut EngineMove {
            &mut self.engine
        }
    }

    fn played(_: &mut Page, _: String, _: &mut Context<Page>) {}

    /// A quiet search (a hint) asked for while a reply was searched replaced it, and nothing
    /// was left to turn `thinking` off: the page waited on the engine for good.
    #[gpui_kit::test]
    fn a_quiet_search_after_a_busy_one_is_not_thinking(cx: &mut gpui_kit::TestAppContext) {
        let mut app = crate::e2e::Harness::start(cx);
        let (thinking, error) = app.with_window(|_, cx| {
            let page = cx.new(|_| Page::default());
            page.update(cx, |page, cx| {
                let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
                EngineMove::ask(page, start, &[], 100, true, played, cx);
                page.engine.error = Some("The engine stopped".into());
                EngineMove::ask(page, start, &[], 100, false, played, cx);
                (page.engine.thinking, page.engine.error.clone())
            })
        });
        assert!(!thinking);
        assert_eq!(error, None);
    }
}
