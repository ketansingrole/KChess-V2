//! The facade's synced games, profiles and friends (`run_facade` arms, moved without changes).

use super::args::{arg, normal};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's games methods.
    pub(crate) async fn run_games(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "syncGames" => {
                let username = match args.first() {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertUsername", value)?,
                };
                let data = self.inner("lichess.syncGames", vec![username]).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "gamePage" => {
                let query = normal("assertGamePageQuery", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database
                    .call_async("store.games.gamePage", &[query])
                    .await
            }
            "gameLibraryOverview" => {
                self.ensure_migrated().await?;
                self.database
                    .call_async("store.games.gameLibraryOverview", &[])
                    .await
            }
            "insights" => {
                let query = normal("assertInsightsQuery", &arg(args, 0))?;
                self.database
                    .call_async("store.insights.insights", &[query])
                    .await
            }
            "gameRatingHistory" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database
                    .call_async("store.games.gameRatingHistory", &[account])
                    .await
            }
            "gamePgn" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let id = normal("assertGameId", &arg(args, 1))?;
                self.ensure_migrated().await?;
                self.database
                    .call_async("store.games.gamePgn", &[account, id])
                    .await
            }
            "cachedProfile" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.cachedProfile", vec![name]).await
            }
            "profile" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.profile", vec![name]).await
            }
            "ratingHistory" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.ratingHistory", vec![name]).await
            }
            "connectLichess" => {
                // `oauthLook` falls back to the safe default colours for anything it cannot read.
                let look = match kchess_domain::records::call("oauthLook", &[arg(args, 0)]) {
                    Some(Ok(look)) => look,
                    _ => Value::Null,
                };
                self.inner("lichess.connectLichess", vec![look]).await
            }
            "stayConnected" => {
                let name = normal("assertOptionalAccount", &arg(args, 0))?;
                let wanted = name.as_str().unwrap_or_default().to_string();
                let connected = self.is_connected(&wanted).await?;
                self.inner(
                    "online.stayConnected",
                    vec![json!(if connected { wanted } else { String::new() })],
                )
                .await
            }
            "clearAccountData" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.cancelAccountSyncs", vec![json!([name])])
                    .await?;
                self.reviews
                    .discard_account_reviews(&[name.as_str().unwrap_or_default().to_string()]);
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call_async("store.games.clearAccountData", std::slice::from_ref(&name))
                    .await?;
                self.inner("lichess.forgetProfile", vec![name]).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "following" => self.inner("lichess.followedUsers", vec![]).await,
            "addFriends" => {
                let names = normal("assertFriendList", &arg(args, 0))?;
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call_async("store.games.addFriends", std::slice::from_ref(&names))
                    .await?;
                self.inner("lichess.primeProfiles", vec![names]).await?;
                Ok(data)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
