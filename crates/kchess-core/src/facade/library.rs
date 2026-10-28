//! The facade's the library: studies, played games, drills, sessions and repertoire notes (`run_facade` arms, moved without changes).

use super::args::{arg, normal, normal_args, now_ms};
use super::{archived_game, study_command};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// What the Electron app's renderer kept in this profile's `localStorage`; None when it
    /// cannot be read, which is logged, so nothing is marked as taken over and the next start
    /// tries again.
    async fn electron_local_storage(&self) -> Option<std::collections::BTreeMap<String, String>> {
        let profile = self.data_dir.clone();
        let read = tokio::task::spawn_blocking(move || {
            kchess_electron_legacy::local_storage::read(&profile)
        })
        .await;
        match read {
            Ok(Ok(found)) => Some(found),
            Ok(Err(cause)) => {
                self.host.log(
                    crate::host::Level::Warn,
                    "library",
                    &format!("The Electron app's saved data could not be read: {cause}"),
                );
                None
            }
            Err(_) => None,
        }
    }

    /// Answers the facade's library methods.
    pub(crate) async fn run_library(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "library" => {
                let library = self
                    .database
                    .call_async("store.library.library", &[json!(now_ms())])
                    .await?;
                if library["imported"] == json!(true) {
                    return Ok(library);
                }
                // The documents the Electron renderer kept before the core did, taken over once.
                let Some(found) = self.electron_local_storage().await else {
                    return Ok(library);
                };
                let mut documents = serde_json::Map::new();
                for (key, text) in found {
                    if kchess_domain::misc::constants::LEGACY_DOCUMENT_KEYS.contains(&key.as_str())
                    {
                        documents.insert(key, Value::String(text));
                    }
                }
                let documents = normal("assertLegacyDocuments", &Value::Object(documents))?;
                self.database
                    .call_async("store.library.importLibrary", &[documents])
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
                        &[command, json!(now_ms()), arg(args, 1), arg(args, 2)],
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
                let imported = self
                    .database
                    .call_async("store.viewPreferences.imported", &[])
                    .await?;
                if imported == json!(true) {
                    return self
                        .database
                        .call_async("store.viewPreferences.all", &[])
                        .await;
                }
                // The view preferences the Electron renderer kept in `localStorage`, taken over once.
                let Some(found) = self.electron_local_storage().await else {
                    return self
                        .database
                        .call_async("store.viewPreferences.all", &[])
                        .await;
                };
                let texts: serde_json::Map<String, Value> = found
                    .into_iter()
                    .filter(|(key, _)| {
                        !kchess_domain::misc::constants::LEGACY_DOCUMENT_KEYS
                            .contains(&key.as_str())
                    })
                    .map(|(key, text)| (key, Value::String(text)))
                    .collect();
                self.database
                    .call_async(
                        "store.viewPreferences.importLegacy",
                        &[Value::Object(texts)],
                    )
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
