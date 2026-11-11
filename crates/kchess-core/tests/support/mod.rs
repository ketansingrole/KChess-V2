//! Helpers shared by the engine integration tests.
#![allow(dead_code)]

use kchess_contracts::CoreEvent;
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

pub mod reviews;
pub mod store;
pub mod wire;

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

    fn emit(&self, event: CoreEvent) {
        let (event, payload) = (event.name(), event.into_payload());
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

/// A copy of the fake engine in `dir` that runs in `mode` with no arguments, as the core starts
/// a configured engine. A copy rather than a shell script, so it runs on Windows too.
pub fn engine_copy(dir: &Path, mode: &str) -> String {
    let path = dir.join(format!("engine-{mode}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(fake_path(), &path).expect("the engine copy");
    path.display().to_string()
}

/// Saves `path` as the engine executable in the core's settings, as the settings page does.
pub fn save_engine_path(core: &kchess_core::Core, path: &str) {
    use kchess_core::store::games;
    let database = core.database_for_tests();
    let mut settings = database
        .read(|c| games::get_settings(c.db))
        .expect("settings");
    settings["enginePath"] = serde_json::json!(path);
    let settings: games::Settings = serde_json::from_value(settings).expect("valid settings");
    database
        .write(|c| games::save_settings(c.db, &settings))
        .expect("save settings");
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
        bundled: PathBuf::from("/nonexistent/kchess-bundled-stockfish"),
        variant: PathBuf::from("/nonexistent/kchess-fairy-stockfish"),
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

/// How long `until` waits before it fails the test: a ceiling for a condition that never comes,
/// not a pause, so it is generous for a loaded Windows runner.
pub const UNTIL_LIMIT: Duration = Duration::from_secs(30);

/// Polls `check` every 10 ms until it holds, failing the test after `UNTIL_LIMIT`: a test never
/// carries on in a state it did not reach.
pub async fn until(what: &str, mut check: impl FnMut() -> bool) {
    wait_until(UNTIL_LIMIT, what, || check().then_some(())).await;
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

/// The first connection to `listener`, once the code under test has made it: the work that
/// connects has started. The stream is returned to keep it open.
pub async fn accepted(listener: &std::net::TcpListener) -> std::net::TcpStream {
    listener
        .set_nonblocking(true)
        .expect("a nonblocking listener");
    wait_until(Duration::from_secs(20), "a connection", || {
        listener.accept().ok().map(|(stream, _)| stream)
    })
    .await
}

/// The engine at `variable`'s path: Stockfish (`KCHESS_ENGINE_PATH`, which CI fetches with `cargo
/// run -p kchess-core --example fetch_stockfish`) or Fairy-Stockfish
/// (`KCHESS_VARIANT_ENGINE_PATH`). The tests that call it are ignored unless the variable was set
/// when they were built (`build.rs`); set to a missing file, the test fails.
pub fn engine(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the engine"));
    assert!(
        path.is_file(),
        "{variable} names no file: {}",
        path.display()
    );
    path
}
