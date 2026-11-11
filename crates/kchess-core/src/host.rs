//! What the core needs from its host: somewhere to log, somewhere to send events, and its
//! configuration.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use kchess_contracts::CoreEvent;
pub use kchess_contracts::error::Level;

/// Host callbacks. They must not block: the core calls them from its worker threads.
pub trait Host: Send + Sync + 'static {
    /// One log line under a `[scope]`, as `logDebug/logInfo/logWarn/logError` write it.
    fn log(&self, level: Level, scope: &str, message: &str);
    /// An event for the host to deliver.
    fn emit(&self, event: CoreEvent);
    /// An event for one client only (`crate::connection`): an analysis's updates, a watch's
    /// frames, a host request. A host with one client delivers it like any event.
    fn emit_to(&self, connection: u64, event: CoreEvent) {
        let _ = connection;
        self.emit(event);
    }
}

/// Where the core keeps its files, and where the host keeps the engines. A client that starts the
/// shared service passes it as JSON (`kchess-service serve --config`), without `data_dir`, which
/// the profile names.
/// Field names stay host-agnostic on purpose: each host maps its own environment onto them, and
/// the headless core knows none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// KChess's data directory; it exists.
    #[serde(skip)]
    pub data_dir: PathBuf,
    /// The main database of an earlier release, imported once into an empty profile.
    pub legacy_database_path: Option<PathBuf>,
    /// The Stockfish executable shipped beside the app. `None` when there is none; the bundled
    /// engine is then not ready.
    pub bundled_engine_path: Option<PathBuf>,
    /// The Fairy-Stockfish executable shipped beside the app, which analyses variants other than
    /// Chess960. `None` when there is none; variant analysis is then unavailable.
    pub variant_engine_path: Option<PathBuf>,
    /// Where the downloaded Stockfish is kept; the default is `engines/stockfish/current` in the
    /// data directory.
    pub managed_engine_dir: Option<PathBuf>,
    /// The `kchess-voice` speech recognition helper, with libvosk beside it. `None` when the host
    /// ships none; voice input is then unavailable.
    pub voice_helper_path: Option<PathBuf>,
    /// Seal Lichess tokens with the profile's own credentials (`secrets.rs`), so every KChess app
    /// opens them; the host then only opens tokens an earlier release sealed. Off, the host seals
    /// every token itself.
    pub system_credentials: bool,
    /// The test switches; `None` outside automated runs. A packaged build ignores them
    /// (`Config::test_switches`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<TestSwitches>,
}

impl Config {
    /// A configuration with only the data directory: no bundled engine and no legacy database.
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            ..Config::default()
        }
    }

    /// The test switches this build honours: none in a packaged build (`crate::dev_overrides`),
    /// so software that can set what a signed KChess is started with cannot send its logins
    /// elsewhere.
    pub fn test_switches(&self) -> Option<&TestSwitches> {
        self.test.as_ref().filter(|_| crate::dev_overrides())
    }
}

/// What an automated run changes (`KCHESS_STORE_DEBUG=1` and the `KCHESS_TEST_*` variables): the
/// profile's key in a file instead of the OS credential store, local stand-ins for the services
/// KChess reaches, and a voice model and microphone that need no download or device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TestSwitches {
    /// Lichess's origin (`KCHESS_TEST_LICHESS_BASE`).
    pub lichess_origin: Option<String>,
    /// Chess.com's origin (`KCHESS_TEST_CHESSCOM_BASE`).
    pub chesscom_origin: Option<String>,
    /// OpenAI's origin (`KCHESS_TEST_OPENAI_BASE`).
    pub openai_origin: Option<String>,
    /// The speech model to install in place of the official one (`KCHESS_TEST_VOICE_ARCHIVE`,
    /// `_NAME`, `_SHA256`, all three).
    pub voice_model: Option<TestVoiceModel>,
    /// A WAV file the app plays in place of the microphone (`KCHESS_TEST_VOICE_AUDIO`).
    pub voice_audio: Option<PathBuf>,
}

