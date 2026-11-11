//! Lichess TV and broadcasts.

use kchess_contracts::check::{BroadcastQuery, LichessId, WatchSession};
use kchess_contracts::core_api::SpectateApi;
use kchess_contracts::types::{
    BroadcastRoundPlace, BroadcastSummary, BroadcastTourDetail, TvChannel, WatchTarget,
};

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::lichess::watch;

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
}
