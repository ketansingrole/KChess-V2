//! Lichess TV and broadcasts.

use kchess_contracts::core_api::SpectateApi;
use serde_json::{Value, json};

use super::args::{text, to_json};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::lichess::watch::{self, WatchTarget};

/// `{ channel }` or `{ gameId }`: what the spectator watches.
fn watch_target(value: &Value) -> Result<WatchTarget> {
    if let Some(channel) = value.get("channel").and_then(Value::as_str) {
        return Ok(WatchTarget::Channel(channel.to_string()));
    }
    if let Some(game) = value.get("gameId").and_then(Value::as_str) {
        return Ok(WatchTarget::Game(game.to_string()));
    }
    Err(CoreError::new("Invalid watch target."))
}

impl Core {
    fn refuse_watch_while_playing(&self) -> Result<()> {
        if self.services.online().playing() {
            return Err(CoreError::new("Finish your game before watching another."));
        }
        Ok(())
    }
}

impl SpectateApi for Core {
    async fn tv_channels(&self) -> Result<Value> {
        to_json(watch::tv_channels(self.services.lichess()).await)
    }

    async fn watch(&self, target: Value) -> Result<Value> {
        self.refuse_watch_while_playing()?;
        let target = watch_target(&target)?;
        Ok(json!(self.services.spectator().mine().watch(target)))
    }

    async fn watch_broadcast(&self, round_id: Value) -> Result<Value> {
        self.refuse_watch_while_playing()?;
        let round = text(&round_id);
        Ok(json!(self.services.spectator().mine().watch_round(&round)))
    }

    async fn stop_watching(&self, session: Option<Value>) -> Result<Value> {
        let session = session.as_ref().and_then(Value::as_u64);
        self.services.spectator().stop_mine(session);
        Ok(Value::Null)
    }

    async fn broadcasts(&self, query: Option<Value>) -> Result<Value> {
        let query = query.as_ref().and_then(Value::as_str);
        to_json(watch::broadcasts(self.services.lichess(), query).await)
    }

    async fn broadcast_tour(&self, id: Value) -> Result<Value> {
        to_json(watch::broadcast_tour(self.services.lichess(), &text(&id)).await)
    }

    async fn broadcast_round(&self, id: Value) -> Result<Value> {
        watch::broadcast_round(self.services.lichess(), &text(&id)).await
    }
}
