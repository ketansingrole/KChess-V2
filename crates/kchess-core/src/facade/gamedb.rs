//! The local game databases.

use kchess_contracts::check::{
    DatabaseId, DatabaseName, Fen, GameDbGameId, GameDbGameIds, LichessMonth, MasterMonths, Moves,
    PgnFilePath, PgnSavePath, PgnText, PlayerName, PlayerNames,
};
use kchess_contracts::core_api::GameDbApi;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    DatabaseSource, GameDbImported, GameDbNameGroup, GameDbNovelty, GameDbOpeningReport,
    GameDbPage, GameDbPlayerReport, GameDbQuery, GameDbSaved, GameDbStatus, GameSite,
    LichessArchive, LichessGames, PlayerGames,
};

use crate::core::Core;
use crate::gamedb::edit::{self, Selection};
use crate::gamedb::novelty;
use crate::gamedb::player;

/// The games an edit acts on: those listed, or those a search finds, never both or neither.
fn selection(games: Option<GameDbGameIds>, search: Option<GameDbQuery>) -> Result<Selection> {
    match (games, search) {
        (Some(games), None) => Ok(Selection::Games(games.0)),
        (None, Some(search)) => Ok(Selection::Search(Box::new(search))),
        _ => Err(CoreError::new("Choose the games, or a search.")),
    }
}

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

    async fn game_db_add_games(
        &self,
        pgn: PgnText,
        into: Option<DatabaseId>,
        name: Option<DatabaseName>,
    ) -> Result<GameDbSaved> {
        let into = match (into, name) {
            (Some(into), None) => edit::Into::Existing(into.0),
            (None, Some(name)) => edit::Into::New(name.0),
            _ => return Err(CoreError::new("Choose a database, or name a new one.")),
        };
        self.gamedb.add_games(into, pgn.0).await
    }

    async fn game_db_replace_game(
        &self,
        database: DatabaseId,
        game: GameDbGameId,
        pgn: PgnText,
    ) -> Result<GameDbSaved> {
        self.gamedb.replace_game(&database.0, game.0, pgn.0).await
    }

    async fn game_db_delete_games(
        &self,
        database: DatabaseId,
        games: Option<GameDbGameIds>,
        search: Option<GameDbQuery>,
    ) -> Result<u64> {
        let selection = selection(games, search)?;
        self.gamedb.delete_games(&database.0, selection).await
    }

    async fn game_db_opening_report(&self, fen: Fen) -> Result<GameDbOpeningReport> {
        self.gamedb.opening_report(fen.0).await
    }

    async fn game_db_event(
        &self,
        database: DatabaseId,
        game: GameDbGameId,
    ) -> Result<kchess_contracts::types::GameDbEventReport> {
        self.gamedb.event_report(&database.0, game.0).await
    }

    async fn game_db_player(
        &self,
        database: DatabaseId,
        name: PlayerName,
    ) -> Result<GameDbPlayerReport> {
        self.gamedb.player_report(&database.0, name.0).await
    }

    async fn game_db_duplicates(&self, database: DatabaseId) -> Result<u64> {
        self.gamedb.duplicates(&database.0).await
    }

    async fn game_db_remove_duplicates(&self, database: DatabaseId) -> Result<u64> {
        self.gamedb
            .delete_games(&database.0, edit::Selection::Duplicates)
            .await
    }

    async fn game_db_name_groups(&self, database: DatabaseId) -> Result<Vec<GameDbNameGroup>> {
        self.gamedb.name_groups(&database.0).await
    }

    async fn game_db_merge_names(
        &self,
        database: DatabaseId,
        into: PlayerName,
        names: PlayerNames,
    ) -> Result<u64> {
        self.gamedb.merge_names(&database.0, into.0, names.0).await
    }

    async fn game_db_copy_games(
        &self,
        database: DatabaseId,
        games: Option<GameDbGameIds>,
        search: Option<GameDbQuery>,
        into: Option<DatabaseId>,
        name: Option<DatabaseName>,
    ) -> Result<GameDbImported> {
        let selection = selection(games, search)?;
        self.gamedb
            .copy_games(
                &database.0,
                selection,
                into.map(|id| id.0),
                name.map(|name| name.0),
            )
            .await
    }

    async fn game_db_export(
        &self,
        database: DatabaseId,
        path: PgnSavePath,
        games: Option<GameDbGameIds>,
        search: Option<GameDbQuery>,
    ) -> Result<u64> {
        let selection = selection(games, search)?;
        self.gamedb.export(&database.0, selection, path.0).await
    }

    async fn game_db_repertoire(
        &self,
        steps: kchess_contracts::check::RepertoireSteps,
    ) -> Result<Vec<kchess_contracts::types::GameDbRepertoireCheck>> {
        self.gamedb.repertoire(steps.0).await
    }

    async fn game_db_repertoire_games(
        &self,
        steps: kchess_contracts::check::RepertoireSteps,
        color: kchess_contracts::check::Side,
    ) -> Result<kchess_contracts::types::GameDbRepertoireGames> {
        let names = self
            .database
            .read_async(|ctx| crate::store::own_games::your_names(ctx.db))
            .await?;
        self.gamedb.repertoire_games(steps.0, color.0, names).await
    }

    async fn game_db_repertoire_player(
        &self,
        steps: kchess_contracts::check::RepertoireSteps,
        color: kchess_contracts::check::Side,
        database: DatabaseId,
    ) -> Result<kchess_contracts::types::GameDbRepertoireGames> {
        self.gamedb
            .repertoire_player(steps.0, color.0, database.0)
            .await
    }

    async fn game_db_repertoire_build(
        &self,
        color: kchess_contracts::check::Side,
        database: DatabaseId,
    ) -> Result<kchess_contracts::types::GameDbRepertoireBuilt> {
        let names = self
            .database
            .read_async(|ctx| crate::store::own_games::your_names(ctx.db))
            .await?;
        self.gamedb
            .repertoire_build(color.0, database.0, names)
            .await
    }

    async fn game_db_novelty(
        &self,
        fen: Fen,
        moves: Moves,
        white: Option<PlayerName>,
        black: Option<PlayerName>,
        database: Option<DatabaseId>,
        game: Option<GameDbGameId>,
    ) -> Result<GameDbNovelty> {
        let line = novelty::Line {
            fen: fen.0,
            moves: moves.0,
            white: white.map(|name| name.0),
            black: black.map(|name| name.0),
            from: database
                .zip(game)
                .map(|(database, game)| (database.0, game.0)),
        };
        self.gamedb.novelty(line).await
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

    async fn game_db_tcec_install(&self) -> Result<GameDbImported> {
        let (latest, downloads) = &self.tcec_urls;
        self.gamedb
            .tcec_install(latest, downloads, &*self.usage)
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
