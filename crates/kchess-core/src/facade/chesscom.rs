//! Chess.com players: their games sync like Lichess games (`crate::chesscom`).

use kchess_contracts::core_api::ChesscomApi;
use serde_json::Value;

use super::args::text;
use crate::core::Core;
use crate::error::Result;
use crate::store::chesscom;
use crate::store::games::as_json;

impl ChesscomApi for Core {
    async fn chesscom_accounts(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database
            .read_async(|ctx| as_json(chesscom::accounts(ctx.db)?))
            .await
    }

    async fn add_chesscom_account(&self, username: Value) -> Result<Value> {
        self.ensure_migrated().await?;
        let result = self.chesscom.add(&text(&username)).await;
        self.reviews.reviews_changed();
        result
    }

    async fn remove_chesscom_account(&self, username: Value) -> Result<Value> {
        self.ensure_migrated().await?;
        let result = self.chesscom.remove(&text(&username)).await;
        self.reviews.reviews_changed();
        result
    }

    async fn sync_chesscom_games(&self, username: Option<Value>) -> Result<Value> {
        self.ensure_migrated().await?;
        let wanted = username.as_ref().map(text);
        let result = self.chesscom.sync(wanted.as_deref()).await;
        self.reviews.reviews_changed();
        result
    }
}
