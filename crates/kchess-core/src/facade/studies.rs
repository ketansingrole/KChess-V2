//! Lichess studies, positions and the opening explorer.

use kchess_contracts::check::{
    Bounded, ChapterName, Fen, GameId, LichessId, Moves, PgnText, Position, StudyId, Username,
};
use kchess_contracts::core_api::{Assist, StudiesApi};
use kchess_contracts::types::{
    ChapterInput, CloudEval, LichessStudy, LookupOptions, OpeningDescription, PopularityMonth,
    PositionLookup, PositionLookupKind, Reply, StudyExport, StudySyncRequest,
};

use crate::core::Core;
use crate::lichess::{lookups, studies};
use kchess_contracts::error::{CoreError, Result};

impl StudiesApi for Core {
    async fn position_lookup(
        &self,
        kind: PositionLookupKind,
        fen: Fen,
        options: Option<LookupOptions>,
    ) -> Result<PositionLookup> {
        let lookups = self.services.lookups();
        lookups.lookup(kind, &fen.0, options.as_ref()).await
    }

    async fn lichess_studies(&self, account: Username) -> Result<Reply<Vec<LichessStudy>>> {
        studies::lichess_studies(self.services.lichess(), &account.0).await
    }

    async fn lichess_study_chapters(
        &self,
        account: Username,
        id: LichessId,
    ) -> Result<Reply<Vec<ChapterInput>>> {
        studies::lichess_study_chapters(self.services.lichess(), &account.0, &id.0).await
    }

    async fn lichess_study_chapter(&self, id: LichessId, chapter: StudyId) -> Result<ChapterInput> {
        studies::public_study_chapter(self.services.lichess(), &id.0, &chapter.0).await
    }

    async fn sync_lichess_study(
        &self,
        request: StudySyncRequest,
    ) -> Result<Reply<Vec<ChapterInput>>> {
        studies::sync_lichess_study(self.services.lichess(), &request).await
    }

    async fn export_to_lichess_study(
        &self,
        account: Username,
        study_id: StudyId,
        name: ChapterName,
        pgn: PgnText,
    ) -> Result<Reply<StudyExport>> {
        let exported = studies::export_to_lichess_study(
            self.services.lichess(),
            &account.0,
            &study_id.0,
            &name.0,
            &pgn.0,
        )
        .await?;
        Ok(match exported {
            Reply::Done(id) => Reply::Done(StudyExport { id }),
            Reply::NeedsReconnect => Reply::NeedsReconnect,
        })
    }

    async fn export_game(&self, id: GameId) -> Result<String> {
        self.services.lichess().export_game(&id.0).await
    }

    async fn masters_game(&self, id: GameId) -> Result<String> {
        self.services.lookups().masters_game(&id.0).await
    }

    async fn opening_description(&self, moves: Moves) -> Result<Option<OpeningDescription>> {
        self.services.lookups().opening_description(&moves.0).await
    }

    async fn opening_popularity(&self, moves: Moves) -> Result<Vec<PopularityMonth>> {
        self.services.lookups().opening_popularity(&moves.0).await
    }

    async fn cloud_eval(&self, fen: Position, lines: Bounded<1, 5>) -> Result<Option<CloudEval>> {
        // Each request sends the position to Lichess, so it needs the setting turned on.
        if !self.get_settings().await?.core.cloud_eval {
            return Err(CoreError::new(
                "Turn on cloud evaluation in Settings → Analysis first.",
            ));
        }
        // A game starting while settings loaded still blocks, as `bestMove` rechecks.
        self.assistance_allowed(Assist::Analysis.refusal_after())?;
        let lichess = self.services.lichess();
        lookups::cloud_eval(lichess, self.services.cloud(), &fen.0, lines.0 as usize).await
    }
}
