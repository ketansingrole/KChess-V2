//! The core as hosts drive it: one asynchronous `call(method, args)` with JSON arguments
//! and results, and events through the host.

use serde_json::Value;
use std::sync::Arc;

use crate::error::{CoreError, Result};
use crate::host::{Config, Host};
use crate::puzzles::PuzzleService;

pub struct Core {
    puzzles: PuzzleService,
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
            puzzles: PuzzleService::new(config, host),
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

    /// Cancel running work and release files; later calls fail.
    pub async fn close(&self) {
        self.puzzles.close().await;
    }
}
