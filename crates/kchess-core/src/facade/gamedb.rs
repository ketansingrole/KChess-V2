//! The local game databases.

use kchess_contracts::check::{
    DatabaseId, DatabaseName, GameDbGameId, LichessMonth, MasterMonths, PgnFilePath,
};
use kchess_contracts::core_api::GameDbApi;
use kchess_contracts::error::Result;
use kchess_contracts::types::{
    DatabaseSource, GameDbImported, GameDbPage, GameDbQuery, GameDbStatus, GameSite,
    LichessArchive, LichessGames, PlayerGames,
};

use crate::core::Core;
use crate::gamedb::player;

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
        let packing = path.packing();
        self.gamedb
            .import(
                path.0,
                packing,
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

    async fn game_db_master_install(&self, months: Option<MasterMonths>) -> Result<GameDbImported> {
        self.gamedb
            .master_install(
                &self.master_games_url,
                months.map(|months| months.0),
                &*self.usage,
            )
            .await
    }

    async fn game_db_lichess_months(&self) -> Result<LichessArchive> {
        self.gamedb
            .lichess_months(&self.lichess_games_url, &*self.usage)
            .await
    }

    async fn game_db_lichess_size(&self, month: LichessMonth) -> Result<Option<u64>> {
        self.gamedb
            .lichess_size(&self.lichess_games_url, &month.0, &*self.usage)
            .await
    }

    async fn game_db_lichess_install(&self, request: LichessGames) -> Result<GameDbImported> {
        self.gamedb
            .lichess_install(&self.lichess_games_url, request, &*self.usage)
            .await
    }

    async fn game_db_player_install(&self, request: PlayerGames) -> Result<GameDbImported> {
        let now = crate::time::now_ms();
        let username = request.username.clone();
        match request.site {
            GameSite::Lichess => {
                let query = player::lichess_query(&request, now);
                let lichess = self.services.lichess();
                self.gamedb
                    .player_install(request, |mut sink| async move {
                        let cancel = sink.cancel();
                        lichess
                            .export_pgn(&username, &query, &cancel, &mut |text| sink.pgn(text))
                            .await?;
                        Ok(sink)
                    })
                    .await
            }
            GameSite::Chesscom => {
                let from = player::chesscom_from(&request, now);
                let wanted = request.clone();
                self.gamedb
                    .player_install(request, |mut sink| async move {
                        let cancel = sink.cancel();
                        let mut on_game = |game: &serde_json::Value| {
                            if !player::chesscom_keeps(&wanted, game) {
                                return Ok(());
                            }
                            match game.get("pgn").and_then(serde_json::Value::as_str) {
                                Some(pgn) => sink.game(pgn),
                                None => Ok(()),
                            }
                        };
                        self.chesscom
                            .player_games(&username, from.as_deref(), &cancel, &mut on_game)
                            .await?;
                        Ok(sink)
                    })
                    .await
            }
        }
    }
}
