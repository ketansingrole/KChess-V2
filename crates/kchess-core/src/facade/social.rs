//! Lichess players: profiles, ratings, messages and relations.

use kchess_contracts::check::{FriendList, MessageText, OptionalAccount, PgnText, Username};
use kchess_contracts::core_api::SocialApi;
use kchess_contracts::types::{
    ActivityDay, Crosstable, ImportedGame, LeaderboardEntry, LichessGame, LichessRelation,
    MessageSent, PerfStats, PerfType, PlayerSuggestion, RelationAction, Reply, TimelineEntry,
};
use serde_json::Value;

use crate::core::Core;
use kchess_contracts::error::Result;

impl SocialApi for Core {
    async fn player_perf(&self, username: Username, perf: PerfType) -> Result<PerfStats> {
        self.services.lichess().player_perf(&username.0, perf).await
    }

    async fn crosstable(&self, a: Username, b: Username) -> Result<Crosstable> {
        self.services.lichess().crosstable(&a.0, &b.0).await
    }

    async fn recent_games(
        &self,
        username: Username,
        rated: Option<bool>,
    ) -> Result<Vec<LichessGame>> {
        let lichess = self.services.lichess();
        lichess.recent_games(&username.0, rated == Some(true)).await
    }

    async fn send_message(
        &self,
        account: Username,
        username: Username,
        text: MessageText,
    ) -> Result<Reply<MessageSent>> {
        let sent = self
            .services
            .lichess()
            .send_message(&account.0, &username.0, &text.0)
            .await?;
        Ok(match sent {
            Reply::Done(()) => Reply::Done(MessageSent { sent: true }),
            Reply::NeedsReconnect => Reply::NeedsReconnect,
        })
    }

    async fn relation(
        &self,
        account: Username,
        username: Username,
    ) -> Result<Reply<LichessRelation>> {
        self.services
            .lichess()
            .relation(&account.0, &username.0)
            .await
    }

    async fn set_relation(
        &self,
        account: Username,
        username: Username,
        action: RelationAction,
    ) -> Result<Reply<bool>> {
        let done = self
            .services
            .lichess()
            .set_relation(&account.0, &username.0, action)
            .await?;
        Ok(match done {
            Reply::Done(()) => Reply::Done(true),
            Reply::NeedsReconnect => Reply::NeedsReconnect,
        })
    }

    async fn profiles(&self, usernames: FriendList) -> Result<Vec<Value>> {
        self.services.lichess().profiles(&usernames.0).await
    }

    async fn leaderboard(&self, perf: PerfType) -> Result<Vec<LeaderboardEntry>> {
        self.services.lichess().leaderboard(perf).await
    }

    async fn search_players(&self, term: Username) -> Result<Vec<PlayerSuggestion>> {
        self.services.lichess().search_players(&term.0).await
    }

    async fn activity(&self, username: Username) -> Result<Vec<ActivityDay>> {
        self.services.lichess().activity(&username.0).await
    }

    async fn timeline(&self) -> Result<Vec<TimelineEntry>> {
        self.services.lichess().timeline().await
    }

    async fn import_game(&self, account: OptionalAccount, pgn: PgnText) -> Result<ImportedGame> {
        self.services
            .lichess()
            .import_game(&account.0, &pgn.0)
            .await
    }

    async fn bookmarked_games(&self, account: Username) -> Result<Reply<Vec<LichessGame>>> {
        self.services.lichess().bookmarked_games(&account.0).await
    }
}
