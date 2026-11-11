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
        let data = self
            .database
            .write_async(move |ctx| as_json(games::add_account(ctx.db, &name, false)?))
            .await?;
        core_data(&data)
    }

    // The account as its check normalizes it (trimmed): the raw text matched no stored
    // account, so `logout(" Alice ")` kept Alice signed in.
    async fn logout(&self, username: Value) -> Result<Value> {
        let data = self.sign_out(Some(text(&username))).await?;
        core_data(&data)
    }

    async fn logout_all(&self) -> Result<Value> {
        let data = self.sign_out(None).await?;
        core_data(&data)
    }

    async fn remove_account(&self, username: Value) -> Result<Value> {
        let name = text(&username);
        let one = std::slice::from_ref(&name);
        if self.end_sessions(Some(&name)).await?.is_empty() {
            self.end_account_work(one, false).await?;
        }
        self.services.lichess().revoke_logins(Some(one)).await;
        let account = name.clone();
        let data = self
            .database
            .write_async(move |ctx| as_json(games::remove_account(ctx.db, &account)?))
            .await?;
        core_data(&data)
    }
}
