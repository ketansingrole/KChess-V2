//! The facade's synced games, profiles and friends.

use super::args::{arg, normal, now_ms, text};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::store::games::{self, as_json};
use crate::store::insights;
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
                    .read_async(move |ctx| games::game_page(ctx, query))
                    .await
            }
            "gameLibraryOverview" => {
                self.ensure_migrated().await?;
                self.database.read_async(games::game_library_overview).await
            }
            "insights" => {
                let query = normal("assertInsightsQuery", &arg(args, 0))?;
                let now = now_ms() as f64;
                self.database
                    .read_async(move |ctx| insights::insights(ctx.host, ctx.db, &query, now))
                    .await
            }
            "gameRatingHistory" => {
                let account = text(&normal("assertGameAccount", &arg(args, 0))?);
                self.ensure_migrated().await?;
                self.database
                    .read_async(move |ctx| as_json(games::rating_history(ctx.db, &account)?))
                    .await
            }
            "gamePgn" => {
                let account = text(&normal("assertGameAccount", &arg(args, 0))?);
                let id = text(&normal("assertGameId", &arg(args, 1))?);
                self.ensure_migrated().await?;
                self.database
                    .read_async(move |ctx| Ok(json!(games::game_pgn(ctx.db, &account, &id)?)))
                    .await
            }
            _ => self.run_game_accounts(method, args).await,
        }
    }

    /// Answers the facade's Lichess profile and account methods.
    async fn run_game_accounts(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
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
                let account = text(&name);
                let data = self
                    .database
                    .write_async(move |ctx| as_json(games::clear_account_data(ctx.db, &account)?))
                    .await?;
                self.inner("lichess.forgetProfile", vec![name]).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "following" => self.inner("lichess.followedUsers", vec![]).await,
            "addFriends" => {
                let names = normal("assertFriendList", &arg(args, 0))?;
                self.ensure_migrated().await?;
                let list = names.as_array().cloned().unwrap_or_default();
                let data = self
                    .database
                    .write_async(move |ctx| as_json(games::add_friends(ctx.db, &list)?))
                    .await?;
                self.inner("lichess.primeProfiles", vec![names]).await?;
                Ok(data)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
