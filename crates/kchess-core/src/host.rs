//! What the core needs from its host while services are split between the languages:
//! somewhere to log, somewhere to send events, and the directories it owns.

use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Host callbacks. They must not block: the core calls them from its worker threads.
pub trait Host: Send + Sync + 'static {
    /// One log line under a `[scope]`, as `logDebug/logInfo/logWarn/logError` write it.
    fn log(&self, level: Level, scope: &str, message: &str);
    /// An event for the host to deliver (`CoreEvents` names, or internal `host:*` requests).
    fn emit(&self, event: &str, payload: Value);
    /// An event for one client only (`crate::connection`): an analysis's updates, a watch's
    /// frames, a host request. A host with one client delivers it like any event.
    fn emit_to(&self, connection: u64, event: &str, payload: Value) {
        let _ = connection;
        self.emit(event, payload);
    }
}

/// Where the core keeps its files, and where the host keeps the engines.
/// Field names stay host-agnostic on purpose: each host maps its own environment onto them, and
/// the headless core knows none.
#[derive(Clone, Debug)]
pub struct Config {
    /// KChess's data directory; it exists.
    pub data_dir: PathBuf,
    /// The main database of an earlier release, imported once into an empty profile
    /// (`CorePlatform.legacyDatabasePath`).
    pub legacy_database_path: Option<PathBuf>,
    /// The Stockfish executable shipped beside the app. `None` when there is none; the bundled
    /// engine is then not ready.
    pub bundled_engine_path: Option<PathBuf>,
    /// Where the downloaded Stockfish is kept (`CorePlatform.managedEngineDir`); the default is
    /// `engines/stockfish/current` in the data directory.
    pub managed_engine_dir: Option<PathBuf>,
    /// The `kchess-voice` speech recognition helper, with libvosk beside it
    /// (`CorePlatform.voiceHelperPath`). `None` when the host ships none; voice input is then
    /// unavailable.
    pub voice_helper_path: Option<PathBuf>,
    /// Seal Lichess tokens with the profile's own credentials (`secrets.rs`), so every KChess app
    /// opens them; the host then only opens tokens an earlier release sealed. Off, the host seals
    /// every token itself (`CorePlatform.secrets`).
    pub system_credentials: bool,
}

impl Config {
    /// A configuration with only the data directory: no bundled engine and no legacy database.
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            legacy_database_path: None,
            bundled_engine_path: None,
            managed_engine_dir: None,
            voice_helper_path: None,
            system_credentials: false,
        }
    }
}
