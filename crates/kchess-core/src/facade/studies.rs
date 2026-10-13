//! The facade's Lichess studies, positions and the explorer (`run_facade` arms, moved without changes).

use super::args::{arg, normal, text_arg};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's studies methods.
    pub(crate) async fn run_studies(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "positionLookup" => {
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                let result = self.inner("positionLookup.lookup", args.to_vec()).await?;
                // A game starting mid-lookup still blocks: the answer must not reach a live game.
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                Ok(result)
            }
            "lichessStudies" => self.inner("studies.list", args.to_vec()).await,
            "lichessStudyChapters" => self.inner("studies.chapters", args.to_vec()).await,
            "syncLichessStudy" => {
                let request = normal("assertStudySyncRequest", &arg(args, 0))?;
                self.inner("studies.sync", vec![request]).await
            }
            "exportToLichessStudy" => {
                let name = text_arg(args, 2);
                self.inner(
                    "studies.export",
                    vec![arg(args, 0), arg(args, 1), json!(name.trim()), arg(args, 3)],
                )
                .await
            }
            "exportGame" => self.inner("lichess.exportGame", args.to_vec()).await,
            "mastersGame" => {
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                let result = self
                    .inner("positionLookup.mastersGame", args.to_vec())
                    .await?;
                // A game starting mid-lookup still blocks: the answer must not reach a live game.
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                Ok(result)
            }
            "cloudEval" => {
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                // Each request sends the position to Lichess, so it needs the setting turned on.
                if self.get_settings().await?["cloudEval"].as_bool() != Some(true) {
                    return Err(CoreError::new(
                        "Turn on cloud evaluation in Settings → Analysis first.",
                    ));
                }
                // A game starting while settings loaded still blocks, as `bestMove` rechecks.
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                let request = normal(
                    "assertAnalysisRequest",
                    &json!({ "fen": arg(args, 0), "lines": arg(args, 1) }),
                )?;
                let result = self
                    .inner(
                        "cloudEval.cloudEval",
                        vec![request["fen"].clone(), request["lines"].clone()],
                    )
                    .await?;
                // A game starting mid-lookup still blocks: the answer must not reach a live game.
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                Ok(result)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
