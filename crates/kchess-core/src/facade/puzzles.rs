//! The facade's puzzles, the puzzle database and training runs (`run_facade` arms, moved without changes).

use super::PUZZLE_DB_URL;
use super::args::{arg, normal};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
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
            "saveRun" => {
                let run = normal("assertRunInput", &arg(args, 0))?;
                self.database.call_async("store.runs.saveRun", &[run]).await
            }
            "runSummary" => {
                let kind = normal("assertRunKind", &arg(args, 0))?;
                self.database
                    .call_async("store.runs.runSummary", &[kind])
                    .await
            }
            "clearRuns" => {
                let kind = match args.first() {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertRunKind", value)?,
                };
                self.database
                    .call_async("store.runs.clearRuns", &[kind])
                    .await
            }
            "saveVoiceAttempt" => {
                let attempt = normal("assertVoiceAttempt", &arg(args, 0))?;
                self.database
                    .call_async("store.voiceLog.saveVoiceAttempt", &[attempt])
                    .await
            }
            "updateVoiceAttempt" => {
                let id = normal("assertVoiceId", &arg(args, 0))?;
                let update = normal("assertVoiceUpdate", &arg(args, 1))?;
                self.database
                    .call_async("store.voiceLog.updateVoiceAttempt", &[id, update])
                    .await
            }
            "voiceHistory" => {
                let limit = normal("assertVoiceLimit", &arg(args, 0))?;
                self.database
                    .call_async("store.voiceLog.voiceHistory", &[limit])
                    .await
            }
            "clearVoiceHistory" => {
                self.database
                    .call_async("store.voiceLog.clearVoiceHistory", &[])
                    .await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
