//! The facade's usage and the lifecycle (`run_facade` arms, moved without changes).

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use serde_json::Value;
use std::sync::Arc;

impl Core {
    /// Answers the facade's system methods.
    pub(crate) async fn run_system(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "usage" => self.usage.report().await,
            "resetUsage" => self.usage.reset().await,
            "trustEnginePath" => self.call_sync(method, args.to_vec()),
            "customThemes" => {
                let (host, data_dir) = (Arc::clone(&self.host), self.data_dir());
                tokio::task::spawn_blocking(move || crate::themes::custom_themes(&*host, &data_dir))
                    .await
                    .map_err(|_| CoreError::new("Custom themes could not be read."))?
            }
            "voiceHistoryDocument" => {
                self.database
                    .read_async(|ctx| {
                        crate::store::voice_log::voice_history_document(ctx.host, ctx.db)
                            .map(Value::String)
                    })
                    .await
            }
            "suspend" => {
                if let Err(cause) = self.dispatch("puzzles.cancel", vec![]).await {
                    self.host.log(
                        Level::Warn,
                        "puzzles",
                        &format!("Puzzle download cancel failed: {}", cause.message),
                    );
                }
                self.services.spectator().stop();
                self.services.online().close();
                self.search.stop_engine(true);
                self.analysis.stop_analysis(true);
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
