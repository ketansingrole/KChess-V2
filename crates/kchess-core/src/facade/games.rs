//! Synced games, profiles and friends.

use std::sync::Arc;

use kchess_contracts::core_api::GamesApi;
use serde_json::{Value, json};

use super::args::{text, to_json};
use super::core_data;
use crate::core::Core;
use crate::error::Result;
use crate::store::games::{self, as_json};
use crate::store::insights;
use crate::time::now_ms;

impl GamesApi for Core {
    async fn sync_games(&self, username: Option<Value>) -> Result<Value> {
        let username = username.as_ref().map(text);
        self.services
            .lichess()
            .sync_games(username.as_deref())
            .await?;
        let data = self.app_data().await?;
        core_data(&data)
    }

    async fn game_page(&self, query: Value) -> Result<Value> {
        self.database
            .read_async(move |ctx| games::game_page(ctx, query))
            .await
    }

    async fn game_library_overview(&self) -> Result<Value> {
        self.database.read_async(games::game_library_overview).await
    }

    async fn insights(&self, query: Value) -> Result<Value> {
        let now = now_ms() as f64;
        self.database
            .read_async(move |ctx| insights::insights(ctx.host, ctx.db, &query, now))
            .await
    }

    async fn game_rating_history(&self, account: Value) -> Result<Value> {
        let account = text(&account);
        self.database
            .read_async(move |ctx| as_json(games::rating_history(ctx.db, &account)?))
            .await
    }

    async fn game_pgn(&self, account: Value, id: Value) -> Result<Value> {
        let (account, id) = (text(&account), text(&id));
        self.database
            .read_async(move |ctx| Ok(json!(games::game_pgn(ctx.db, &account, &id)?)))
            .await
    }

    async fn cached_profile(&self, username: Value) -> Result<Value> {
        let cached = self
            .services
            .lichess()
            .cached_profile(&text(&username))
            .await;
        to_json(cached.map(|cached| {
            let mut object = serde_json::Map::new();
            object.insert("profile".into(), cached.profile.unwrap_or(Value::Null));
            object.insert(
                "ratingHistory".into(),
                cached.rating_history.unwrap_or(Value::Null),
            );
            if let Some(at) = cached.profile_fetched_at {
                object.insert("profileFetchedAt".into(), json!(at));
            }
            Value::Object(object)
        }))
    }

    async fn profile(&self, username: Value) -> Result<Value> {
        self.services.lichess().profile(&text(&username)).await
    }

    async fn rating_history(&self, username: Value) -> Result<Value> {
        self.services
            .lichess()
            .rating_history(&text(&username))
            .await
    }

    async fn connect_lichess(&self, look: Option<Value>) -> Result<Value> {
        // An unreadable look falls back to the safe default colours.
        let look = kchess_domain::records::oauth::look(&look.unwrap_or(Value::Null))
            .filter(|look| !look.is_null());
        // The window is focused when the browser returns.
        let capabilities = Arc::clone(&self.capabilities);
        let focus = move || {
            let capabilities = Arc::clone(&capabilities);
            tokio::spawn(async move {
                let _ = capabilities.focus().await;
            });
        };
        let connected = self
            .services
            .lichess()
            .connect_lichess(look.as_ref(), Some(&focus))
            .await?;
        let data = self.app_data().await?;
        Ok(json!({ "data": core_data(&data)?, "username": connected.username }))
    }

    async fn stay_connected(&self, account: Value) -> Result<Value> {
        let wanted = text(&account);
        let connected = self.is_connected(&wanted).await?;
        self.services
            .online()
            .stay_connected(if connected { &wanted } else { "" });
        Ok(Value::Null)
    }

    async fn clear_account_data(&self, username: Value) -> Result<Value> {
        let account = text(&username);
        let lichess = self.services.lichess();
        lichess.cancel_account_syncs(std::slice::from_ref(&account));
        self.reviews
            .discard_account_reviews(std::slice::from_ref(&account));
        let cleared = account.clone();
        let data = self
            .database
            .write_async(move |ctx| as_json(games::clear_account_data(ctx.db, &cleared)?))
            .await?;
        lichess.forget_profile(&account);
        core_data(&data)
    }

    async fn following(&self) -> Result<Value> {
        to_json(self.services.lichess().followed_users().await)
    }

    async fn add_friends(&self, usernames: Value) -> Result<Value> {
        let list = usernames.as_array().cloned().unwrap_or_default();
        let data = self
            .database
            .write_async(move |ctx| as_json(games::add_friends(ctx.db, &list)?))
            .await?;
        let names: Vec<String> = usernames
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(text)
            .collect();
        self.services.lichess().prime_profiles(&names).await?;
        core_data(&data)
    }
}
