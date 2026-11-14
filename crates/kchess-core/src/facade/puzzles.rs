//! Puzzles, the puzzle database, training runs and the voice input log.

use kchess_contracts::check::{ActivityMax, Days, PuzzleId, Username, VoiceId, VoiceLimit};
use kchess_contracts::core_api::PuzzlesApi;
use kchess_contracts::types::{
    LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleActivityEntry, PuzzleAttemptInput,
    PuzzleDbStatus, PuzzleDraw, PuzzleProgress, PuzzleProgressQuery, PuzzleRequest,
    PuzzleSolveRequest, PuzzleSolveResult, Reply, RunInput, RunKind, RunSaved, RunSummary,
    VoiceAttempt, VoiceAttemptInput, VoiceUpdate,
};
use serde_json::Value;

use crate::core::Core;
use crate::lichess::puzzles as lichess;
use crate::store::{runs, voice_log};
use kchess_contracts::error::Result;

impl PuzzlesApi for Core {
    async fn puzzle_next(&self, request: PuzzleRequest) -> Result<Reply<PuzzleDraw>> {
        lichess::puzzle_next(self.services.lichess(), &request).await
    }

    async fn puzzle_solve(&self, request: PuzzleSolveRequest) -> Result<Reply<PuzzleSolveResult>> {
        lichess::puzzle_solve(self.services.lichess(), &request).await
    }

    async fn puzzle_daily(&self) -> Result<Puzzle> {
        lichess::puzzle_daily(self.services.lichess()).await
    }

    async fn puzzle_by_id(&self, id: PuzzleId) -> Result<Puzzle> {
        lichess::puzzle_by_id(self.services.lichess(), &id.0).await
    }

    async fn puzzle_dashboard(&self, account: Username, days: Days) -> Result<Reply<Value>> {
        lichess::puzzle_dashboard(self.services.lichess(), &account.0, days.0).await
    }

    async fn puzzle_activity(
        &self,
        account: Username,
        max: ActivityMax,
    ) -> Result<Reply<Vec<PuzzleActivityEntry>>> {
        lichess::puzzle_activity(self.services.lichess(), &account.0, max.0).await
    }

    async fn storm_dashboard(&self, username: Username, days: Days) -> Result<Value> {
        lichess::storm_dashboard(self.services.lichess(), &username.0, days.0).await
    }

    async fn puzzle_db_status(&self) -> Result<PuzzleDbStatus> {
        self.puzzles.status().await
    }

    async fn puzzle_db_install(&self) -> Result<PuzzleDbStatus> {
        self.puzzles.install(self.puzzle_db_url.clone()).await
    }

    async fn puzzle_db_cancel(&self) -> Result<()> {
        self.puzzles.cancel();
        Ok(())
    }

    async fn puzzle_db_delete(&self) -> Result<PuzzleDbStatus> {
        self.puzzles.delete().await
    }

    async fn local_puzzles(&self, query: LocalPuzzleQuery) -> Result<Vec<Puzzle>> {
        self.puzzles.query(query).await
    }

    async fn local_ladder(&self, query: LocalLadderQuery) -> Result<Vec<Puzzle>> {
        self.puzzles.ladder(query).await
    }

    async fn record_puzzle_attempt(&self, attempt: PuzzleAttemptInput) -> Result<PuzzleProgress> {
        self.puzzles.record_attempt(attempt).await
    }

    async fn puzzle_progress(&self, query: PuzzleProgressQuery) -> Result<PuzzleProgress> {
        self.puzzles.progress(query).await
    }

    async fn save_run(&self, run: RunInput) -> Result<RunSaved> {
        self.database
            .write_async(move |ctx| runs::save_run(ctx.host, ctx.db, &run))
            .await
    }

    async fn run_summary(&self, kind: RunKind) -> Result<RunSummary> {
        self.database
            .read_async(move |ctx| runs::run_summary(ctx.host, ctx.db, kind))
            .await
    }

    async fn clear_runs(&self, kind: Option<RunKind>) -> Result<()> {
        self.database
            .write_async(move |ctx| runs::clear_runs(ctx.db, kind))
            .await
    }

    async fn save_voice_attempt(&self, attempt: VoiceAttemptInput) -> Result<i64> {
        self.database
            .write_async(move |ctx| voice_log::save_voice_attempt(ctx.db, &attempt))
            .await
    }

    async fn update_voice_attempt(&self, id: VoiceId, update: VoiceUpdate) -> Result<()> {
        self.database
            .write_async(move |ctx| voice_log::update_voice_attempt(ctx.db, id.0, &update))
            .await
    }

    async fn voice_history(&self, limit: VoiceLimit) -> Result<Vec<VoiceAttempt>> {
        self.database
            .read_async(move |ctx| voice_log::voice_history(ctx.host, ctx.db, limit.0))
            .await
    }

    async fn clear_voice_history(&self) -> Result<()> {
        self.database
            .write_async(|ctx| voice_log::clear_voice_history(ctx.db))
            .await
    }

    async fn voice_history_document(&self) -> Result<String> {
        self.database
            .read_async(|ctx| voice_log::voice_history_document(ctx.host, ctx.db))
            .await
    }
}
