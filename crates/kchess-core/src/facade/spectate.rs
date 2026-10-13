//! The facade's spectating and broadcasts (`run_facade` arms, moved without changes).

use super::args::{arg, normal, normal_args};
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's spectate methods.
    pub(crate) async fn run_spectate(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "tvChannels" => self.inner("spectate.tvChannels", vec![]).await,
            "watch" => {
                if self.services.online().playing() {
                    return Err(CoreError::new("Finish your game before watching another."));
                }
                let target = normal_args(
                    "assertWatchTarget",
                    &[arg(args, 0), json!(kchess_domain::misc::TV_CHANNEL_KEYS)],
                )?;
                self.inner("spectate.watch", vec![target]).await
            }
            "watchBroadcast" => {
                if self.services.online().playing() {
                    return Err(CoreError::new("Finish your game before watching another."));
                }
                let round = normal("assertLichessId", &arg(args, 0))?;
                self.inner("spectate.watchRound", vec![round]).await
            }
            "stopWatching" => {
                self.services.spectator().stop();
                Ok(Value::Null)
            }
            "broadcasts" => {
                let query = normal("assertBroadcastQuery", &arg(args, 0))?;
                self.inner("spectate.broadcasts", vec![query]).await
            }
            "broadcastTour" => {
                let id = normal("assertLichessId", &arg(args, 0))?;
                self.inner("spectate.broadcastTour", vec![id]).await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
