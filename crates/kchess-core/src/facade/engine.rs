//! The facade's the computer's engine, analysis and game review (`run_facade` arms, moved without changes).

use super::args::{arg, normal};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};
use std::sync::Arc;

impl Core {
    /// Answers the facade's engine methods.
    pub(crate) async fn run_engine(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "engineStatus" => {
                let configured = self.get_settings().await?["enginePath"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let mut status = self.inner("engine.status", vec![json!(configured)]).await?;
                if status["ready"].as_bool() == Some(true) {
                    let identity = self.inner("engine.identity", vec![status.clone()]).await?;
                    status["identity"] = identity;
                }
                Ok(status)
            }
            "installEngine" => {
                let id = self.next_request();
                let installed = self
                    .inner("managedEngine.install", vec![json!(id), Value::Null])
                    .await?;
                Ok(json!({
                    "path": installed["path"],
                    "version": installed["version"],
                    "updated": installed["updated"],
                }))
            }
            "deleteEngine" => {
                let id = self.next_request();
                self.inner("managedEngine.delete", vec![json!(id), Value::Null])
                    .await
            }
            "stopEngine" => {
                self.search.stop_engine(false);
                Ok(Value::Null)
            }
            "bestMove" => {
                self.assistance_allowed(
                    "Engine assistance is unavailable during a live Lichess game.",
                )?;
                let moves = normal("assertMoves", &arg(args, 0))?;
                let level = normal("assertLevel", &arg(args, 1))?;
                let options = normal("assertBestMoveOptions", &arg(args, 2))?;
                let configured = async {
                    let settings = self.get_settings().await?;
                    self.assistance_allowed(
                        "Engine assistance is unavailable during a live Lichess game.",
                    )?;
                    Ok(settings["enginePath"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string())
                };
                self.search
                    .best_move_with_settings(
                        moves,
                        level.as_str().unwrap_or_default(),
                        options,
                        configured,
                    )
                    .await
                    .map(|value| json!(value))
            }
            "startAnalysis" => {
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                let configured = async {
                    let settings = self.get_settings().await?;
                    self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                    Ok(settings["enginePath"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string())
                };
                let host = Arc::clone(&self.host);
                self.analysis
                    .start_analysis_with_settings(
                        arg(args, 0),
                        configured,
                        Arc::new(move |update| {
                            host.emit(crate::engine::analysis::ANALYSIS_EVENT, json!(update))
                        }),
                    )
                    .await
                    .map(|id| json!(id))
            }
            "stopAnalysis" => {
                let kill = match args.first() {
                    None | Some(Value::Null) => false,
                    Some(value) => normal("assertBoolean", value)?.as_bool() == Some(true),
                };
                self.analysis.stop_analysis(kill);
                Ok(Value::Null)
            }
            "reviewGet" => {
                self.assistance_allowed(
                    "Review is unavailable until your Lichess game status is verified.",
                )?;
                let request = normal(
                    "assertReviewRequest",
                    &json!({ "fen": arg(args, 0), "moves": arg(args, 1) }),
                )?;
                self.inner(
                    "reviews.get",
                    vec![request["fen"].clone(), request["moves"].clone()],
                )
                .await
            }
            "reviewRequest" => {
                self.assistance_allowed("Review is unavailable during a live Lichess game.")?;
                let request = normal("assertReviewRequest", &arg(args, 0))?;
                self.inner("reviews.request", vec![request]).await
            }
            "reviewCancel" => {
                let key = normal("assertReviewKey", &arg(args, 0))?;
                self.inner("reviews.cancel", vec![key]).await
            }
            "reviewStatus" => self.inner("reviews.status", vec![]).await,
            "reviewSummaries" => {
                let ids = normal("assertGameIds", &arg(args, 0))?;
                self.database
                    .call_async("store.reviewStore.reviewSummaries", &[ids])
                    .await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
