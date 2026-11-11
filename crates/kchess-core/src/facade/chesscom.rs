//! Chess.com players: their games sync like Lichess games (`crate::chesscom`).

use kchess_contracts::check::ChesscomUsername;
use kchess_contracts::core_api::ChesscomApi;
use kchess_contracts::types::ChesscomAccount;

use crate::core::Core;
use crate::error::Result;
use crate::store::chesscom;

impl ChesscomApi for Core {
    async fn chesscom_accounts(&self) -> Result<Vec<ChesscomAccount>> {
        self.database
            .read_async(|ctx| chesscom::accounts(ctx.db))
            .await
    }

    async fn add_chesscom_account(
        &self,
        username: ChesscomUsername,
    ) -> Result<Vec<ChesscomAccount>> {
        self.chesscom.add(&username.0).await
    }

    async fn remove_chesscom_account(
        &self,
        username: ChesscomUsername,
    ) -> Result<Vec<ChesscomAccount>> {
        self.chesscom.remove(&username.0).await
    }

    async fn sync_chesscom_games(
        &self,
        username: Option<ChesscomUsername>,
    ) -> Result<Vec<ChesscomAccount>> {
        self.chesscom
            .sync(username.as_ref().map(|name| name.0.as_str()))
            .await
    }
}
