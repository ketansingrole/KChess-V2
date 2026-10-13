//! The facade's tournaments (`run_facade` arms, moved without changes).

use super::args::{arg, normal, now_ms};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use serde_json::{Value, json};

impl Core {
    /// Answers the facade's tournaments methods.
    pub(crate) async fn run_tournaments(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "tournaments" => {
                let account = normal("assertOptionalAccount", &arg(args, 0))?;
                self.inner("tournaments.list", vec![account]).await
            }
            "tournament" => {
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let account = normal("assertOptionalAccount", &arg(args, 2))?;
                let page = match arg(args, 3).as_i64() {
                    Some(page) if (1..=200).contains(&page) => page,
                    _ => 1,
                };
                let detail = self
                    .inner(
                        "tournaments.get",
                        vec![system.clone(), id.clone(), account, json!(page)],
                    )
                    .await?;
                // An arena says whether you are in it; forget it when it is over or you left.
                let withdrew = detail["me"]["withdraw"].as_bool() == Some(true);
                if system.as_str() == Some("arena")
                    && (detail["status"].as_str() == Some("finished")
                        || detail["me"].is_null()
                        || withdrew)
                {
                    self.inner("store.library.forgetTournament", vec![system, id])
                        .await?;
                }
                Ok(detail)
            }
            "joinTournament" => {
                let name = normal("assertUsername", &arg(args, 2))?;
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let password = normal("assertTournamentPassword", &arg(args, 3))?;
                let result = self
                    .inner(
                        "tournaments.join",
                        vec![system.clone(), id.clone(), name.clone(), password],
                    )
                    .await?;
                if result != json!(true) {
                    return Ok(result);
                }
                // Pairings arrive on the event stream: keep it open while in the tournament.
                self.inner("online.stayConnected", vec![name.clone()])
                    .await?;
                let now = now_ms();
                // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
                let mut entry = json!({
                    "system": system,
                    "id": id,
                    "account": name,
                    "name": id,
                    "until": now + 86_400_000i64,
                });
                match self
                    .inner(
                        "tournaments.get",
                        vec![system.clone(), id.clone(), name.clone(), json!(1)],
                    )
                    .await
                {
                    Ok(detail) => {
                        entry["name"] = detail["name"].clone();
                        entry["until"] = if detail["finishesAt"].is_null() {
                            entry["until"].clone()
                        } else {
                            detail["finishesAt"].clone()
                        };
                    }
                    Err(cause) => self.host.log(
                        Level::Warn,
                        "tournaments",
                        &format!(
                            "Joined tournament details are unavailable: id={} {}",
                            id.as_str().unwrap_or_default(),
                            cause.message
                        ),
                    ),
                }
                self.database
                    .call_async("store.library.rememberTournament", &[entry, json!(now)])
                    .await?;
                Ok(result)
            }
            "leaveTournament" => {
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let account = normal("assertUsername", &arg(args, 2))?;
                let result = self
                    .inner(
                        "tournaments.leave",
                        vec![system.clone(), id.clone(), account],
                    )
                    .await?;
                if result == json!(true) {
                    self.inner("store.library.forgetTournament", vec![system, id])
                        .await?;
                }
                Ok(result)
            }
            "createTournament" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let arena = normal("assertNewArena", &arg(args, 1))?;
                self.inner("tournaments.create", vec![account, arena]).await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
