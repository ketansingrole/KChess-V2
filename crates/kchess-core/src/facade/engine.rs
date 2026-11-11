//! The computer's engine, analysis and game review.

use kchess_contracts::CoreEvent;
use std::path::PathBuf;
use std::sync::Arc;

use kchess_contracts::Variant;
use kchess_contracts::check::Moves;
use kchess_contracts::core_api::{Assist, EngineApi};
use kchess_contracts::types::{
    AnalysisRequest, BestMoveOptions, EngineInstalled, EngineLevel, EngineStatus,
};
use kchess_domain::misc::checks;
use serde_json::{Value, json};

use super::args::{normal, text, to_json};
use crate::core::{Core, LocationOverride, Mutation};
use crate::engine::review::ReviewRequest;
use crate::engine::status::{engine_identity, engine_status, variant_engine_status};
use crate::error::{CoreError, Result};
use crate::store::games::as_json;

impl Core {
    /// The engine path the settings name.
    async fn configured_engine(&self, gate: Assist) -> Result<String> {
        let settings = self.get_settings().await?;
        // A game can start while the settings were read.
        self.assistance_allowed(gate.refusal_after())?;
        Ok(text(&settings["enginePath"]))
    }
}

impl EngineApi for Core {
    async fn engine_status(&self, variant: Option<Variant>) -> Result<EngineStatus> {
        let host = self.host.as_ref();
        let mut status = if variant.and_then(Variant::fairy_name).is_some() {
            variant_engine_status(host, &self.ctx.locations).await
        } else {
            let configured = text(&self.get_settings().await?["enginePath"]);
            engine_status(host, &self.ctx.locations, &configured).await
        };
        if status.ready {
            status.identity = Some(engine_identity(&status).await?);
        }
        Ok(status)
    }

    async fn install_engine(&self) -> Result<EngineInstalled> {
        let id = self.next_request();
        let installed = self
            .managed_request(id, Mutation::Install, &LocationOverride::default())
            .await?;
        serde_json::from_value(installed)
            .map_err(|cause| CoreError::new(format!("Invalid install: {cause}")))
    }

    async fn delete_engine(&self) -> Result<()> {
        let id = self.next_request();
        self.managed_request(id, Mutation::Delete, &LocationOverride::default())
            .await
            .map(drop)
    }

    // A window stops its own computer search, not another window's game.
    async fn stop_engine(&self) -> Result<()> {
        self.search
            .stop_connection(crate::connection::current_or_app());
        Ok(())
    }

    async fn best_move(
        &self,
        Moves(moves): Moves,
        level: EngineLevel,
        options: Option<BestMoveOptions>,
    ) -> Result<String> {
        let configured = self.configured_engine(Assist::Engine);
        self.search
            .best_move_with_settings(
                moves,
                level.as_str(),
                options.unwrap_or_default(),
                configured,
            )
            .await
    }

    async fn start_analysis(&self, request: AnalysisRequest) -> Result<u64> {
        let configured = self.configured_engine(Assist::Analysis);
        let host = Arc::clone(&self.host);
        // Its updates go to the window that asked, not to every window.
        let connection = crate::connection::current_or_app();
        self.analysis
            .start_analysis_with_settings(
                request,
                configured,
                Arc::new(move |update| {
                    host.emit_to(connection, CoreEvent::Analysis(json!(update)))
                }),
            )
            .await
    }

    async fn stop_analysis(&self, kill: Option<bool>) -> Result<()> {
        self.analysis.stop_mine(kill == Some(true));
        Ok(())
    }

    async fn review_get(&self, fen: Value, moves: Value, variant: Option<Value>) -> Result<Value> {
        let mut asked = json!({ "fen": fen, "moves": moves });
        // Standard chess has no variant; `null` is not one.
        if let Some(variant) = variant {
            asked["variant"] = variant;
        }
        let request = normal(checks::review_request, &asked)?;
        let moves: Vec<String> = match &request["moves"] {
            Value::Null => Vec::new(),
            moves => serde_json::from_value(moves.clone())
                .map_err(|e| CoreError::new(format!("Invalid moves: {e}")))?,
        };
        let variant = request["variant"].as_str();
        let fen = text(&request["fen"]);
        to_json(self.reviews.get_review(&fen, &moves, variant).await)
    }

    async fn review_request(&self, request: Value) -> Result<Value> {
        let request: ReviewRequest = serde_json::from_value(request)
            .map_err(|e| CoreError::new(format!("Invalid request: {e}")))?;
        to_json(self.reviews.request_review(request).await)
    }

    async fn review_cancel(&self, key: Value) -> Result<Value> {
        self.reviews.cancel_review(&text(&key));
        Ok(Value::Null)
    }

    async fn review_status(&self) -> Result<Value> {
        as_json(self.reviews.review_status())
    }

    async fn review_summaries(&self, ids: Value) -> Result<Value> {
        let ids: Vec<String> = ids
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(text)
            .collect();
        self.database
            .read_async(move |ctx| crate::store::reviews::review_summaries(ctx.host, ctx.db, &ids))
            .await
    }

    async fn trust_engine_path(&self, path: String) -> Result<()> {
        self.search.trust_engine_path(&PathBuf::from(path));
        Ok(())
    }
}
