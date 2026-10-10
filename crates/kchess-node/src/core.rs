//! `NativeCore`: the Rust core's services for a Node or Electron host. Calls are asynchronous
//! (`call(method, argsJson) → Promise<resultJson>`); logs and events reach the host through
//! callbacks given to the constructor. Errors reject with the core's message; cancellations
//! carry `AbortError:` before it, which the host turns into an `AbortError`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use kchess_core::{Config, Core, Host, Level};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use serde_json::Value;

// Weak: the callbacks never keep the host process alive on their own.
type Callback = ThreadsafeFunction<String, (), String, napi::Status, false, true>;

struct JsHost {
    /// `(json)` with `{ level, scope, message }`.
    log: Callback,
    /// `(json)` with `{ event, payload }`.
    emit: Callback,
}

impl Host for JsHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        let line = serde_json::json!({ "level": level.name(), "scope": scope, "message": message });
        self.log
            .call(line.to_string(), ThreadsafeFunctionCallMode::NonBlocking);
    }

    fn emit(&self, event: &str, payload: Value) {
        let message = serde_json::json!({ "event": event, "payload": payload });
        self.emit
            .call(message.to_string(), ThreadsafeFunctionCallMode::NonBlocking);
    }
}

#[napi(object)]
pub struct NativeCoreOptions {
    pub data_dir: String,
    pub legacy_database_path: Option<String>,
    /// `CorePlatform.bundledEnginePath`: the bundled Stockfish script, run by `node_path`.
    pub bundled_engine_path: Option<String>,
    /// `process.execPath`, the Node executable that runs the bundled script.
    pub node_path: Option<String>,
    /// `CorePlatform.nodeEnv`: extra environment for the bundled script.
    pub node_env: Option<HashMap<String, String>>,
    /// `CorePlatform.managedEngineDir`: where the downloaded Stockfish is kept.
    pub managed_engine_dir: Option<String>,
}

#[napi]
pub struct NativeCore {
    core: Arc<Core>,
}

#[napi]
impl NativeCore {
    /// Opens the core over a profile; throws when a live core already runs there.
    #[napi(constructor)]
    pub fn new(
        options: NativeCoreOptions,
        log: Callback,
        emit: Callback,
    ) -> napi::Result<NativeCore> {
        let config = Config {
            legacy_database_path: options.legacy_database_path.map(PathBuf::from),
            bundled_engine_path: options.bundled_engine_path.map(PathBuf::from),
            node_path: options.node_path.map(PathBuf::from),
            node_env: options
                .node_env
                .unwrap_or_default()
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            managed_engine_dir: options.managed_engine_dir.map(PathBuf::from),
            ..Config::new(PathBuf::from(options.data_dir))
        };
        let core = Core::open(config, Arc::new(JsHost { log, emit }))
            .map_err(|error| napi::Error::from_reason(error.message))?;
        Ok(NativeCore {
            core: Arc::new(core),
        })
    }

    /// Run a core method with a JSON array of arguments; resolves with its JSON result.
    #[napi(catch_unwind)]
    pub async fn call(&self, method: String, args: String) -> napi::Result<String> {
        let core = Arc::clone(&self.core);
        let args: Vec<Value> = serde_json::from_str(&args)
            .map_err(|e| napi::Error::from_reason(format!("Invalid arguments: {e}")))?;
        match core.call(&method, args).await {
            Ok(value) => Ok(value.to_string()),
            Err(error) if error.aborted => Err(napi::Error::from_reason(format!(
                "AbortError: {}",
                error.message
            ))),
            Err(error) => Err(napi::Error::from_reason(error.message)),
        }
    }

    /// Run a synchronous core method (storage); returns its JSON result or throws its message.
    #[napi(catch_unwind)]
    pub fn call_sync(&self, method: String, args: String) -> napi::Result<String> {
        let args: Vec<Value> = serde_json::from_str(&args)
            .map_err(|e| napi::Error::from_reason(format!("Invalid arguments: {e}")))?;
        self.core
            .call_sync(&method, args)
            .map(|value| value.to_string())
            .map_err(|error| napi::Error::from_reason(error.message))
    }

    /// Cancel running work and release the core's files.
    #[napi]
    pub async fn close(&self) {
        self.core.close().await;
    }
}
