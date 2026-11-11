//! Lichess studies, positions and the opening explorer.

use kchess_contracts::core_api::{Assist, StudiesApi};
use kchess_domain::misc::checks;
use serde_json::{Value, json};

use super::args::{count, normal, parse, reply_json, text, to_json};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::lichess::accounts::Reply;
use crate::lichess::{lookups, studies};

impl StudiesApi for Core {
    async fn position_lookup(
        &self,
        kind: Value,
        fen: Value,
        options: Option<Value>,
    ) -> Result<Value> {
        let lookups = self.services.lookups();
        to_json(lookups.lookup(&kind, &fen, options.as_ref()).await)
    }

    async fn lichess_studies(&self, account: Value) -> Result<Value> {
        reply_json(studies::lichess_studies(self.services.lichess(), &text(&account)).await)
    }

    async fn lichess_study_chapters(&self, account: Value, id: Value) -> Result<Value> {
        let (account, id) = (text(&account), text(&id));
        reply_json(studies::lichess_study_chapters(self.services.lichess(), &account, &id).await)
    }

    async fn lichess_study_chapter(&self, id: Value, chapter: Value) -> Result<Value> {
        let (id, chapter) = (text(&id), text(&chapter));
        to_json(studies::public_study_chapter(self.services.lichess(), &id, &chapter).await)
    }

    async fn sync_lichess_study(&self, request: Value) -> Result<Value> {
        let request = parse(request, "request")?;
        reply_json(studies::sync_lichess_study(self.services.lichess(), &request).await)
    }

    async fn export_to_lichess_study(
        &self,
        account: Value,
        study_id: Value,
        name: Value,
        pgn: Value,
    ) -> Result<Value> {
        let (account, study, pgn) = (text(&account), text(&study_id), text(&pgn));
        let name = text(&name).trim().to_string();
        let exported = studies::export_to_lichess_study(
            self.services.lichess(),
            &account,
            &study,
            &name,
            &pgn,
        )
        .await?;
        Ok(match exported {
            Reply::Done(id) => json!({ "id": id }),
            Reply::NeedsReconnect => json!({ "needsReconnect": true }),
        })
    }

    async fn export_game(&self, id: Value) -> Result<Value> {
        to_json(self.services.lichess().export_game(&text(&id)).await)
    }

    async fn masters_game(&self, id: Value) -> Result<Value> {
        to_json(self.services.lookups().masters_game(&text(&id)).await)
    }

    async fn opening_description(&self, moves: Value) -> Result<Value> {
        to_json(self.services.lookups().opening_description(&moves).await)
    }

    async fn opening_popularity(&self, moves: Value) -> Result<Value> {
        to_json(self.services.lookups().opening_popularity(&moves).await)
    }

    async fn cloud_eval(&self, fen: Value, lines: Value) -> Result<Value> {
        // Each request sends the position to Lichess, so it needs the setting turned on.
        if self.get_settings().await?["cloudEval"].as_bool() != Some(true) {
            return Err(CoreError::new(
                "Turn on cloud evaluation in Settings → Analysis first.",
            ));
        }
        // A game starting while settings loaded still blocks, as `bestMove` rechecks.
        self.assistance_allowed(Assist::Analysis.refusal_after())?;
        let request = normal(
            checks::analysis_request,
            &json!({ "fen": fen, "lines": lines }),
        )?;
        let (fen, lines) = (text(&request["fen"]), count(&request["lines"], 2)?);
        let lichess = self.services.lichess();
        to_json(lookups::cloud_eval(lichess, self.services.cloud(), &fen, lines as usize).await)
    }
}
