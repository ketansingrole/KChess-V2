//! Lichess tournaments.

use kchess_contracts::check::{
    OptionalAccount, StandingPage, TournamentId, TournamentPassword, Username,
};
use kchess_contracts::core_api::TournamentsApi;
use kchess_contracts::types::JoinedTournament;
use kchess_contracts::types::{
    NewArena, Reply, TournamentDetail, TournamentList, TournamentStatus, TournamentSummary,
    TournamentSystem,
};

use crate::core::Core;
use crate::host::Level;
use crate::lichess::tournaments;
use crate::store::library;
use crate::time::now_ms;
use kchess_contracts::error::Result;
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl Core {
    /// Forgets one account's membership of a tournament, or every account's without one; with
    /// `unchanged_since`, only when no membership changed after that count was read.
    async fn forget_tournament(
        &self,
        system: TournamentSystem,
        id: &str,
        account: Option<&str>,
        unchanged_since: Option<u64>,
    ) -> Result<()> {
        let id = id.to_owned();
        let account = account.map(str::to_owned);
        let memberships = Arc::clone(&self.memberships);
        self.database
            .write_async(move |ctx| {
                if unchanged_since.is_some_and(|seen| seen != memberships.load(Ordering::SeqCst)) {
                    return Ok(());
                }
                library::forget_tournament(ctx.host, ctx.db, system, &id, account.as_deref())?;
                memberships.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
    }
}

/// A call's `Reply<()>` as the facade answers it: `true` once done.
fn done<E: Into<kchess_contracts::error::CoreError>>(
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
        // Whether the account's membership was kept as the lookup began, and the count of changes
        // then: one a join kept meanwhile (another window), or a leave and a join again, is newer
        // than this answer, which an answer from before them erased, and pairings stopped keeping
        // the account's stream open.
        let changes = self.memberships.load(Ordering::SeqCst);
        let was_kept = if system == TournamentSystem::Arena && !account.0.is_empty() {
            let (tournament, name) = (id.0.clone(), account.0.clone());
            let now = now_ms() as f64;
            self.database
                .read_async(move |ctx| library::joined_tournaments(ctx.host, ctx.db, now))
                .await?
                .iter()
                .any(|entry| {
                    entry.system == system && entry.id == tournament && entry.joined_by(&name)
                })
        } else {
            false
        };
        let (detail, signed) =
            tournaments::tournament_signed(lichess, system, &id.0, &account.0, page).await?;
        if system == TournamentSystem::Arena {
            // An arena that is over is over for every account.
            if detail.summary.status == TournamentStatus::Finished {
                self.forget_tournament(system, &id.0, None, None).await?;
            } else if was_kept
                // An anonymous answer (the login unreadable) has no place for anyone.
                && signed
                && detail.me.as_ref().is_none_or(|me| me.withdraw)
            {
                // `me` is the looked-up account's place only: another account's stays.
                self.forget_tournament(system, &id.0, Some(&account.0), Some(changes))
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
        // A logout or removal meanwhile ends the join's work: it keeps no stream open and
        // remembers no tournament for an account that is gone.
        let epoch = lichess.account_epoch(&name);
        let current = || lichess.account_epoch(&name) == epoch;
        let result =
            done(tournaments::join_tournament(lichess, system, &id, &name, password).await)?;
        if result != Reply::Done(true) || !current() {
            return Ok(result);
        }
        // Pairings arrive on the event stream: keep it open while in the tournament.
        self.services.online().stay_connected(&name);
        let now = now_ms();
        // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
        let mut entry = JoinedTournament {
            system,
            id: id.clone(),
            account: name.clone(),
            name: id.clone(),
            until: (now + 86_400_000i64) as f64,
        };
        match tournaments::tournament(lichess, system, &id, &name, 1).await {
            Ok(detail) => {
                entry.name = detail.summary.name;
                if let Some(finishes_at) = detail.summary.finishes_at {
                    entry.until = finishes_at as f64;
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
        if !current() {
            return Ok(result);
        }
        let memberships = Arc::clone(&self.memberships);
        self.database
            .write_async(move |ctx| {
                library::remember_tournament(ctx.host, ctx.db, &entry, now as f64)?;
                memberships.fetch_add(1, Ordering::SeqCst);
                Ok(())
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
            self.forget_tournament(system, &id.0, Some(&account.0), None)
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
