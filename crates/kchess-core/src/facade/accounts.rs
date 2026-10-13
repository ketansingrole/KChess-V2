//! The facade's Lichess accounts (`run_facade` arms, moved without changes).

use super::args::{arg, normal, text_arg};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's accounts methods.
    pub(crate) async fn run_accounts(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "addAccount" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database
                    .call_async("store.games.addAccount", &[name, json!(false)])
                    .await
            }
            "logout" => {
                let username = text_arg(args, 0);
                self.end_sessions(Some(&username)).await?;
                let data = self.logout_accounts(Some(&username)).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "logoutAll" => {
                self.end_sessions(None).await?;
                let data = self.logout_accounts(None).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "removeAccount" => {
                let name = text_arg(args, 0);
                if self.end_sessions(Some(&name)).await?.is_empty() {
                    self.inner("lichess.cancelAccountSyncs", vec![json!([name])])
                        .await?;
                    self.reviews
                        .discard_account_reviews(std::slice::from_ref(&name));
                    self.usage.flush().await;
                    self.services.forget_usage(std::slice::from_ref(&name));
                    self.inner("store.library.forgetTournamentsOf", vec![json!([name])])
                        .await?;
                }
                self.inner("challenges.forget", vec![json!(name)]).await?;
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call_async("store.games.removeAccount", &[json!(name)])
                    .await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
