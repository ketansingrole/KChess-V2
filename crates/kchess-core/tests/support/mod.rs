//! Helpers shared by the engine integration tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use kchess_core::engine::uci::EngineHub;
use kchess_core::host::{Host, Level};
use tokio::process::Command;

/// Keeps every log line so a test can assert on what the controller reported.
#[derive(Default)]
pub struct Recording {
    pub lines: Mutex<Vec<String>>,
}

impl Host for Recording {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.lines
            .lock()
            .expect("log lock")
            .push(format!("{} [{scope}] {message}", level.name()));
    }

    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
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

/// The lines the fake engine has received so far, in order.
pub fn received(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}
