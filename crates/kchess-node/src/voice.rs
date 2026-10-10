//! `NativeVoiceModel`: the offline voice model cache (`kchess_core::voice`) for the Electron main
//! process. The model is not a `CoreApi` method (the shell serves its file over `kchess://` and
//! reports its progress over IPC), so it has its own small surface beside `NativeCore`.
//! Status and progress cross as JSON text, as the rules do; errors reject with the message.

use std::path::PathBuf;
use std::sync::Arc;

use kchess_core::host::{Host, Level};
use kchess_core::voice::{Archive, Listener, Progress, VoiceModel, VoiceModelCache};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use serde_json::{Value, json};

// Weak, as in `core.rs`: the callbacks never keep the host process alive on their own.
type Callback = ThreadsafeFunction<String, (), String, napi::Status, false, true>;

/// Logs to the host's log sink as `NativeCore` does: `(json)` with `{ level, scope, message }`.
struct LogHost {
    log: Callback,
}

impl Host for LogHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        let line = json!({ "level": level.name(), "scope": scope, "message": message });
        self.log
            .call(line.to_string(), ThreadsafeFunctionCallMode::NonBlocking);
    }

    fn emit(&self, _event: &str, _payload: Value) {}
}

#[napi(object)]
pub struct NativeVoiceModelOptions {
    /// A local zip used instead of the download (the e2e fixture the tooling prepares).
    pub archive_path: Option<String>,
}

#[napi]
pub struct NativeVoiceModel {
    cache: VoiceModelCache,
}

#[napi]
impl NativeVoiceModel {
    /// The cache in `directory`, which holds `model.tar.gz` and its marker.
    #[napi(constructor)]
    pub fn new(
        directory: String,
        options: Option<NativeVoiceModelOptions>,
        log: Callback,
    ) -> NativeVoiceModel {
        let archive = match options.and_then(|options| options.archive_path) {
            Some(path) => Archive::File(PathBuf::from(path)),
            None => Archive::Url(VoiceModel::official().url),
        };
        let cache = VoiceModelCache::new(
            Arc::new(LogHost { log }),
            PathBuf::from(directory),
            archive,
            VoiceModel::official(),
        );
        NativeVoiceModel { cache }
    }

    /// The published archive's path (served as `kchess://app/voice/model.tar.gz`).
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.cache.path().to_string_lossy().into_owned()
    }

    /// `VoiceModelStatus` as JSON.
    #[napi(catch_unwind)]
    pub async fn status(&self) -> napi::Result<String> {
        let cache = self.cache.clone();
        let status = cache.status().await;
        serde_json::to_string(&status).map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Verify or prepare the model; resolves with its path. `progress` receives each
    /// `VoiceModelProgress` as JSON until the call settles.
    #[napi(catch_unwind)]
    pub async fn ensure(&self, progress: Callback) -> napi::Result<String> {
        let cache = self.cache.clone();
        let listener: Listener = Arc::new(move |report: &Progress| {
            if let Ok(json) = serde_json::to_string(report) {
                progress.call(json, ThreadsafeFunctionCallMode::NonBlocking);
            }
        });
        cache
            .ensure(Some(listener))
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|error| napi::Error::from_reason(error.message))
    }
}

/// `VOICE_MODEL` as JSON: `{ name, url, sha256 }`.
#[napi(catch_unwind)]
pub fn voice_model_metadata() -> napi::Result<String> {
    serde_json::to_string(&VoiceModel::official())
        .map_err(|e| napi::Error::from_reason(e.to_string()))
}
