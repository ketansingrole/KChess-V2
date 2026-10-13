//! The facade's online play, challenges and Lichess players (`run_facade` arms, moved without changes).

use super::args::{arg, normal};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's online methods.
    pub(crate) async fn run_online(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "startOnline" => {
                let options = normal("assertOnlineOptions", &arg(args, 0))?;
                self.inner("online.start", vec![options]).await
            }
            "resumeOnline" => self.inner("online.resume", vec![]).await,
            "cancelOnline" => self.inner("online.cancel", vec![]).await,
            "playOnline" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let uci = normal("assertUci", &arg(args, 1))?;
                self.inner("online.move", vec![id, uci]).await
            }
            "onlineAction" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let action = normal("assertAction", &arg(args, 1))?;
                self.inner("online.action", vec![id, action]).await
            }
            "presence" => {
                let usernames = normal("assertUsernames", &arg(args, 0))?;
                self.inner("online.presence", vec![usernames]).await
            }
            "onlineChat" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                self.inner("online.chat", vec![id]).await
            }
            "sendChat" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let room = normal("assertChatRoom", &arg(args, 1))?;
                let text = normal("assertChatText", &arg(args, 2))?;
                self.inner("online.sendChat", vec![id, room, text]).await
            }
            "challenges" => self.inner("challenges.list", vec![]).await,
            "acceptChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("in")
                    || challenge["playable"].as_bool() != Some(true)
                {
                    let problem = challenge["problem"]
                        .as_str()
                        .unwrap_or("Only challenges sent to you can be accepted.");
                    return Err(CoreError::new(problem));
                }
                self.inner("online.acceptChallenge", vec![challenge])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "declineChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("in") {
                    return Err(CoreError::new("Withdraw your own challenge instead."));
                }
                let reason = normal("assertDeclineReason", &arg(args, 1))?;
                self.inner("online.declineChallenge", vec![challenge, reason])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "cancelChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("out") {
                    return Err(CoreError::new("Decline a challenge sent to you instead."));
                }
                self.inner("online.withdrawChallenge", vec![challenge])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "ongoingGames" => self.inner("online.ongoing", vec![]).await,
            "openGame" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let id = normal("assertGameId", &arg(args, 1))?;
                self.inner("online.open", vec![account, id]).await
            }
            "playerPerf" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let perf = normal("assertPerfType", &arg(args, 1))?;
                self.inner("lichess.playerPerf", vec![name, perf]).await
            }
            "crosstable" => {
                let a = normal("assertUsername", &arg(args, 0))?;
                let b = normal("assertUsername", &arg(args, 1))?;
                self.inner("lichess.crosstable", vec![a, b]).await
            }
            "recentGames" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let rated = arg(args, 1).as_bool() == Some(true);
                self.inner("lichess.recentGames", vec![name, json!(rated)])
                    .await
            }
            "sendMessage" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let username = normal("assertUsername", &arg(args, 1))?;
                let text = normal("assertMessageText", &arg(args, 2))?;
                self.inner("lichess.sendMessage", vec![account, username, text])
                    .await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
