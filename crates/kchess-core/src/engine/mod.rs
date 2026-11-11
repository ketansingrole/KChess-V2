//! Chess engines: UCI processes, the scheduler, search, the managed engine, analysis and review.
//!
//! `search`, `analysis` and `review` each own an engine slot and their state, one per core
//! instance. They share an `EngineContext`: the host, the process registry, the engine budget and
//! where engines are found.

use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio::task::AbortHandle;

use crate::host::Host;
use kchess_contracts::error::Result;
use scheduler::{Priority, Scheduler};
use status::{EngineLocations, EngineStatus, engine_command};
use uci::{EngineHub, UciController};

pub mod analysis;
pub mod bridge;
pub mod managed;
pub mod review;
pub mod scheduler;
pub mod search;
pub(crate) mod slot;
pub mod status;
pub mod uci;

/// How a status becomes the process that runs it. The default runs the engine
/// `status` names; tests substitute a scripted engine.
pub type CommandFactory = Arc<dyn Fn(&EngineStatus) -> Command + Send + Sync>;

/// What the engine services share with the core: the host, the process registry, the engine
/// budget, and where engines are found.
#[derive(Clone)]
pub struct EngineContext {
    pub host: Arc<dyn Host>,
    pub hub: Arc<EngineHub>,
    pub scheduler: Arc<Scheduler>,
    pub locations: Arc<EngineLocations>,
    pub command: CommandFactory,
}

impl EngineContext {
    pub fn new(
        host: Arc<dyn Host>,
        hub: Arc<EngineHub>,
        scheduler: Arc<Scheduler>,
        locations: EngineLocations,
    ) -> EngineContext {
        let locations = Arc::new(locations);
        let for_factory = Arc::clone(&locations);
        EngineContext {
            host,
            hub,
            scheduler,
            locations,
            command: Arc::new(move |status| engine_command(&for_factory, status)),
        }
    }

    /// Waits for the engine budget at `priority`, then runs `work` under it. The budget is
    /// released on every exit path; a higher priority cancels `token`, which stops the search.
    pub(crate) async fn lease<T>(
        &self,
        priority: Priority,
        token: &tokio_util::sync::CancellationToken,
        work: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        self.scheduler.with_lease(priority, token, work).await
    }

    /// Refuses while an executable is being replaced, then starts the process.
    pub(crate) fn spawn(&self, status: &EngineStatus) -> Result<UciController> {
        self.hub.assert_available()?;
        UciController::start(&self.hub, (self.command)(status), uci::DEFAULT_DEADLINE)
    }
}

/// Runs `action` after `ms`, unless the returned handle is aborted first. Without
/// a Tokio runtime nothing is scheduled.
pub(crate) fn after(ms: u64, action: impl FnOnce() + Send + 'static) -> Option<AbortHandle> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let task = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        action();
    });
    Some(task.abort_handle())
}

/// Replaces a timer, aborting the previous one.
pub(crate) fn replace_timer(slot: &mut Option<AbortHandle>, next: Option<AbortHandle>) {
    if let Some(old) = slot.take() {
        old.abort();
    }
    *slot = next;
}

/// Clears a timer.
pub(crate) fn clear_timer(slot: &mut Option<AbortHandle>) {
    replace_timer(slot, None);
}

/// A uniform number in [0, 1).
pub(crate) fn random_unit() -> f64 {
    let mut bytes = [0u8; 8];
    let filled = getrandom::fill(&mut bytes).is_ok();
    let bits = if filled {
        u64::from_le_bytes(bytes)
    } else {
        // Without OS entropy, the clock still varies between calls.
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        )
        .unwrap_or(0)
    };
    (bits >> 11) as f64 / (1u64 << 53) as f64
}
