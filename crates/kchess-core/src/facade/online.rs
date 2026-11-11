//! Online play, challenges and Lichess players.

use kchess_contracts::core_api::OnlineApi;
use serde_json::{Value, json};

use super::args::{parse, reply_json, reply_true, text, to_json};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::lichess::accounts::Reply;
use crate::lichess::challenges::ChallengeInfo;

/// Names the check passed, as the services take them.
fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(text)
        .collect()
}

impl Core {
    /// The open challenge `id`, or the error it is no longer open.
    fn known_challenge(&self, id: &str) -> Result<ChallengeInfo> {
        let found = self.services.online().challenges().get(id);
        found.ok_or_else(|| CoreError::new("That challenge is no longer open."))
    }
}

impl OnlineApi for Core {
    async fn start_online(&self, options: Value) -> Result<Value> {
        let options = parse(options, "options")?;
        to_json(self.services.online().start(&options).await)
    }

    async fn resume_online(&self) -> Result<Value> {
        to_json(self.services.online().resume().await.map(|resumed| {
            resumed.map(|found| json!({ "id": found.id, "account": found.account }))
        }))
    }

    async fn cancel_online(&self) -> Result<Value> {
        self.services.online().cancel();
        Ok(Value::Null)
    }

    async fn play_online(&self, id: Value, r#move: Value) -> Result<Value> {
        let online = self.services.online();
        online.move_uci(&text(&id), &text(&r#move)).await?;
        Ok(Value::Null)
    }

    async fn play_correspondence(
        &self,
        account: Value,
        id: Value,
        fen: Value,
        r#move: Value,
    ) -> Result<Value> {
        let (account, id, fen, uci) = (text(&account), text(&id), text(&fen), text(&r#move));
        let online = self.services.online();
        online
            .move_correspondence(&account, &id, &fen, &uci)
            .await?;
        Ok(Value::Null)
    }

    async fn online_action(&self, id: Value, action: Value) -> Result<Value> {
        let action = parse(action, "action")?;
        self.services.online().action(&text(&id), action).await?;
        Ok(Value::Null)
    }

    async fn online_chat(&self, id: Value) -> Result<Value> {
        to_json(self.services.online().chat(&text(&id)).await)
    }

    async fn send_chat(&self, id: Value, room: Value, text_: Value) -> Result<Value> {
        let (id, room, body) = (text(&id), text(&room), text(&text_));
        self.services.online().send_chat(&id, &room, &body).await?;
        Ok(Value::Null)
    }

    async fn presence(&self, usernames: Value) -> Result<Value> {
        to_json(self.services.online().presence(&names(&usernames)).await)
    }

    async fn challenges(&self) -> Result<Value> {
        to_json(Ok::<_, CoreError>(
            self.services.online().challenges().list(),
        ))
    }

    async fn accept_challenge(&self, id: Value) -> Result<Value> {
        let id = text(&id);
        let challenge = self.known_challenge(&id)?;
        let shown = json!(challenge);
        if shown["direction"].as_str() != Some("in") || shown["playable"].as_bool() != Some(true) {
            let problem = shown["problem"]
                .as_str()
                .unwrap_or("Only challenges sent to you can be accepted.");
            return Err(CoreError::new(problem));
        }
        self.services.online().accept_challenge(&challenge).await?;
        self.services.online().challenges().remove(&id);
        Ok(Value::Null)
    }

    async fn decline_challenge(&self, id: Value, reason: Value) -> Result<Value> {
        let id = text(&id);
        let challenge = self.known_challenge(&id)?;
        if json!(challenge)["direction"].as_str() != Some("in") {
            return Err(CoreError::new("Withdraw your own challenge instead."));
        }
        let online = self.services.online();
        online.decline_challenge(&challenge, &text(&reason)).await?;
        online.challenges().remove(&id);
        Ok(Value::Null)
    }

    async fn cancel_challenge(&self, id: Value) -> Result<Value> {
        let id = text(&id);
        let challenge = self.known_challenge(&id)?;
        if json!(challenge)["direction"].as_str() != Some("out") {
            return Err(CoreError::new("Decline a challenge sent to you instead."));
        }
        self.services
            .online()
            .withdraw_challenge(&challenge)
            .await?;
        self.services.online().challenges().remove(&id);
        Ok(Value::Null)
    }

    async fn ongoing_games(&self) -> Result<Value> {
        to_json(self.services.online().ongoing().await)
    }

    async fn open_game(&self, account: Value, id: Value) -> Result<Value> {
        let online = self.services.online();
        online.open(&text(&account), &text(&id)).await?;
        Ok(Value::Null)
    }

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
