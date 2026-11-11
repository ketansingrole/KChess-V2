//! The facade's puzzles, the puzzle database and training runs (`run_facade` arms, moved without changes).

use super::PUZZLE_DB_URL;
use super::args::{arg, normal, text};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use crate::store::{runs, voice_log};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's puzzles methods.
    pub(crate) async fn run_puzzles(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "puzzleNext" => {
                let request = normal("assertPuzzleRequest", &arg(args, 0))?;
                self.inner("lichess.puzzleNext", vec![request]).await
            }
            "puzzleSolve" => {
                let request = normal("assertPuzzleSolve", &arg(args, 0))?;
                self.inner("lichess.puzzleSolve", vec![request]).await
            }
            "puzzleDaily" => self.inner("lichess.puzzleDaily", vec![]).await,
            "puzzleById" => {
                let id = normal("assertPuzzleId", &arg(args, 0))?;
                self.inner("lichess.puzzleById", vec![id]).await
            }
            "puzzleDashboard" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let days = normal("assertDays", &arg(args, 1))?;
                self.inner(
                    "lichess.puzzleDashboard",
                    vec![json!({ "account": account, "days": days })],
                )
                .await
            }
            "puzzleActivity" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let max = normal("assertActivityMax", &arg(args, 1))?;
                self.inner("lichess.puzzleActivity", vec![account, max])
                    .await
            }
            "stormDashboard" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let days = normal("assertDays", &arg(args, 1))?;
                self.inner("lichess.stormDashboard", vec![name, days]).await
            }
            _ => self.run_local_puzzles(method, args).await,
        }
    }

    /// Answers the facade's puzzle database methods.
    async fn run_local_puzzles(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "puzzleDbStatus" => self.puzzle_operation("puzzles.status", vec![]).await,
            "puzzleDbInstall" => {
                self.puzzle_operation("puzzles.install", vec![json!(PUZZLE_DB_URL)])
                    .await
            }
            "puzzleDbCancel" => {
                if let Err(cause) = self.dispatch("puzzles.cancel", vec![]).await {
                    self.host.log(
                        Level::Warn,
                        "puzzles",
                        &format!("Puzzle download cancel failed: {}", cause.message),
                    );
                }
                Ok(Value::Null)
            }
            "puzzleDbDelete" => self.puzzle_operation("puzzles.delete", vec![]).await,
            "localPuzzles" => {
                let query = normal("assertLocalQuery", &arg(args, 0))?;
                self.puzzle_operation("puzzles.query", vec![query]).await
            }
            "localLadder" => {
                let query = normal("assertLadderQuery", &arg(args, 0))?;
                self.puzzle_operation("puzzles.ladder", vec![query]).await
            }
            "recordPuzzleAttempt" => {
                let attempt = normal("assertPuzzleAttempt", &arg(args, 0))?;
                self.puzzle_operation("puzzles.recordAttempt", vec![attempt])
                    .await
            }
            "puzzleProgress" => {
                let query = normal("assertPuzzleProgressQuery", &arg(args, 0))?;
                self.puzzle_operation("puzzles.progress", vec![query]).await
            }
            _ => self.run_training_logs(method, args).await,
        }
    }

    /// Answers the facade's training run and voice log methods.
    async fn run_training_logs(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "saveRun" => {
                let run = normal("assertRunInput", &arg(args, 0))?;
                self.database
                    .write_async(move |ctx| runs::save_run(ctx.host, ctx.db, &run))
                    .await
            }
            "runSummary" => {
                let kind = text(&normal("assertRunKind", &arg(args, 0))?);
                self.database
                    .read_async(move |ctx| runs::run_summary(ctx.host, ctx.db, &kind))
                    .await
            }
            "clearRuns" => {
                let kind = match args.first() {
                    None | Some(Value::Null) => None,
                    Some(value) => normal("assertRunKind", value)?.as_str().map(str::to_string),
                };
                self.database
                    .write_async(move |ctx| runs::clear_runs(ctx.db, kind.as_deref()))
                    .await?;
                Ok(Value::Null)
            }
            "saveVoiceAttempt" => {
                let attempt = normal("assertVoiceAttempt", &arg(args, 0))?;
                let id = self
                    .database
                    .write_async(move |ctx| voice_log::save_voice_attempt(ctx.db, &attempt))
                    .await?;
                Ok(json!(id))
            }
            "updateVoiceAttempt" => {
                let id = normal("assertVoiceId", &arg(args, 0))?
                    .as_i64()
                    .ok_or_else(|| CoreError::new("id must be a number"))?;
                let update = normal("assertVoiceUpdate", &arg(args, 1))?;
                self.database
                    .write_async(move |ctx| voice_log::update_voice_attempt(ctx.db, id, &update))
                    .await?;
                Ok(Value::Null)
            }
            "voiceHistory" => {
                let limit = normal("assertVoiceLimit", &arg(args, 0))?
                    .as_i64()
                    .ok_or_else(|| CoreError::new("limit must be a number"))?;
                self.database
                    .read_async(move |ctx| {
                        voice_log::voice_history(ctx.host, ctx.db, limit).map(Value::Array)
                    })
                    .await
            }
            "clearVoiceHistory" => {
                self.database
                    .write_async(|ctx| voice_log::clear_voice_history(ctx.db))
                    .await?;
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
