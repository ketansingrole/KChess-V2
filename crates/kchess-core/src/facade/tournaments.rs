//! The facade's tournaments.

use super::args::{arg, normal, now_ms, text};
use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::Level;
use crate::store::library;
use serde_json::{Value, json};

impl Core {
    /// Forgets one account's membership of a tournament, or every account's without one.
    async fn forget_tournament(
        &self,
        system: &Value,
        id: &Value,
        account: Option<&Value>,
    ) -> Result<()> {
        let (system, id) = (text(system), text(id));
        let account = account.map(text);
        self.database
            .write_async(move |ctx| {
                library::forget_tournament(ctx.host, ctx.db, &system, &id, account.as_deref())
            })
            .await
    }

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
                        vec![system.clone(), id.clone(), account.clone(), json!(page)],
                    )
                    .await?;
                if system.as_str() == Some("arena") {
                    // An arena that is over is over for every account.
                    if detail["status"].as_str() == Some("finished") {
                        self.forget_tournament(&system, &id, None).await?;
                    } else if account.as_str().is_some_and(|name| !name.is_empty())
                        && (detail["me"].is_null()
                            || detail["me"]["withdraw"].as_bool() == Some(true))
                    {
                        // `me` is the looked-up account's place only: another account's stays.
                        self.forget_tournament(&system, &id, Some(&account)).await?;
                    }
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
                    .write_async(move |ctx| {
                        library::remember_tournament(ctx.host, ctx.db, &entry, now as f64)
                    })
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
                        vec![system.clone(), id.clone(), account.clone()],
                    )
                    .await?;
                if result == json!(true) {
                    self.forget_tournament(&system, &id, Some(&account)).await?;
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
