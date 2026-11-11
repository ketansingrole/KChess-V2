//! The facade's library methods: studies, played games, drills, sessions and repertoire notes.

use super::args::{arg, normal, normal_args, now_ms, text};
use super::{archived_game, study_command};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::store::{library, library_import, view_preferences};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's library methods.
    pub(crate) async fn run_library(&self, method: &str, args: &[Value]) -> Result<Value> {
        let now = now_ms() as f64;
        let database = &self.database;
        match method {
            "library" => {
                database
                    .read_async(move |ctx| library_import::cached_library(ctx, now))
                    .await
            }
            "studyCommand" => {
                let shape = normal("assertStudyCommandShape", &arg(args, 0))?;
                let command = study_command(&shape)?;
                let incremental = arg(args, 1) == Value::Bool(true);
                let based_on = arg(args, 2).as_str().map(str::to_string);
                database
                    .write_async(move |ctx| {
                        library::run_study_command(
                            ctx.db,
                            command,
                            now,
                            incremental,
                            based_on.as_deref(),
                        )
                    })
                    .await
            }
            "saveArchivedGame" => {
                let shape = normal("assertArchivedGameShape", &arg(args, 0))?;
                let game = archived_game(&shape)?;
                database
                    .write_async(move |ctx| library::write_game(ctx.db, &game))
                    .await?;
                Ok(Value::Null)
            }
            "removeArchivedGame" => {
                let id = text(&normal("assertLibraryId", &arg(args, 0))?);
                database
                    .write_async(move |ctx| library::remove_archived_game(ctx.db, &id))
                    .await?;
                Ok(Value::Null)
            }
            "addMistakes" => {
                let key = text(&normal("assertReviewKey", &arg(args, 0))?);
                let color = match args.get(1) {
                    None | Some(Value::Null) => None,
                    Some(value) => normal("assertSide", value)?.as_str().map(str::to_string),
                };
                database
                    .write_async(move |ctx| {
                        library::add_mistakes(ctx.host, ctx.db, &key, color.as_deref(), now)
                    })
                    .await
            }
            "answerMistake" => {
                let id = text(&normal("assertLibraryId", &arg(args, 0))?);
                let solved = normal("assertBoolean", &arg(args, 1))? == Value::Bool(true);
                database
                    .write_async(move |ctx| library::answer_mistake(ctx.db, &id, solved, now))
                    .await
            }
            "mistakeGames" => {
                database
                    .read_async(|ctx| library::mistake_games(ctx.host, ctx.db))
                    .await
            }
            "saveSession" => {
                let kind = normal("assertSessionKind", &arg(args, 0))?;
                let value = normal_args("assertSession", &[kind.clone(), arg(args, 1)])?;
                let kind = text(&kind);
                database
                    .write_async(move |ctx| library::save_session(ctx.db, &kind, value))
                    .await
            }
            "session" => {
                let kind = text(&normal("assertSessionKind", &arg(args, 0))?);
                database
                    .read_async(move |ctx| library::session(ctx.host, ctx.db, &kind))
                    .await
            }
            "mistakes" => {
                database
                    .read_async(|ctx| library_import::read_mistakes(ctx.db).map(Value::Array))
                    .await
            }
            "joinedTournaments" => {
                database
                    .read_async(move |ctx| library::joined_tournaments(ctx.host, ctx.db, now))
                    .await
            }
            "recordRepertoireMiss" => {
                let key = text(&normal("assertRepertoireKey", &arg(args, 0))?);
                let fen = normal(
                    "assertAnalysisRequest",
                    &json!({ "fen": arg(args, 1), "lines": 1 }),
                )?;
                let fen = text(&fen["fen"]);
                database
                    .write_async(move |ctx| {
                        library::record_repertoire_miss(ctx.host, ctx.db, &key, &fen)
                    })
                    .await
            }
            "clearRepertoireMisses" => {
                let key = text(&normal("assertRepertoireKey", &arg(args, 0))?);
                database
                    .write_async(move |ctx| {
                        library::clear_repertoire_misses(ctx.host, ctx.db, &key)
                    })
                    .await
            }
            "restoreRepertoireMisses" => {
                let key = text(&normal("assertRepertoireKey", &arg(args, 0))?);
                let misses = arg(args, 1);
                database
                    .write_async(move |ctx| {
                        library::restore_repertoire_misses(ctx.host, ctx.db, &key, &misses)
                    })
                    .await
            }
            "reviewRepertoireLine" => {
                let key = text(&normal("assertRepertoireKey", &arg(args, 0))?);
                let line = text(&normal("assertRepertoireLine", &arg(args, 1))?);
                let correct = normal("assertBoolean", &arg(args, 2))? == Value::Bool(true);
                database
                    .write_async(move |ctx| {
                        library::review_repertoire_line(ctx.host, ctx.db, &key, &line, correct, now)
                    })
                    .await
            }
            "viewPreferences" => {
                database
                    .read_async(|ctx| view_preferences::all(ctx).map(Value::Object))
                    .await
            }
            "setViewPreference" => {
                let key = text(&normal("assertViewPreferenceKey", &arg(args, 0))?);
                let value = arg(args, 1);
                database
                    .write_async(move |ctx| view_preferences::set(ctx, &key, &value))
                    .await?;
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
