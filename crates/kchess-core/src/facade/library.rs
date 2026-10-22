//! The facade's the library: studies, played games, drills, sessions and repertoire notes (`run_facade` arms, moved without changes).

use super::args::{arg, normal, normal_args, now_ms};
use super::{archived_game, study_command};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's library methods.
    pub(crate) async fn run_library(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "library" => {
                self.database
                    .call_async("store.library.library", &[json!(now_ms())])
                    .await
            }
            "importLibrary" => {
                let documents = normal("assertLegacyDocuments", &arg(args, 0))?;
                self.database
                    .call_async("store.library.importLibrary", &[documents])
                    .await
            }
            "studyCommand" => {
                let shape = normal("assertStudyCommandShape", &arg(args, 0))?;
                let command = study_command(&shape)?;
                self.database
                    .call_async(
                        "store.library.studyCommand",
                        &[command, json!(now_ms()), arg(args, 1)],
                    )
                    .await
            }
            "saveArchivedGame" => {
                let shape = normal("assertArchivedGameShape", &arg(args, 0))?;
                let game = archived_game(&shape)?;
                self.database
                    .call_async("store.library.saveArchivedGame", &[game])
                    .await
            }
            "removeArchivedGame" => {
                let id = normal("assertLibraryId", &arg(args, 0))?;
                self.database
                    .call_async("store.library.removeArchivedGame", &[id])
                    .await
            }
            "addMistakes" => {
                let key = normal("assertReviewKey", &arg(args, 0))?;
                let color = match args.get(1) {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertSide", value)?,
                };
                self.database
                    .call_async("store.library.addMistakes", &[key, color, json!(now_ms())])
                    .await
            }
            "answerMistake" => {
                let id = normal("assertLibraryId", &arg(args, 0))?;
                let solved = normal("assertBoolean", &arg(args, 1))?;
                self.database
                    .call_async(
                        "store.library.answerMistake",
                        &[id, solved, json!(now_ms())],
                    )
                    .await
            }
            "mistakeGames" => {
                self.database
                    .call_async("store.library.mistakeGames", &[])
                    .await
            }
            "saveSession" => {
                let kind = normal("assertSessionKind", &arg(args, 0))?;
                let value = normal_args("assertSession", &[kind.clone(), arg(args, 1)])?;
                self.database
                    .call_async("store.library.saveSession", &[kind, value])
                    .await
            }
            "joinedTournaments" => {
                self.database
                    .call_async("store.library.joinedTournaments", &[json!(now_ms())])
                    .await
            }
            "recordRepertoireMiss" => {
                let key = normal("assertRepertoireKey", &arg(args, 0))?;
                let fen = normal(
                    "assertAnalysisRequest",
                    &json!({ "fen": arg(args, 1), "lines": 1 }),
                )?;
                self.database
                    .call_async(
                        "store.library.recordRepertoireMiss",
                        &[key, fen["fen"].clone()],
                    )
                    .await
            }
            "clearRepertoireMisses" => {
                let key = normal("assertRepertoireKey", &arg(args, 0))?;
                self.database
                    .call_async("store.library.clearRepertoireMisses", &[key])
                    .await
            }
            "viewPreferences" => {
                self.database
                    .call_async("store.viewPreferences.all", &[])
                    .await
            }
            "setViewPreference" => {
                let key = normal("assertViewPreferenceKey", &arg(args, 0))?;
                self.database
                    .call_async("store.viewPreferences.set", &[key, arg(args, 1)])
                    .await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
