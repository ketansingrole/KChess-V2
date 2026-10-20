//! The facade's Lichess accounts (`run_facade` arms, moved without changes).

use super::args::{arg, normal};
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
                let username = username_arg(args)?;
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
                let name = username_arg(args)?;
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

/// The account named by the first argument as the validator normalizes it (trimmed): the raw
/// text matched no stored account, so `logout(" Alice ")` kept Alice signed in.
fn username_arg(args: &[Value]) -> Result<String> {
    normal("assertUsername", &arg(args, 0))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::new("Invalid username."))
}
