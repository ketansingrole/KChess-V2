//! The local game databases.

use kchess_contracts::check::{DatabaseId, DatabaseName, GameDbGameId, PgnFilePath};
use kchess_contracts::core_api::GameDbApi;
use kchess_contracts::error::Result;
use kchess_contracts::types::{
    DatabaseSource, GameDbImported, GameDbPage, GameDbQuery, GameDbStatus,
};

use crate::core::Core;

impl GameDbApi for Core {
    async fn game_db_status(&self) -> Result<GameDbStatus> {
        self.gamedb.status().await
    }

    async fn game_db_import(
        &self,
        path: PgnFilePath,
        into: Option<DatabaseId>,
        name: Option<DatabaseName>,
    ) -> Result<GameDbImported> {
        let compressed = path.compressed();
        self.gamedb
            .import(
                path.0,
                compressed,
                into.map(|id| id.0),
                name.map(|name| name.0),
                DatabaseSource::Imported,
            )
            .await
    }

    async fn game_db_cancel(&self) -> Result<()> {
        self.gamedb.cancel();
        Ok(())
    }

    async fn game_db_delete(&self, database: DatabaseId) -> Result<GameDbStatus> {
        self.gamedb.delete(&database.0).await
    }

    async fn game_db_rename(
        &self,
        database: DatabaseId,
        name: DatabaseName,
    ) -> Result<GameDbStatus> {
        self.gamedb.rename(&database.0, name.0).await
    }

    async fn game_db_games(&self, query: GameDbQuery) -> Result<GameDbPage> {
        self.gamedb.games(query).await
    }

    async fn game_db_game(&self, database: DatabaseId, game: GameDbGameId) -> Result<String> {
        self.gamedb.game(&database.0, game.0).await
    }
}
