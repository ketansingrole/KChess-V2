//! Synced games, profiles and friends.

use std::sync::Arc;

use kchess_contracts::check::{FriendList, GameAccount, GameId, OptionalAccount, Username};
use kchess_contracts::core_api::GamesApi;
use kchess_contracts::types::{
    AppData, CachedProfile, ConnectedLichess, FollowingReport, GameLibraryOverview, GamePage,
    GamePageQuery, InsightsQuery, InsightsReport, LichessUser, RatingSeries,
};

use crate::core::Core;
use crate::store::games;
use crate::store::insights;
use crate::time::now_ms;
use kchess_contracts::error::{CoreError, Result};

impl GamesApi for Core {
    async fn sync_games(&self, username: Option<Username>) -> Result<AppData> {
        let username = username.map(|name| name.0);
        self.services
            .lichess()
            .sync_games(username.as_deref())
            .await?;
        self.app_data().await
    }

    async fn game_page(&self, query: GamePageQuery) -> Result<GamePage> {
        self.database
            .read_async(move |ctx| games::game_page(ctx, &query))
            .await
    }

    async fn game_library_overview(&self) -> Result<GameLibraryOverview> {
        self.database.read_async(games::game_library_overview).await
    }

    async fn insights(&self, query: InsightsQuery) -> Result<InsightsReport> {
        let now = now_ms() as f64;
        let tablebases = self.tablebases.clone();
        // A filter shown before is answered as it was, until its games or reviews change. Else
        // its games are read, and replayed for their endgames and structures off the database,
        // which meanwhile serves every other call.
        let read = self
            .database
            .read_async(move |ctx| {
                let tables = tablebases.tables();
                let key = insights::report_key(ctx.db, &query, tables.as_ref())?;
                if let Some(report) = ctx.cache.insights(ctx.db, &key, now) {
                    return Ok(Insights::Kept(report));
                }
                Ok(Insights::Read(InsightsRead {
                    stamp: ctx.cache.insights_stamp(ctx.db),
                    report: insights::insights(ctx.host, ctx.db, &query, now)?,
                    rows: insights::replay_rows(ctx.db, &query, now)?,
                    until: insights::unchanged_until(ctx.db, &query, now)?,
                    key,
                    tables,
                }))
            })
            .await?;
        let mut read = match read {
            Insights::Read(read) => read,
            Insights::Kept(report) => return Ok(report),
        };
        let (rows, tables) = (read.rows.take(), read.tables.take());
        let pinned = tables.clone();
        (read.report.endgames, read.report.structures) = tokio::task::spawn_blocking(move || {
            insights::replay(rows.as_deref(), tables.as_deref())
        })
        .await
        .map_err(|cause| CoreError::wrap("Insights replay failed", cause))??;
        self.database.cache().remember_insights(
            read.stamp,
            read.key,
            pinned.as_ref(),
            now,
            read.until,
            &read.report,
        );
        Ok(read.report)
    }

    async fn game_rating_history(&self, account: GameAccount) -> Result<Vec<RatingSeries>> {
        self.database
            .read_async(move |ctx| games::rating_history(ctx.db, &account.0))
            .await
    }

    async fn game_pgn(&self, account: GameAccount, id: GameId) -> Result<Option<String>> {
        self.database
            .read_async(move |ctx| games::game_pgn(ctx.db, &account.0, &id.0))
            .await
    }

    async fn cached_profile(&self, username: Username) -> Result<CachedProfile> {
        self.services.lichess().cached_profile(&username.0).await
    }

    async fn profile(&self, username: Username) -> Result<LichessUser> {
        self.services.lichess().profile(&username.0).await
    }

    async fn rating_history(&self, username: Username) -> Result<Vec<RatingSeries>> {
        self.services.lichess().rating_history(&username.0).await
    }

    async fn connect_lichess(&self) -> Result<ConnectedLichess> {
        // The window is focused when the browser returns.
        let capabilities = Arc::clone(&self.capabilities);
        let focus = move || {
            let capabilities = Arc::clone(&capabilities);
            tokio::spawn(async move {
                let _ = capabilities.focus().await;
            });
        };
        let connected = self
            .services
            .lichess()
            .connect_lichess(Some(&focus))
            .await?;
        Ok(ConnectedLichess {
            data: self.app_data().await?,
            username: connected.username,
        })
    }

    async fn stay_connected(&self, account: OptionalAccount) -> Result<()> {
        let wanted = account.0;
        let connected = self.is_connected(&wanted).await?;
        self.services
            .online()
            .stay_connected(if connected { &wanted } else { "" });
        Ok(())
    }

    async fn clear_account_data(&self, username: Username) -> Result<AppData> {
        let account = username.0;
        let lichess = self.services.lichess();
        lichess.cancel_account_syncs(std::slice::from_ref(&account));
        self.reviews
            .discard_account_reviews(std::slice::from_ref(&account));
        let cleared = account.clone();
        let data = self
            .database
            .write_async(move |ctx| games::clear_account_data(ctx.db, &cleared))
            .await?;
        lichess.forget_profile(&account);
        Ok(data)
    }

    async fn following(&self) -> Result<FollowingReport> {
        self.services.lichess().followed_users().await
    }

    async fn add_friends(&self, usernames: FriendList) -> Result<AppData> {
        let names = usernames.0;
        let list = names.clone();
        let data = self
            .database
            .write_async(move |ctx| games::add_friends(ctx.db, &list))
            .await?;
        self.services.lichess().prime_profiles(&names).await?;
        Ok(data)
    }
}

/// An Insights report as the database answers it: kept from before, or read with its games to
/// replay.
enum Insights {
    Kept(InsightsReport),
    Read(InsightsRead),
}

/// An Insights report read from the database, its games still to replay.
struct InsightsRead {
    stamp: crate::store::InsightsStamp,
    key: String,
    report: InsightsReport,
    rows: Option<Vec<insights::ReplayRow>>,
    until: f64,
    tables: Option<std::sync::Arc<shakmaty_syzygy::Tablebase<shakmaty::Chess>>>,
}
