//! The computer's engine, analysis and game review.

use kchess_contracts::CoreEvent;
use std::path::PathBuf;
use std::sync::Arc;

use kchess_contracts::Variant;
use kchess_contracts::check::{Fen, GameIds, Moves, ReviewKey};
use kchess_contracts::core_api::{Assist, EngineApi};
use kchess_contracts::types::{
    AnalysisRequest, BestMoveOptions, EngineInstalled, EngineLevel, EngineStatus, Keyed,
    ReviewRequest, ReviewStatus, ReviewSummary, StoredReview,
};

use crate::core::{Core, LocationOverride, Mutation};
use crate::engine::status::{engine_identity, engine_status, variant_engine_status};
use crate::error::{CoreError, Result};

impl Core {
    /// The engine path the settings name.
    async fn configured_engine(&self, gate: Assist) -> Result<String> {
        let settings = self.get_settings().await?;
        // A game can start while the settings were read.
        self.assistance_allowed(gate.refusal_after())?;
        Ok(settings.core.engine_path)
    }
}

impl EngineApi for Core {
    async fn engine_status(&self, variant: Option<Variant>) -> Result<EngineStatus> {
        let host = self.host.as_ref();
        let mut status = if variant.and_then(Variant::fairy_name).is_some() {
            variant_engine_status(host, &self.ctx.locations).await
        } else {
            let configured = self.get_settings().await?.core.engine_path;
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
                Arc::new(move |update| host.emit_to(connection, CoreEvent::Analysis(update))),
            )
            .await
    }

    async fn stop_analysis(&self, kill: Option<bool>) -> Result<()> {
        self.analysis.stop_mine(kill == Some(true));
        Ok(())
    }

    async fn review_get(
        &self,
        Fen(fen): Fen,
        Moves(moves): Moves,
        variant: Option<String>,
    ) -> Result<Option<StoredReview>> {
        // Standard chess and Chess960 have no variant to review by.
        let variant = variant
            .filter(|key| Variant::from_key(key).is_some_and(|variant| !variant.standard_rules()));
        self.reviews
            .get_review(&fen, &moves, variant.as_deref())
            .await
    }

    async fn review_request(&self, request: ReviewRequest) -> Result<Option<StoredReview>> {
        self.reviews.request_review(request).await
    }

    async fn review_cancel(&self, ReviewKey(key): ReviewKey) -> Result<()> {
        self.reviews.cancel_review(&key);
        Ok(())
    }

    async fn review_status(&self) -> Result<ReviewStatus> {
        Ok(self.reviews.review_status())
    }

    async fn review_summaries(&self, GameIds(ids): GameIds) -> Result<Keyed<ReviewSummary>> {
        self.database
            .read_async(move |ctx| crate::store::reviews::review_summaries(ctx.host, ctx.db, &ids))
            .await
    }

    async fn trust_engine_path(&self, path: String) -> Result<()> {
        self.search.trust_engine_path(&PathBuf::from(path));
        Ok(())
    }
}
