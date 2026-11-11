//! The library: studies, played games, drills, sessions and repertoire notes.

use kchess_contracts::check::{
    ArchivedGame, LibraryId, MissCounts, PgnText, Position, RepertoireKey, RepertoireLine,
    ReviewKey, SessionObject, Side, ViewPreferenceKey,
};
use kchess_contracts::core_api::LibraryApi;
use kchess_contracts::types::{
    Drill, JoinedTournament, Library, MistakeGame, MistakesAdded, SessionKind, StudyCommand,
    StudyCommandResult,
};
use kchess_domain::misc::documents;
use kchess_domain::misc::value::J;
use serde_json::{Map, Value};

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::store::{library, library_import, view_preferences};
use crate::time::now_ms;

impl LibraryApi for Core {
    async fn library(&self) -> Result<Library> {
        let now = now_ms() as f64;
        self.database
            .read_async(move |ctx| library_import::cached_library(ctx, now))
            .await
    }

    async fn study_command(
        &self,
        mut command: StudyCommand,
        incremental: Option<bool>,
        based_on: Option<PgnText>,
    ) -> Result<StudyCommandResult> {
        // A restored study is read as the rules read a stored one.
        if let StudyCommand::Restore { study } = &mut command {
            *study = kchess_domain::library::decode_study(study)
                .ok_or_else(|| CoreError::new("That study cannot be restored."))?;
        }
        let (incremental, now) = (incremental == Some(true), now_ms() as f64);
        self.database
            .write_async(move |ctx| {
                let based_on = based_on.as_ref().map(|pgn| pgn.0.as_str());
                library::run_study_command(ctx.db, command, now, incremental, based_on)
            })
            .await
    }

    async fn save_archived_game(&self, game: ArchivedGame) -> Result<()> {
        let game = kchess_domain::library::decode_archived_game(&game.0)
            .ok_or_else(|| CoreError::new("This game cannot be saved."))?;
        self.database
            .write_async(move |ctx| library::write_game(ctx.db, &game))
            .await
    }

    async fn remove_archived_game(&self, id: LibraryId) -> Result<()> {
        self.database
            .write_async(move |ctx| library::remove_archived_game(ctx.db, &id.0))
            .await
    }

    async fn add_mistakes(
        &self,
        review_key: ReviewKey,
        color: Option<Side>,
    ) -> Result<MistakesAdded> {
        let (color, now) = (color.map(|side| side.0), now_ms() as f64);
        self.database
            .write_async(move |ctx| {
                library::add_mistakes(ctx.host, ctx.db, &review_key.0, color, now)
            })
            .await
    }

    async fn answer_mistake(&self, id: LibraryId, solved: bool) -> Result<Vec<Drill>> {
        let now = now_ms() as f64;
        self.database
            .write_async(move |ctx| library::answer_mistake(ctx.db, &id.0, solved, now))
            .await
    }

    async fn mistake_games(&self) -> Result<Vec<MistakeGame>> {
        self.database
            .read_async(|ctx| library::mistake_games(ctx.host, ctx.db))
            .await
    }

    async fn save_session(&self, kind: SessionKind, session: SessionObject) -> Result<()> {
        let checked =
            documents::assert_session(vec![J::Str(kind.as_str().into()), J::from(&session.0)]);
        let value = checked.map(J::into_value).map_err(CoreError::new)?;
        self.database
            .write_async(move |ctx| library::save_session(ctx.db, kind, value))
            .await
    }

    async fn joined_tournaments(&self) -> Result<Vec<JoinedTournament>> {
        let now = now_ms() as f64;
        self.database
            .read_async(move |ctx| library::joined_tournaments(ctx.host, ctx.db, now))
            .await
    }

    async fn session(&self, kind: SessionKind) -> Result<Option<Value>> {
        self.database
            .read_async(move |ctx| library::session(ctx.host, ctx.db, kind))
            .await
    }

    async fn mistakes(&self) -> Result<Vec<Drill>> {
        self.database
            .read_async(|ctx| library_import::read_mistakes(ctx.db))
            .await
    }

    async fn record_repertoire_miss(&self, key: RepertoireKey, fen: Position) -> Result<Value> {
        self.database
            .write_async(move |ctx| {
                library::record_repertoire_miss(ctx.host, ctx.db, &key.0, &fen.0)
            })
            .await
    }

    async fn clear_repertoire_misses(&self, key: RepertoireKey) -> Result<Value> {
        self.database
            .write_async(move |ctx| library::clear_repertoire_misses(ctx.host, ctx.db, &key.0))
            .await
    }

    async fn restore_repertoire_misses(
        &self,
        key: RepertoireKey,
        misses: MissCounts,
    ) -> Result<Value> {
        self.database
            .write_async(move |ctx| {
                library::restore_repertoire_misses(ctx.host, ctx.db, &key.0, &misses.0)
            })
            .await
    }

    async fn review_repertoire_line(
        &self,
        key: RepertoireKey,
        line: RepertoireLine,
        correct: bool,
    ) -> Result<Value> {
        let now = now_ms() as f64;
        self.database
            .write_async(move |ctx| {
                library::review_repertoire_line(ctx.host, ctx.db, &key.0, &line.0, correct, now)
            })
            .await
    }

    async fn view_preferences(&self) -> Result<Map<String, Value>> {
        self.database.read_async(view_preferences::all).await
    }

    async fn set_view_preference(
        &self,
        key: ViewPreferenceKey,
        value: Option<Value>,
    ) -> Result<()> {
        let value = value.unwrap_or(Value::Null);
        self.database
            .write_async(move |ctx| view_preferences::set(ctx, &key.0, &value))
            .await
    }
}
