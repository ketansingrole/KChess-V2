//! Lichess players: profiles, ratings, messages and relations.

use kchess_contracts::core_api::SocialApi;
use serde_json::{Value, json};

use super::args::{reply_json, reply_true, text, to_json};
use super::online::names;
use crate::core::Core;
use crate::error::Result;
use crate::lichess::accounts::Reply;

impl SocialApi for Core {
    async fn player_perf(&self, username: Value, perf: Value) -> Result<Value> {
        let lichess = self.services.lichess();
        to_json(lichess.player_perf(&text(&username), &text(&perf)).await)
    }

    async fn crosstable(&self, a: Value, b: Value) -> Result<Value> {
        to_json(
            self.services
                .lichess()
                .crosstable(&text(&a), &text(&b))
                .await,
        )
    }

    async fn recent_games(&self, username: Value, rated: Option<Value>) -> Result<Value> {
        let rated = rated.is_some_and(|rated| rated.as_bool() == Some(true));
        let lichess = self.services.lichess();
        to_json(lichess.recent_games(&text(&username), rated).await)
    }

    async fn send_message(&self, account: Value, username: Value, text_: Value) -> Result<Value> {
        let (account, name, body) = (text(&account), text(&username), text(&text_));
        let sent = self
            .services
            .lichess()
            .send_message(&account, &name, &body)
            .await?;
        Ok(match sent {
            Reply::Done(()) => json!({ "sent": true }),
            Reply::NeedsReconnect => json!({ "needsReconnect": true }),
        })
    }

    async fn relation(&self, account: Value, username: Value) -> Result<Value> {
        let lichess = self.services.lichess();
        reply_json(lichess.relation(&text(&account), &text(&username)).await)
    }

    async fn set_relation(&self, account: Value, username: Value, action: Value) -> Result<Value> {
        let (account, name, action) = (text(&account), text(&username), text(&action));
        reply_true(
            self.services
                .lichess()
                .set_relation(&account, &name, &action)
                .await,
        )
    }

    async fn profiles(&self, usernames: Value) -> Result<Value> {
        to_json(self.services.lichess().profiles(&names(&usernames)).await)
    }

    async fn leaderboard(&self, perf: Value) -> Result<Value> {
        to_json(self.services.lichess().leaderboard(&text(&perf)).await)
    }

    async fn search_players(&self, term: Value) -> Result<Value> {
        to_json(self.services.lichess().search_players(&text(&term)).await)
    }

    async fn activity(&self, username: Value) -> Result<Value> {
        to_json(self.services.lichess().activity(&text(&username)).await)
    }

    async fn timeline(&self) -> Result<Value> {
        to_json(self.services.lichess().timeline().await)
    }

    async fn import_game(&self, account: Value, pgn: Value) -> Result<Value> {
        let lichess = self.services.lichess();
        to_json(lichess.import_game(&text(&account), &text(&pgn)).await)
    }

    async fn bookmarked_games(&self, account: Value) -> Result<Value> {
        reply_json(
            self.services
                .lichess()
                .bookmarked_games(&text(&account))
                .await,
        )
    }
}