/// A local speech model archive, and the folder name and SHA-256 it must have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestVoiceModel {
    pub archive: PathBuf,
    pub name: String,
    pub sha256: String,
}

impl TestSwitches {
    /// The switches the variables `var` reads set (empty ones count as unset): `None` unless
    /// `honoured` and `KCHESS_STORE_DEBUG=1`.
    pub fn from_vars(honoured: bool, var: impl Fn(&str) -> Option<String>) -> Option<TestSwitches> {
        let var = |name: &str| var(name).filter(|value| !value.is_empty());
        if !honoured || var("KCHESS_STORE_DEBUG").as_deref() != Some("1") {
            return None;
        }
        let origin = |name: &str| var(name).map(|origin| origin.trim_end_matches('/').to_string());
        let voice_model = match (
            var("KCHESS_TEST_VOICE_ARCHIVE"),
            var("KCHESS_TEST_VOICE_NAME"),
            var("KCHESS_TEST_VOICE_SHA256"),
        ) {
            (Some(archive), Some(name), Some(sha256)) => Some(TestVoiceModel {
                archive: PathBuf::from(archive),
                name,
                sha256,
            }),
            _ => None,
        };
        Some(TestSwitches {
            lichess_origin: origin("KCHESS_TEST_LICHESS_BASE"),
            chesscom_origin: origin("KCHESS_TEST_CHESSCOM_BASE"),
            openai_origin: origin("KCHESS_TEST_OPENAI_BASE"),
            voice_model,
            voice_audio: var("KCHESS_TEST_VOICE_AUDIO").map(PathBuf::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// The test switches are read only under `KCHESS_STORE_DEBUG=1` in a build that honours them.
    #[test]
    fn the_test_switches_need_the_store_debug_switch() {
        let set = vars(&[
            ("KCHESS_STORE_DEBUG", "1"),
            ("KCHESS_TEST_LICHESS_BASE", "http://127.0.0.1:9/"),
            ("KCHESS_TEST_VOICE_ARCHIVE", "/model.zip"),
            ("KCHESS_TEST_VOICE_NAME", "model"),
        ]);
        let switches = TestSwitches::from_vars(true, &set).expect("on");
        assert_eq!(
            switches.lichess_origin.as_deref(),
            Some("http://127.0.0.1:9")
        );
        assert_eq!(switches.chesscom_origin, None);
        assert_eq!(switches.voice_model, None, "a model needs all three");
        assert_eq!(TestSwitches::from_vars(false, &set), None);
        let off = vars(&[("KCHESS_TEST_LICHESS_BASE", "http://127.0.0.1:9")]);
        assert_eq!(TestSwitches::from_vars(true, off), None);
    }

    /// The configuration crosses to the service with the field names earlier builds of this
    /// release write; the profile is named apart.
    #[test]
    fn the_config_crosses_as_json_without_its_profile() {
        let config = Config {
            bundled_engine_path: Some(PathBuf::from("stockfish")),
            system_credentials: true,
            ..Config::new(PathBuf::from("profile"))
        };
        let text = serde_json::to_value(&config).expect("json");
        assert_eq!(
            text,
            serde_json::json!({
                "legacyDatabasePath": null,
                "bundledEnginePath": "stockfish",
                "variantEnginePath": null,
                "managedEngineDir": null,
                "voiceHelperPath": null,
                "systemCredentials": true,
            })
        );
        let back: Config = serde_json::from_value(text).expect("config");
        assert_eq!(
            back,
            Config {
                data_dir: PathBuf::new(),
                ..config
            }
        );
        let older: Config =
            serde_json::from_value(serde_json::json!({ "bundledEnginePath": "sf" })).expect("old");
        assert!(!older.system_credentials && older.test.is_none());
    }
}
