//! Helpers shared by the engine integration tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kchess_core::engine::EngineContext;
use kchess_core::engine::managed::Target;
use kchess_core::engine::scheduler::Scheduler;
use kchess_core::engine::status::EngineLocations;
use kchess_core::engine::uci::EngineHub;
use kchess_core::host::{Host, Level};
use tokio::process::Command;

pub mod review_store;
pub mod store;

/// Keeps every log line so a test can assert on what the controller reported.
#[derive(Default)]
pub struct Recording {
    pub lines: Mutex<Vec<String>>,
    pub events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Host for Recording {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.lines
            .lock()
            .expect("log lock")
            .push(format!("{} [{scope}] {message}", level.name()));
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .expect("event lock")
            .push((event.to_string(), payload));
    }
}

impl Recording {
    pub fn contains(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .expect("log lock")
            .iter()
            .any(|line| line.contains(needle))
    }
}

pub fn host() -> Arc<Recording> {
    Arc::new(Recording::default())
}

pub fn hub(host: &Arc<Recording>) -> EngineHub {
    EngineHub::new(host.clone())
}

/// A command that runs the scripted fake engine in `mode`.
pub fn fake(mode: &str, log: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kchess-fake-uci"));
    command.arg(mode);
    if let Some(log) = log {
        command.arg(log);
    }
    command
}

/// The executable the fake engine is built as. Its path passes the engine checks, so requests
/// can name it as their configured engine.
pub fn fake_path() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_kchess-fake-uci"))
        .display()
        .to_string()
}

/// The lines the fake engine has received so far, in order.
pub fn received(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A fresh log file name for a test.
pub fn log_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "kchess-engine-{name}-{}-{nanos}.log",
        std::process::id()
    ))
}

/// Engine locations that find nothing but the configured executable.
pub fn locations() -> EngineLocations {
    EngineLocations {
        managed_dir: std::env::temp_dir().join("kchess-no-managed-engine"),
        target: Target::current(),
        bundled_script: PathBuf::from("/nonexistent/kchess-bundled-stockfish.js"),
        node_path: PathBuf::from("node"),
        node_env: BTreeMap::new(),
    }
}

/// A scheduler with `budget` engine threads, on battery power when `battery` is set.
pub fn scheduler(budget: usize, battery: bool) -> Arc<Scheduler> {
    Arc::new(Scheduler::with_budget(budget, move || battery))
}

/// An engine context whose engines are the fake engine in `mode`, logging to `log`.
pub fn context(
    host: &Arc<Recording>,
    scheduler: Arc<Scheduler>,
    mode: &str,
    log: &Path,
) -> EngineContext {
    let mut ctx = EngineContext::new(host.clone(), Arc::new(hub(host)), scheduler, locations());
    let mode = mode.to_string();
    let log = log.to_path_buf();
    ctx.command = Arc::new(move |_status| fake(&mode, Some(&log)));
    ctx
}

/// Polls `check` every 10 ms until it returns `Some`, failing after `timeout`.
pub async fn wait_until<T>(
    timeout: Duration,
    what: &str,
    mut check: impl FnMut() -> Option<T>,
) -> T {
    let started = std::time::Instant::now();
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(started.elapsed() < timeout, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
