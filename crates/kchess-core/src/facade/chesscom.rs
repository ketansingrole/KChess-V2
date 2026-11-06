//! The facade's Chess.com players: their games sync like Lichess games (`crate::chesscom`).

use super::args::{arg, normal};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::Value;

impl Core {
    /// Answers the facade's Chess.com methods.
    pub(crate) async fn run_chesscom(&self, method: &str, args: &[Value]) -> Result<Value> {
        self.ensure_migrated().await?;
        let name = |value: &Value| -> Result<String> {
            Ok(normal("assertChesscomUsername", value)?
                .as_str()
                .unwrap_or_default()
                .to_string())
        };
        match method {
            "chesscomAccounts" => {
                self.database
                    .call_async("store.chesscom.accounts", &[])
                    .await
            }
            "addChesscomAccount" => {
                let result = self.chesscom.add(&name(&arg(args, 0))?).await;
                self.reviews.reviews_changed();
                result
            }
            "removeChesscomAccount" => {
                let result = self.chesscom.remove(&name(&arg(args, 0))?).await;
                self.reviews.reviews_changed();
                result
            }
            "syncChesscomGames" => {
                let wanted = match args.first() {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(name(value)?),
                };
                let result = self.chesscom.sync(wanted.as_deref()).await;
                self.reviews.reviews_changed();
                result
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
