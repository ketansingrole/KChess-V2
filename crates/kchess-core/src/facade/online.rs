//! Online play and challenges.

use kchess_contracts::core_api::OnlineApi;
use serde_json::{Value, json};

use super::args::{parse, text, to_json};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::lichess::challenges::ChallengeInfo;

/// Names the check passed, as the services take them.
pub(super) fn names(value: &Value) -> Vec<String> {
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
}
