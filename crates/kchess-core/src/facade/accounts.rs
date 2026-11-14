//! Lichess accounts.

use kchess_contracts::check::Username;
use kchess_contracts::core_api::AccountsApi;
use kchess_contracts::types::AppData;

use crate::core::Core;
use crate::store::games;
use kchess_contracts::error::Result;

impl AccountsApi for Core {
    async fn add_account(&self, username: Username) -> Result<AppData> {
        self.database
            .write_async(move |ctx| games::add_account(ctx.db, &username.0, false))
            .await
    }

    // The account as its check normalizes it (trimmed): the raw text matched no stored
    // account, so `logout(" Alice ")` kept Alice signed in.
    async fn logout(&self, username: Username) -> Result<AppData> {
        self.sign_out(Some(username.0)).await
    }

    async fn logout_all(&self) -> Result<AppData> {
        self.sign_out(None).await
    }

    async fn remove_account(&self, username: Username) -> Result<AppData> {
        let name = username.0;
        let one = std::slice::from_ref(&name);
        if self.end_sessions(Some(&name)).await?.is_empty() {
            self.end_account_work(one, false).await?;
        }
        self.services.lichess().revoke_logins(Some(one)).await;
        self.database
            .write_async(move |ctx| games::remove_account(ctx.db, &name))
            .await
    }
}
