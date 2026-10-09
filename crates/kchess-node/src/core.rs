//! `NativeCore`: the Rust core's services for a Node or Electron host. Calls are asynchronous
//! (`call(method, argsJson) → Promise<resultJson>`); logs and events reach the host through
//! callbacks given to the constructor. Errors reject with the core's message; cancellations
//! carry `AbortError:` before it, which the host turns into an `AbortError`.

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
}

#[napi]
pub struct NativeCore {
    core: Arc<Core>,
}

#[napi]
impl NativeCore {
    #[napi(constructor)]
    pub fn new(options: NativeCoreOptions, log: Callback, emit: Callback) -> NativeCore {
        let config = Config {
            data_dir: PathBuf::from(options.data_dir),
            legacy_database_path: options.legacy_database_path.map(PathBuf::from),
        };
        NativeCore {
            core: Arc::new(Core::new(config, Arc::new(JsHost { log, emit }))),
        }
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
