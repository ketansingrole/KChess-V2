//! The core as hosts drive it: one asynchronous `call(method, args)` with JSON arguments
//! and results, and events through the host.

use serde_json::Value;
use std::sync::Arc;

use crate::error::{CoreError, Result};
use crate::host::{Config, Host};
use crate::puzzles::PuzzleService;

pub struct Core {
    config: Config,
    host: Arc<dyn Host>,
    puzzles: PuzzleService,
    /// `kchess.db`, opened on first use.
    db: std::sync::Mutex<Option<rusqlite::Connection>>,
}

fn arg<T: serde::de::DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T> {
    serde_json::from_value(args.get(i).cloned().unwrap_or(Value::Null))
        .map_err(|e| CoreError::new(format!("Invalid {name}: {e}")))
}

fn json<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| CoreError::new(e.to_string()))
}

impl Core {
    pub fn new(config: Config, host: Arc<dyn Host>) -> Core {
        Core {
            puzzles: PuzzleService::new(config.clone(), Arc::clone(&host)),
            config,
            host,
            db: std::sync::Mutex::new(None),
        }
    }

    /// Run one method. Unknown methods and malformed arguments are errors.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        match method {
            "puzzles.status" => json(self.puzzles.status().await?),
            "puzzles.install" => {
                let url: String = arg(&args, 0, "url")?;
                json(self.puzzles.install(url).await?)
            }
            "puzzles.cancel" => {
                self.puzzles.cancel();
                Ok(Value::Null)
            }
            "puzzles.delete" => json(self.puzzles.delete().await?),
            "puzzles.query" => json(self.puzzles.query(arg(&args, 0, "query")?).await?),
            "puzzles.ladder" => json(self.puzzles.ladder(arg(&args, 0, "query")?).await?),
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }

    /// Run one synchronous method: storage that TypeScript callers still use synchronously.
    /// Unknown methods and malformed arguments are errors.
    pub fn call_sync(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        if !method.starts_with("store.") {
            return Err(CoreError::new(format!("Unknown core method {method}.")));
        }
        let mut db = self
            .db
            .lock()
            .map_err(|_| CoreError::new("The database is unavailable."))?;
        if method == "store.debug.historical" && crate::store::debug::enabled() {
            // Test-only: an earlier release's database, written before the store opens the file.
            if db.is_some() {
                return Err(CoreError::new("The database is already open."));
            }
            let path = self.config.data_dir.join("kchess.db");
            return crate::store::debug::historical(&path, &args);
        }
        if db.is_none() {
            let path = self.config.data_dir.join("kchess.db");
            *db = Some(crate::store::open(&path, self.host.as_ref())?);
        }
        match db.as_ref() {
            Some(connection) => {
                let ctx = crate::store::StoreContext {
                    db: connection,
                    host: self.host.as_ref(),
                    config: &self.config,
                };
                crate::store::call(&ctx, method, &args)
            }
            None => Err(CoreError::new("The database is unavailable.")),
        }
    }

    /// Cancel running work and release files; later calls fail.
    pub async fn close(&self) {
        self.puzzles.close().await;
        if let Ok(mut db) = self.db.lock() {
            db.take();
        }
    }
}
