//! Chess engines, migrating from `core/src/services/{uci,engineScheduler,engine,managedEngine,
//! stockfishAsset,analysis,review}.ts` (see RUST_MIGRATION.md).
//!
//! `search`, `analysis` and `review` each own their engine processes and their state (the
//! TypeScript module-level `serviceState`, one per core instance). They share an `EngineContext`:
//! the host, the process registry, the engine budget and where engines are found.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::task::AbortHandle;

use crate::error::{CoreError, Result};
use crate::host::Host;
use scheduler::Scheduler;
use status::{EngineLocations, EngineStatus, engine_command};
use uci::{EngineHub, UciController};

pub mod analysis;
pub mod managed;
pub mod review;
pub mod scheduler;
pub mod search;
pub mod status;
pub mod uci;

/// How a status becomes the process that runs it (`spawnEngine`). The default runs the engine
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

    /// `spawnEngine`: refuses while an executable is being replaced, then starts the process.
    pub(crate) fn spawn(&self, status: &EngineStatus) -> Result<UciController> {
        self.hub.assert_available()?;
        UciController::start(&self.hub, (self.command)(status), uci::DEFAULT_DEADLINE)
    }
}

/// `kchess_domain` rules and validators by their TypeScript name, with JSON arguments.
pub(crate) fn domain(method: &str, args: Vec<Value>) -> Result<Value> {
    let text = serde_json::to_string(&args).map_err(|cause| CoreError::new(cause.to_string()))?;
    let output = kchess_domain::api::call(method, &text).map_err(CoreError::new)?;
    serde_json::from_str(&output).map_err(|cause| CoreError::new(cause.to_string()))
}

/// A `kchess_domain` `assert*` validator: the sidecar of values JSON cannot carry is empty.
pub(crate) fn validate(method: &str, value: &Value) -> Result<Value> {
    domain(method, vec![json!([]), value.clone()])
}

/// `parseInfo`: one UCI `info` line, with its scored principal variation from White's side.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct InfoLine {
    pub rank: f64,
    pub depth: f64,
    #[serde(default)]
    pub cp: Option<f64>,
    #[serde(default)]
    pub mate: Option<f64>,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ParsedInfo {
    pub line: InfoLine,
    #[serde(default)]
    pub nps: Option<f64>,
}

pub(crate) fn parse_info(text: &str, white_to_move: bool) -> Option<ParsedInfo> {
    let parsed = domain("parseInfo", vec![json!(text), json!(white_to_move)]).ok()?;
    serde_json::from_value(parsed).ok()
}

/// `Date.now()`: milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// `String(n)` for an engine option value: whole numbers carry no fraction.
pub(crate) fn js_number(value: f64) -> String {
    format!("{value}")
}

/// Runs `action` after `ms`, unless the returned handle is aborted first (`setTimeout`). Without
/// a Tokio runtime nothing is scheduled.
pub(crate) fn after(ms: u64, action: impl FnOnce() + Send + 'static) -> Option<AbortHandle> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let task = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        action();
    });
    Some(task.abort_handle())
}

/// Replaces a timer, aborting the previous one (`clearTimeout` then `setTimeout`).
pub(crate) fn replace_timer(slot: &mut Option<AbortHandle>, next: Option<AbortHandle>) {
    if let Some(old) = slot.take() {
        old.abort();
    }
    *slot = next;
}

/// Clears a timer (`clearTimeout`).
pub(crate) fn clear_timer(slot: &mut Option<AbortHandle>) {
    replace_timer(slot, None);
}

/// A uniform number in [0, 1), the source of `Math.random()`.
pub(crate) fn random_unit() -> f64 {
    let mut bytes = [0u8; 8];
    let filled = getrandom::getrandom(&mut bytes).is_ok();
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
