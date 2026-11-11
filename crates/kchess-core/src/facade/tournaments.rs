//! Lichess tournaments.

use kchess_contracts::check::{
    OptionalAccount, StandingPage, TournamentId, TournamentPassword, Username,
};
use kchess_contracts::core_api::TournamentsApi;
use kchess_contracts::types::{
    NewArena, Reply, TournamentDetail, TournamentList, TournamentStatus, TournamentSummary,
    TournamentSystem,
};
use serde_json::json;

use crate::core::Core;
use crate::error::Result;
use crate::host::Level;
use crate::lichess::tournaments;
use crate::store::library;
use crate::time::now_ms;

impl Core {
    /// Forgets one account's membership of a tournament, or every account's without one.
    async fn forget_tournament(
        &self,
        system: TournamentSystem,
        id: &str,
        account: Option<&str>,
    ) -> Result<()> {
        let id = id.to_owned();
        let account = account.map(str::to_owned);
        self.database
            .write_async(move |ctx| {
                library::forget_tournament(
                    ctx.host,
                    ctx.db,
                    system.as_str(),
                    &id,
                    account.as_deref(),
                )
            })
            .await
    }
}

/// A call's `Reply<()>` as the facade answers it: `true` once done.
fn done<E: Into<crate::error::CoreError>>(
    reply: std::result::Result<Reply<()>, E>,
) -> Result<Reply<bool>> {
    Ok(match reply.map_err(Into::into)? {
        Reply::Done(()) => Reply::Done(true),
        Reply::NeedsReconnect => Reply::NeedsReconnect,
    })
}

impl TournamentsApi for Core {
    async fn tournaments(&self, account: OptionalAccount) -> Result<TournamentList> {
        tournaments::tournaments(self.services.lichess(), &account.0).await
    }

    async fn tournament(
        &self,
        system: TournamentSystem,
        id: TournamentId,
        account: OptionalAccount,
        page: Option<StandingPage>,
    ) -> Result<TournamentDetail> {
        let page = page.map_or(1, |page| page.0 as u32);
        let lichess = self.services.lichess();
        let detail = tournaments::tournament(lichess, system, &id.0, &account.0, page).await?;
        if system == TournamentSystem::Arena {
            // An arena that is over is over for every account.
            if detail.summary.status == TournamentStatus::Finished {
                self.forget_tournament(system, &id.0, None).await?;
            } else if !account.0.is_empty() && detail.me.as_ref().is_none_or(|me| me.withdraw) {
                // `me` is the looked-up account's place only: another account's stays.
                self.forget_tournament(system, &id.0, Some(&account.0))
                    .await?;
            }
        }
        Ok(detail)
    }

    async fn join_tournament(
        &self,
        system: TournamentSystem,
        id: TournamentId,
        account: Username,
        password: Option<TournamentPassword>,
    ) -> Result<Reply<bool>> {
        let (id, name) = (id.0, account.0);
        let password = password.as_ref().map(|password| password.0.as_str());
        let lichess = self.services.lichess();
        let result =
            done(tournaments::join_tournament(lichess, system, &id, &name, password).await)?;
        if result != Reply::Done(true) {
            return Ok(result);
        }
        // Pairings arrive on the event stream: keep it open while in the tournament.
        self.services.online().stay_connected(&name);
        let now = now_ms();
        // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
        let mut entry = json!({
            "system": system.as_str(),
            "id": id,
            "account": name,
            "name": id,
            "until": now + 86_400_000i64,
        });
        match tournaments::tournament(lichess, system, &id, &name, 1).await {
            Ok(detail) => {
                entry["name"] = json!(detail.summary.name);
                if let Some(finishes_at) = detail.summary.finishes_at {
                    entry["until"] = json!(finishes_at);
                }
            }
            Err(cause) => self.host.log(
                Level::Warn,
                "tournaments",
                &format!(
                    "Joined tournament details are unavailable: id={id} {}",
                    cause.message
                ),
            ),
        }
        self.database
            .write_async(move |ctx| {
                library::remember_tournament(ctx.host, ctx.db, &entry, now as f64)
            })
            .await?;
        Ok(result)
    }

    async fn leave_tournament(
        &self,
        system: TournamentSystem,
        id: TournamentId,
        account: Username,
    ) -> Result<Reply<bool>> {
        let lichess = self.services.lichess();
        let left = tournaments::leave_tournament(lichess, system, &id.0, &account.0).await;
        let result = done(left)?;
        if result == Reply::Done(true) {
            self.forget_tournament(system, &id.0, Some(&account.0))
                .await?;
        }
        Ok(result)
    }

    async fn create_tournament(
        &self,
        account: Username,
        arena: NewArena,
    ) -> Result<Reply<TournamentSummary>> {
        let lichess = self.services.lichess();
        tournaments::create_tournament(lichess, &account.0, &arena).await
    }
}
