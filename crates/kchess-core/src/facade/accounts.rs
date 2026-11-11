//! Lichess accounts.

use kchess_contracts::core_api::AccountsApi;
use serde_json::Value;

use super::args::text;
use super::core_data;
use crate::core::Core;
use crate::error::Result;
use crate::store::games::{self, as_json};

impl AccountsApi for Core {
    async fn add_account(&self, username: Value) -> Result<Value> {
        let name = text(&username);
        self.ensure_migrated().await?;
        let data = self
            .database
            .write_async(move |ctx| as_json(games::add_account(ctx.db, &name, false)?))
            .await?;
        core_data(&data)
    }

    // The account as its check normalizes it (trimmed): the raw text matched no stored
    // account, so `logout(" Alice ")` kept Alice signed in.
    async fn logout(&self, username: Value) -> Result<Value> {
        let username = text(&username);
        self.end_sessions(Some(&username)).await?;
        let lichess = self.services.lichess();
        lichess
            .revoke_logins(Some(std::slice::from_ref(&username)))
            .await;
        let data = self.logout_accounts(Some(&username)).await?;
        self.reviews.reviews_changed();
        core_data(&data)
    }

    async fn logout_all(&self) -> Result<Value> {
        self.end_sessions(None).await?;
        self.services.lichess().revoke_logins(None).await;
        let data = self.logout_accounts(None).await?;
        self.reviews.reviews_changed();
        core_data(&data)
    }

    async fn remove_account(&self, username: Value) -> Result<Value> {
        let name = text(&username);
        let one = std::slice::from_ref(&name);
        if self.end_sessions(Some(&name)).await?.is_empty() {
            self.services.lichess().cancel_account_syncs(one);
            self.reviews.discard_account_reviews(one);
            self.usage.flush().await;
            self.services.forget_usage(one);
            self.forget_tournaments_of(vec![name.clone()]).await?;
        }
        self.services.online().challenges().forget(&name);
        self.services.lichess().revoke_logins(Some(one)).await;
        self.ensure_migrated().await?;
        let account = name.clone();
        let data = self
            .database
            .write_async(move |ctx| as_json(games::remove_account(ctx.db, &account)?))
            .await?;
        self.reviews.reviews_changed();
        core_data(&data)
    }
}
