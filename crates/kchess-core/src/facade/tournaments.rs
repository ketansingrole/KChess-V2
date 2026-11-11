//! Lichess tournaments.

use kchess_contracts::core_api::TournamentsApi;
use serde_json::{Value, json};

use super::args::{now_ms, parse, reply_json, reply_true, text, to_json};
use crate::core::Core;
use crate::error::Result;
use crate::host::Level;
use crate::lichess::tournaments;
use crate::store::library;

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
}

impl Core {
    /// One tournament's details, as the account sees it.
    async fn tournament_detail(
        &self,
        system: &Value,
        id: &Value,
        account: &Value,
        page: u32,
    ) -> Result<Value> {
        let (system, id, account) = (text(system), text(id), text(account));
        let lichess = self.services.lichess();
        to_json(tournaments::tournament(lichess, &system, &id, &account, page).await)
    }
}

impl TournamentsApi for Core {
    async fn tournaments(&self, account: Value) -> Result<Value> {
        to_json(tournaments::tournaments(self.services.lichess(), &text(&account)).await)
    }

    async fn tournament(
        &self,
        system: Value,
        id: Value,
        account: Value,
        page: Option<Value>,
    ) -> Result<Value> {
        let page = match page.as_ref().and_then(Value::as_i64) {
            Some(page) if (1..=200).contains(&page) => page as u32,
            _ => 1,
        };
        let detail = self.tournament_detail(&system, &id, &account, page).await?;
        if system.as_str() == Some("arena") {
            // An arena that is over is over for every account.
            if detail["status"].as_str() == Some("finished") {
                self.forget_tournament(&system, &id, None).await?;
            } else if account.as_str().is_some_and(|name| !name.is_empty())
                && (detail["me"].is_null() || detail["me"]["withdraw"].as_bool() == Some(true))
            {
                // `me` is the looked-up account's place only: another account's stays.
                self.forget_tournament(&system, &id, Some(&account)).await?;
            }
        }
        Ok(detail)
    }

    async fn join_tournament(
        &self,
        system: Value,
        id: Value,
        account: Value,
        password: Option<Value>,
    ) -> Result<Value> {
        let name = account;
        let password = password.as_ref().and_then(Value::as_str);
        let result = reply_true(
            tournaments::join_tournament(
                self.services.lichess(),
                &text(&system),
                &text(&id),
                &text(&name),
                password,
            )
            .await,
        )?;
        if result != json!(true) {
            return Ok(result);
        }
        // Pairings arrive on the event stream: keep it open while in the tournament.
        self.services.online().stay_connected(&text(&name));
        let now = now_ms();
        // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
        let mut entry = json!({
            "system": system,
            "id": id,
            "account": name,
            "name": id,
            "until": now + 86_400_000i64,
        });
        match self.tournament_detail(&system, &id, &name, 1).await {
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

    async fn leave_tournament(&self, system: Value, id: Value, account: Value) -> Result<Value> {
        let left = tournaments::leave_tournament(
            self.services.lichess(),
            &text(&system),
            &text(&id),
            &text(&account),
        )
        .await;
        let result = reply_true(left)?;
        if result == json!(true) {
            self.forget_tournament(&system, &id, Some(&account)).await?;
        }
        Ok(result)
    }

    async fn create_tournament(&self, account: Value, arena: Value) -> Result<Value> {
        let arena = parse(arena, "arena")?;
        let lichess = self.services.lichess();
        reply_json(tournaments::create_tournament(lichess, &text(&account), &arena).await)
    }
}
