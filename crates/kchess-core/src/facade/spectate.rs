//! Lichess TV and broadcasts.

use kchess_contracts::check::{BroadcastQuery, LichessId, WatchSession};
use kchess_contracts::core_api::SpectateApi;
use kchess_contracts::types::{
    BroadcastAlert, BroadcastRoundPlace, BroadcastSummary, BroadcastTourDetail, TvChannel,
    WatchTarget,
};

use crate::core::Core;
use crate::lichess::watch;
use crate::store::library;
use kchess_contracts::error::{CoreError, Result};

impl Core {
    fn refuse_watch_while_playing(&self) -> Result<()> {
        if self.services.online().playing() {
            return Err(CoreError::new("Finish your game before watching another."));
        }
        Ok(())
    }
}

impl SpectateApi for Core {
    async fn tv_channels(&self) -> Result<Vec<TvChannel>> {
        watch::tv_channels(self.services.lichess()).await
    }

    async fn watch(&self, target: WatchTarget) -> Result<u64> {
        self.refuse_watch_while_playing()?;
        Ok(self.services.spectator().mine().watch(target))
    }

    async fn watch_broadcast(&self, round_id: LichessId) -> Result<u64> {
        self.refuse_watch_while_playing()?;
        Ok(self.services.spectator().mine().watch_round(&round_id.0))
    }

    async fn stop_watching(&self, session: Option<WatchSession>) -> Result<()> {
        let session = session.map(|session| session.0);
        self.services.spectator().stop_mine(session);
        Ok(())
    }

    async fn broadcasts(&self, query: Option<BroadcastQuery>) -> Result<Vec<BroadcastSummary>> {
        let query = query.as_ref().map(|query| query.0.as_str());
        watch::broadcasts(self.services.lichess(), query).await
    }

    async fn broadcast_tour(&self, id: LichessId) -> Result<BroadcastTourDetail> {
        watch::broadcast_tour(self.services.lichess(), &id.0).await
    }

    async fn broadcast_round(&self, id: LichessId) -> Result<BroadcastRoundPlace> {
        watch::broadcast_round(self.services.lichess(), &id.0).await
    }

    async fn broadcast_alerts(&self) -> Result<Vec<BroadcastAlert>> {
        self.database
            .read_async(|ctx| library::broadcast_alerts(ctx.host, ctx.db))
            .await
    }

    async fn set_broadcast_alert(
        &self,
        tour_id: LichessId,
        on: bool,
        round_id: Option<LichessId>,
    ) -> Result<Vec<BroadcastAlert>> {
        let (tour, round) = (tour_id.0, round_id.map(|round| round.0));
        // Turning one on, or one round of a whole tournament's off, needs its rounds as they are.
        let detail = if on || round.is_some() {
            Some(watch::broadcast_tour(self.services.lichess(), &tour).await?)
        } else {
            None
        };
        let alerts = self
            .database
            .write_async(move |ctx| {
                library::change_broadcast_alerts(ctx.host, ctx.db, |alerts| {
                    watch::alerts::set(alerts, &tour, round.as_deref(), on, detail.as_ref())
                })
            })
            .await?;
        self.broadcast_alerts.wake();
        Ok(alerts)
    }
}
