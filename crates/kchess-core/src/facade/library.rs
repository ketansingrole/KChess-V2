//! The library: studies, played games, drills, sessions and repertoire notes.

use kchess_contracts::check::LibraryId;
use kchess_contracts::core_api::LibraryApi;
use kchess_domain::misc::documents;
use kchess_domain::misc::value::J;
use serde_json::Value;

use super::args::text;
use super::{archived_game, study_command};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::store::{library, library_import, view_preferences};
use crate::time::now_ms;

impl LibraryApi for Core {
    async fn library(&self) -> Result<Value> {
        let now = now_ms() as f64;
        self.database
            .read_async(move |ctx| library_import::cached_library(ctx, now))
            .await
    }

    async fn study_command(
        &self,
        command: Value,
        incremental: Option<Value>,
        based_on: Option<Value>,
    ) -> Result<Value> {
        let (command, now) = (study_command(&command)?, now_ms() as f64);
        let incremental = incremental == Some(Value::Bool(true));
        let based_on = based_on
            .as_ref()
            .and_then(Value::as_str)
            .map(str::to_string);
        self.database
            .write_async(move |ctx| {
                library::run_study_command(ctx.db, command, now, incremental, based_on.as_deref())
            })
            .await
    }

    async fn save_archived_game(&self, game: Value) -> Result<Value> {
        let game = archived_game(&game)?;
        self.database
            .write_async(move |ctx| library::write_game(ctx.db, &game))
            .await?;
        Ok(Value::Null)
    }

    async fn remove_archived_game(&self, id: LibraryId) -> Result<()> {
        self.database
            .write_async(move |ctx| library::remove_archived_game(ctx.db, &id.0))
            .await
    }

    async fn add_mistakes(&self, review_key: Value, color: Option<Value>) -> Result<Value> {
        let (key, now) = (text(&review_key), now_ms() as f64);
        let color = color.as_ref().and_then(Value::as_str).map(str::to_string);
        self.database
            .write_async(move |ctx| {
                library::add_mistakes(ctx.host, ctx.db, &key, color.as_deref(), now)
            })
            .await
    }

    async fn answer_mistake(&self, id: Value, solved: Value) -> Result<Value> {
        let (id, solved, now) = (text(&id), solved == Value::Bool(true), now_ms() as f64);
        self.database
            .write_async(move |ctx| library::answer_mistake(ctx.db, &id, solved, now))
            .await
    }

    async fn mistake_games(&self) -> Result<Value> {
        self.database
            .read_async(|ctx| library::mistake_games(ctx.host, ctx.db))
            .await
    }

    async fn save_session(&self, kind: Value, session: Value) -> Result<Value> {
        let checked = documents::assert_session(vec![J::from(&kind), J::from(&session)]);
        let value = checked.map(J::into_value).map_err(CoreError::new)?;
        let kind = text(&kind);
        self.database
            .write_async(move |ctx| library::save_session(ctx.db, &kind, value))
            .await
    }

    async fn joined_tournaments(&self) -> Result<Value> {
        let now = now_ms() as f64;
        self.database
            .read_async(move |ctx| library::joined_tournaments(ctx.host, ctx.db, now))
            .await
    }

    async fn session(&self, kind: Value) -> Result<Value> {
        let kind = text(&kind);
        self.database
            .read_async(move |ctx| library::session(ctx.host, ctx.db, &kind))
            .await
    }

    async fn mistakes(&self) -> Result<Value> {
        self.database
            .read_async(|ctx| library_import::read_mistakes(ctx.db).map(Value::Array))
            .await
    }

    async fn record_repertoire_miss(&self, key: Value, fen: Value) -> Result<Value> {
        let (key, fen) = (text(&key), text(&fen));
        self.database
            .write_async(move |ctx| library::record_repertoire_miss(ctx.host, ctx.db, &key, &fen))
            .await
    }

    async fn clear_repertoire_misses(&self, key: Value) -> Result<Value> {
        let key = text(&key);
        self.database
            .write_async(move |ctx| library::clear_repertoire_misses(ctx.host, ctx.db, &key))
            .await
    }

    async fn restore_repertoire_misses(&self, key: Value, misses: Value) -> Result<Value> {
        let key = text(&key);
        self.database
            .write_async(move |ctx| {
                library::restore_repertoire_misses(ctx.host, ctx.db, &key, &misses)
            })
            .await
    }

    async fn review_repertoire_line(
        &self,
        key: Value,
        line: Value,
        correct: Value,
    ) -> Result<Value> {
        let (key, line) = (text(&key), text(&line));
        let (correct, now) = (correct == Value::Bool(true), now_ms() as f64);
        self.database
            .write_async(move |ctx| {
                library::review_repertoire_line(ctx.host, ctx.db, &key, &line, correct, now)
            })
            .await
    }

    async fn view_preferences(&self) -> Result<Value> {
        self.database
            .read_async(|ctx| view_preferences::all(ctx).map(Value::Object))
            .await
    }

    async fn set_view_preference(&self, key: Value, value: Value) -> Result<Value> {
        let key = text(&key);
        self.database
            .write_async(move |ctx| view_preferences::set(ctx, &key, &value))
            .await?;
        Ok(Value::Null)
    }
}
