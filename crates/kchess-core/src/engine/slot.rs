//! One engine process a service keeps between uses: computer moves, analysis and review each own
//! a slot. A slot finds the engine to run, reuses its process while that is the same executable
//! and still healthy, and closes it once it has been idle long enough.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::AbortHandle;

use super::status::{EngineStatus, engine_identity, engine_status, variant_engine_status};
use super::uci::UciController;
use super::{EngineContext, after, clear_timer, replace_timer};
use crate::tasks::lock;
use kchess_contracts::error::{CoreError, ErrorKind, Result};

/// The message when no engine is configured or found.
pub(crate) const NO_ENGINE: &str = "Stockfish could not be found. Choose an engine in Settings.";
/// The message when the bundled variant engine is missing.
pub(crate) const NO_VARIANT_ENGINE: &str = "Fairy-Stockfish, which plays this variant, is missing.";

/// The engine a request runs on: its status and its identity (executable and version).
pub(crate) struct Located {
    pub status: EngineStatus,
    pub key: String,
}

/// An open engine process and the identity it was started for.
#[derive(Clone)]
pub(crate) struct OpenEngine {
    pub key: String,
    pub uci: Arc<UciController>,
}

#[derive(Default)]
struct State {
    engine: Option<OpenEngine>,
    idle: Option<AbortHandle>,
}

pub(crate) struct EngineSlot {
    ctx: EngineContext,
    idle_after: Duration,
    state: Mutex<State>,
}

impl EngineSlot {
    pub fn new(ctx: EngineContext, idle_after: Duration) -> Arc<EngineSlot> {
        Arc::new(EngineSlot {
            ctx,
            idle_after,
            state: Mutex::new(State::default()),
        })
    }

    /// The engine for `configured`, or the bundled Fairy-Stockfish for a variant other than
    /// Chess960; refused when it is not ready.
    pub async fn locate(&self, configured: &str, variant: bool) -> Result<Located> {
        let ctx = &self.ctx;
        let status = if variant {
            variant_engine_status(&*ctx.host, &ctx.locations).await
        } else {
            engine_status(&*ctx.host, &ctx.locations, configured).await
        };
        if !status.ready {
            return Err(CoreError::new(if variant {
                NO_VARIANT_ENGINE
            } else {
                NO_ENGINE
            })
            .with_kind(ErrorKind::NoEngine));
        }
        let key = engine_identity(&status).await?;
        Ok(Located { status, key })
    }

    /// The open process when it runs the located engine and is healthy; otherwise the old one is
    /// closed and a new one started. Stops the idle countdown.
    pub fn open(&self, located: &Located) -> Result<OpenEngine> {
        let mut state = lock(&self.state);
        clear_timer(&mut state.idle);
        if let Some(engine) = &state.engine
            && engine.key == located.key
            && engine.uci.failed().is_none()
        {
            return Ok(engine.clone());
        }
        if let Some(stale) = state.engine.take() {
            stale.uci.close();
        }
        let engine = OpenEngine {
            key: located.key.clone(),
            uci: Arc::new(self.ctx.spawn(&located.status)?),
        };
        state.engine = Some(engine.clone());
        Ok(engine)
    }

    /// Stops the idle countdown: work is about to use the engine.
    pub fn keep(&self) {
        clear_timer(&mut lock(&self.state).idle);
    }

    /// Starts the idle countdown, after which the engine closes. With no engine open there is
    /// nothing to count down.
    pub fn release(self: &Arc<Self>) {
        let mut state = lock(&self.state);
        if state.engine.is_none() {
            return;
        }
        let weak = Arc::downgrade(self);
        let millis = u64::try_from(self.idle_after.as_millis()).unwrap_or(u64::MAX);
        let idle = after(millis, move || {
            if let Some(slot) = weak.upgrade() {
                slot.close();
            }
        });
        replace_timer(&mut state.idle, idle);
    }

    /// Closes the engine and stops the idle countdown.
    pub fn close(&self) {
        let mut state = lock(&self.state);
        clear_timer(&mut state.idle);
        if let Some(engine) = state.engine.take() {
            engine.uci.close();
        }
    }

    /// No engine is open, or the open one has not failed.
    pub fn healthy(&self) -> bool {
        lock(&self.state)
            .engine
            .as_ref()
            .is_none_or(|engine| engine.uci.failed().is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::managed::Target;
    use crate::engine::scheduler::Scheduler;
    use crate::engine::status::EngineLocations;
    use crate::engine::uci::EngineHub;
    use crate::host::Host;
    use crate::test_host::{QUIET, TestHost};

    /// A slot whose engine is this test program listing no tests: a process that starts and
    /// ends, enough for the slot's bookkeeping.
    fn slot(idle_after: Duration) -> Arc<EngineSlot> {
        let host: Arc<dyn Host> = TestHost::quiet();
        let dir = std::env::temp_dir().join("kchess-slot-no-engines");
        let locations = EngineLocations {
            managed_dir: dir.join("managed"),
            target: Target::current(),
            bundled: dir.join("stockfish"),
            variant: dir.join("fairy-stockfish"),
        };
        let hub = Arc::new(EngineHub::new(Arc::clone(&host)));
        let scheduler = Arc::new(Scheduler::with_budget(1, || false));
        let mut ctx = EngineContext::new(host, hub, scheduler, locations);
        ctx.command = Arc::new(|_| {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "no such test"]);
            command
        });
        EngineSlot::new(ctx, idle_after)
    }

    async fn open(slot: &EngineSlot) {
        let located = Located {
            status: engine_status(&QUIET, &slot.ctx.locations, "").await,
            key: "engine".into(),
        };
        slot.open(&located).unwrap();
    }

    fn engine_open(slot: &EngineSlot) -> bool {
        lock(&slot.state).engine.is_some()
    }

    #[tokio::test]
    async fn an_idle_engine_closes_unless_work_keeps_it() {
        let slot = slot(Duration::from_millis(20));
        assert!(slot.healthy(), "no engine open");
        slot.release();
        assert!(lock(&slot.state).idle.is_none(), "nothing to count down");
        open(&slot).await;
        slot.release();
        slot.keep();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(engine_open(&slot), "kept past its idle time");
        slot.release();
        tokio::time::timeout(Duration::from_secs(5), async {
            while engine_open(&slot) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the idle engine closed");
        slot.close();
    }
}
