//! Puzzles, the puzzle database, training runs and the voice input log.

use kchess_contracts::core_api::PuzzlesApi;
use serde_json::{Value, json};

use super::PUZZLE_DB_URL;
use super::args::{count, parse, reply_json, text, to_json};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use crate::lichess::puzzles as lichess;
use crate::store::{runs, voice_log};

impl Core {
    /// A puzzle database operation's answer; a failure is logged.
    fn puzzle_operation<T: serde::Serialize>(&self, result: Result<T>) -> Result<Value> {
        to_json(result).inspect_err(|cause| self.puzzle_failed(cause))
    }

    fn puzzle_failed(&self, cause: &CoreError) {
        self.host.log(
            Level::Warn,
            "puzzles",
            &format!("Puzzle operation failed: {}", cause.message),
        );
    }
}

impl PuzzlesApi for Core {
    async fn puzzle_next(&self, request: Value) -> Result<Value> {
        let request = parse(request, "request")?;
        reply_json(lichess::puzzle_next(self.services.lichess(), &request).await)
    }

    async fn puzzle_solve(&self, request: Value) -> Result<Value> {
        let request = parse(request, "request")?;
        reply_json(lichess::puzzle_solve(self.services.lichess(), &request).await)
    }

    async fn puzzle_daily(&self) -> Result<Value> {
        lichess::puzzle_daily(self.services.lichess()).await
    }

    async fn puzzle_by_id(&self, id: Value) -> Result<Value> {
        lichess::puzzle_by_id(self.services.lichess(), &text(&id)).await
    }

    async fn puzzle_dashboard(&self, account: Value, days: Value) -> Result<Value> {
        let (account, days) = (text(&account), count(&days, 2)?);
        reply_json(lichess::puzzle_dashboard(self.services.lichess(), &account, days).await)
    }

    async fn puzzle_activity(&self, account: Value, max: Value) -> Result<Value> {
        let (account, max) = (text(&account), count(&max, 2)?);
        reply_json(lichess::puzzle_activity(self.services.lichess(), &account, max).await)
    }

    async fn storm_dashboard(&self, username: Value, days: Value) -> Result<Value> {
        let (name, days) = (text(&username), count(&days, 2)?);
        lichess::storm_dashboard(self.services.lichess(), &name, days).await
    }

    async fn puzzle_db_status(&self) -> Result<Value> {
        self.puzzle_operation(self.puzzles.status().await)
    }

    async fn puzzle_db_install(&self) -> Result<Value> {
        self.puzzle_operation(self.puzzles.install(PUZZLE_DB_URL.into()).await)
    }

    async fn puzzle_db_cancel(&self) -> Result<Value> {
        self.puzzles.cancel();
        Ok(Value::Null)
    }

    async fn puzzle_db_delete(&self) -> Result<Value> {
        self.puzzle_operation(self.puzzles.delete().await)
    }

    async fn local_puzzles(&self, query: Value) -> Result<Value> {
        let query = parse(query, "query").inspect_err(|cause| self.puzzle_failed(cause))?;
        self.puzzle_operation(self.puzzles.query(query).await)
    }

    async fn local_ladder(&self, query: Value) -> Result<Value> {
        let query = parse(query, "query").inspect_err(|cause| self.puzzle_failed(cause))?;
        self.puzzle_operation(self.puzzles.ladder(query).await)
    }

    async fn record_puzzle_attempt(&self, attempt: Value) -> Result<Value> {
        let attempt = parse(attempt, "attempt").inspect_err(|cause| self.puzzle_failed(cause))?;
        self.puzzle_operation(self.puzzles.record_attempt(attempt).await)
    }

    async fn puzzle_progress(&self, query: Value) -> Result<Value> {
        let query = parse(query, "query").inspect_err(|cause| self.puzzle_failed(cause))?;
        self.puzzle_operation(self.puzzles.progress(query).await)
    }

    async fn save_run(&self, run: Value) -> Result<Value> {
        self.database
            .write_async(move |ctx| runs::save_run(ctx.host, ctx.db, &run))
            .await
    }

    async fn run_summary(&self, kind: Value) -> Result<Value> {
        let kind = text(&kind);
        self.database
            .read_async(move |ctx| runs::run_summary(ctx.host, ctx.db, &kind))
            .await
    }

    async fn clear_runs(&self, kind: Option<Value>) -> Result<Value> {
        let kind = kind.as_ref().and_then(Value::as_str).map(str::to_string);
        self.database
            .write_async(move |ctx| runs::clear_runs(ctx.db, kind.as_deref()))
            .await?;
        Ok(Value::Null)
    }

    async fn save_voice_attempt(&self, attempt: Value) -> Result<Value> {
        let id = self
            .database
            .write_async(move |ctx| voice_log::save_voice_attempt(ctx.db, &attempt))
            .await?;
        Ok(json!(id))
    }

    async fn update_voice_attempt(&self, id: Value, update: Value) -> Result<Value> {
        let id = id
            .as_i64()
            .ok_or_else(|| CoreError::new("id must be a number"))?;
        self.database
            .write_async(move |ctx| voice_log::update_voice_attempt(ctx.db, id, &update))
            .await?;
        Ok(Value::Null)
    }

    async fn voice_history(&self, limit: Value) -> Result<Value> {
        let limit = limit
            .as_i64()
            .ok_or_else(|| CoreError::new("limit must be a number"))?;
        self.database
            .read_async(move |ctx| {
                voice_log::voice_history(ctx.host, ctx.db, limit).map(Value::Array)
            })
            .await
    }

    async fn clear_voice_history(&self) -> Result<Value> {
        self.database
            .write_async(|ctx| voice_log::clear_voice_history(ctx.db))
            .await?;
        Ok(Value::Null)
    }

    async fn voice_history_document(&self) -> Result<Value> {
        self.database
            .read_async(|ctx| {
                voice_log::voice_history_document(ctx.host, ctx.db).map(Value::String)
            })
            .await
    }
}
