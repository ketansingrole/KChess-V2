//! The live online game: its stream, moves, chat, presence, actions, logout and close (moved
//! from `session.rs` without changes).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::{Value, json};

use crate::lichess::accounts::path_segment;
use crate::lichess::challenges::is_game_id;
use crate::lichess::client::authorize;

use crate::lichess::online::events::{game_in_progress, truncate};
use crate::lichess::online::ongoing::finished_check;
use crate::lichess::online::requests::{Emitter, Listen, StreamRequest};
use crate::lichess::online::state::{
    ChatLine, Lane, OnlineAction, OnlineSession, PRESENCE_MAX, PRESENCE_TTL, PresenceReport, Res,
    UserPresence, core,
};

impl OnlineSession {
    /// Start the board's stream of game `id` (`openGame`). The state is set before this returns,
    /// so the game counts as played at once; the stream itself runs on a task.
    pub(super) fn begin_game(&self, id: &str, token: &str) {
        let (handle, account, session) = {
            let mut st = self.st();
            if let Some(previous) = st.game_controller.take() {
                previous.token.cancel();
            }
            let handle = self.next_handle(&mut st);
            st.game_controller = Some(handle.clone());
            st.live_game = id.to_string();
            st.live_correspondence = false;
            st.protected_game = id.to_string();
            st.protected_account = st.current_account.clone();
            (handle, st.current_account.clone(), st.epoch)
        };
        let finished = Arc::new(AtomicBool::new(false));
        let this = self.clone();
        let (game_id, finished_for_emit) = (id.to_string(), Arc::clone(&finished));
        let emit: Emitter = Arc::new(move |event: Value| {
            this.on_game_event(&game_id, &finished_for_emit, event);
        });
        let options = Listen {
            lane: Lane::Game,
            reconnect: true,
            finished: Some(Arc::clone(&finished)),
            quiet: None,
            session,
            account: account.clone(),
            game_id: id.to_string(),
        };
        let request = StreamRequest {
            method: reqwest::Method::GET,
            path: format!("/api/board/game/stream/{}", path_segment(id)),
            query: Vec::new(),
            auth: Some(authorize(token)),
            form: None,
        };
        let this = self.clone();
        let game_id = id.to_string();
        let policy = self.policy();
        let cancel = handle.token.clone();
        let handle_id = handle.id;
        tokio::spawn(async move {
            policy
                .with_usage(&account, "play", async {
                    this.listen(request, cancel, emit, options).await;
                })
                .await;
            // `.finally`: the live game ends when its stream ended for a finished game, and this
            // stream is still the game's controller.
            let mut st = this.st();
            if finished.load(Ordering::SeqCst)
                && finished_check(&st, handle_id)
                && st.live_game == game_id
            {
                st.live_game.clear();
            }
        });
    }

    fn on_game_event(&self, game_id: &str, finished: &AtomicBool, event: Value) {
        // Lichess closes the game stream when the game ends; don't reconnect then.
        let kind = event["type"].as_str().unwrap_or("").to_string();
        if kind == "gameFull" {
            let correspondence = event["speed"].as_str() == Some("correspondence");
            self.st().live_correspondence = correspondence;
        }
        let status = match kind.as_str() {
            "gameFull" => event["state"]["status"].as_str(),
            "gameState" => event["status"].as_str(),
            _ => None,
        };
        let over = status.is_some_and(|status| !status.is_empty() && !game_in_progress(status));
        if over {
            finished.store(true, Ordering::SeqCst);
            let mut st = self.st();
            st.live_game.clear();
            if st.protected_game == game_id {
                st.protected_game.clear();
            }
        }
        let mut payload = event;
        payload["id"] = json!(game_id);
        self.emit("online:event", payload);
    }

    /// Sets the hook run before each state is reported, given whether a live game is in progress
    /// and whether assistance is blocked (see `Inner::state_hook`).
    pub fn set_state_hook(&self, hook: Arc<dyn Fn(bool, bool) + Send + Sync>) {
        if let Ok(mut slot) = self.inner.state_hook.lock() {
            *slot = Some(hook);
        }
    }

    /// Runs the state hook, if any; called without the state lock held.
    pub(super) fn run_state_hook(&self, live_game: bool, blocked: bool) {
        let hook = self
            .inner
            .state_hook
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(hook) = hook {
            hook(live_game, blocked);
        }
    }

    /// Whether a game is being played: the board, or a protected game not yet verified gone.
    pub fn playing(&self) -> bool {
        let st = self.st();
        !st.live_game.is_empty() || !st.protected_game.is_empty()
    }

    /// Unknown startup or recovery state also blocks assistance, even before a game id is known.
    pub fn assistance_blocked(&self) -> bool {
        let st = self.st();
        st.recovery_unverified || !st.live_game.is_empty() || !st.protected_game.is_empty()
    }

    /// The token and account of the live game (`liveToken`).
    async fn live_token(&self, id: &str, what: &str) -> Res<(String, String)> {
        let account = {
            let st = self.st();
            if id != st.live_game {
                return Err(core(format!("Reconnect to this game before {what}.")));
            }
            st.current_account.clone()
        };
        let token = self
            .token(&account)
            .await?
            .ok_or_else(|| core("Lichess login unavailable."))?;
        Ok((account, token))
    }

    /// Send one move (`move`). The move is sent once: an ambiguous answer is an error, never a
    /// retry.
    pub async fn move_uci(&self, id: &str, uci: &str) -> Res<()> {
        let (account, token) = self.live_token(id, "sending a move").await?;
        let path = format!(
            "/api/board/game/{}/move/{}",
            path_segment(id),
            path_segment(uci)
        );
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(&path, &[], Some(&authorize(&token)), &cancel)
                    .await
            })
            .await?;
        Ok(())
    }

    /// The private chat between the two players so far (the last 200 lines).
    pub async fn chat(&self, id: &str) -> Res<Vec<ChatLine>> {
        let (account, token) = self.live_token(id, "reading the chat").await?;
        let path = format!("/api/board/game/{}/chat", path_segment(id));
        let cancel = self.inner.lichess.lifetime().clone();
        let lines: Value = self
            .policy()
            .with_usage(&account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .get_json::<Value>(&path, &[], Some(&authorize(&token)), &cancel)
                    .await
            })
            .await?;
        let all = lines.as_array().cloned().unwrap_or_default();
        let start = all.len().saturating_sub(200);
        Ok(all[start..]
            .iter()
            .filter_map(|line| {
                let user = line.get("user")?.as_str()?;
                let text = line.get("text")?.as_str()?;
                Some(ChatLine {
                    user: truncate(user, 40),
                    text: truncate(text, 400),
                    room: "player".to_string(),
                })
            })
            .collect())
    }

    /// Send a chat line to a room of the live game.
    pub async fn send_chat(&self, id: &str, room: &str, text: &str) -> Res<()> {
        let (account, token) = self.live_token(id, "chatting").await?;
        let path = format!("/api/board/game/{}/chat", path_segment(id));
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(
                        &path,
                        &[("room", room), ("text", text)],
                        Some(&authorize(&token)),
                        &cancel,
                    )
                    .await
            })
            .await?;
        Ok(())
    }

    /// Lichess's online/playing/signal flags for some users, plus how long the request took. Works
    /// with no game in progress too (the Friends page), just without a login.
    pub async fn presence(&self, usernames: &[String]) -> Res<PresenceReport> {
        let account = self.st().current_account.clone();
        let mut ids: Vec<String> = usernames.iter().map(|name| name.to_lowercase()).collect();
        ids.sort();
        ids.dedup();
        let key = format!("{account}|{}", ids.join(","));
        {
            let mut st = self.st();
            st.presence
                .retain(|(_, stored, _)| stored.elapsed() < PRESENCE_TTL);
            if let Some((_, _, report)) = st.presence.iter().find(|(known, _, _)| *known == key) {
                return Ok(report.clone());
            }
        }
        let report = self.fetch_presence(&ids, &account).await?;
        let mut st = self.st();
        st.presence.retain(|(known, _, _)| *known != key);
        if st.presence.len() >= PRESENCE_MAX {
            st.presence.remove(0);
        }
        st.presence.push((key, Instant::now(), report.clone()));
        Ok(report)
    }

    async fn fetch_presence(&self, ids: &[String], account: &str) -> Res<PresenceReport> {
        let token = if account.is_empty() {
            None
        } else {
            self.token(account).await?
        };
        let started = Instant::now();
        let query = [
            ("ids", ids.join(",")),
            ("withSignal", "true".to_string()),
            ("withGameIds", "true".to_string()),
        ];
        let cancel = self.inner.lichess.lifetime().clone();
        let rows: Value = self
            .policy()
            .with_usage(account, "presence", async {
                self.inner
                    .lichess
                    .client()
                    .get_json::<Value>(
                        "/api/users/status",
                        &query,
                        token.as_deref().map(authorize).as_deref(),
                        &cancel,
                    )
                    .await
            })
            .await?;
        let latency_ms = (started.elapsed().as_secs_f64() * 1000.0).round() as u64;
        let mut users = BTreeMap::new();
        for row in rows.as_array().cloned().unwrap_or_default() {
            let Some(id) = row["id"].as_str() else {
                continue;
            };
            let playing_id = row["playingId"]
                .as_str()
                .filter(|game| is_game_id(game))
                .map(str::to_string);
            users.insert(
                id.to_lowercase(),
                UserPresence {
                    online: row["online"].as_bool().unwrap_or(false),
                    playing: row["playing"].as_bool().unwrap_or(false),
                    signal: row["signal"].as_number().cloned(),
                    playing_id,
                },
            );
        }
        Ok(PresenceReport { users, latency_ms })
    }

    /// An action on the live game: resign, abort, draw, takeback, claim, berserk. Each is sent
    /// once.
    pub async fn action(&self, id: &str, action: OnlineAction) -> Res<()> {
        let (account, token) = self.live_token(id, "sending an action").await?;
        let game = path_segment(id);
        let (path, form): (String, Vec<(&str, &str)>) = match action {
            OnlineAction::Resign => (format!("/api/board/game/{game}/resign"), vec![]),
            OnlineAction::Abort => (format!("/api/board/game/{game}/abort"), vec![]),
            OnlineAction::OfferDraw | OnlineAction::AcceptDraw | OnlineAction::DeclineDraw => (
                format!(
                    "/api/board/game/{game}/draw/{}",
                    if action == OnlineAction::DeclineDraw {
                        "false"
                    } else {
                        "yes"
                    }
                ),
                vec![],
            ),
            OnlineAction::ClaimVictory => (format!("/api/board/game/{game}/claim-victory"), vec![]),
            OnlineAction::ClaimDraw => (format!("/api/board/game/{game}/claim-draw"), vec![]),
            OnlineAction::Berserk => (format!("/api/board/game/{game}/berserk"), vec![]),
            OnlineAction::Takeback | OnlineAction::DeclineTakeback => (
                format!(
                    "/api/board/game/{game}/takeback/{}",
                    if action == OnlineAction::Takeback {
                        "yes"
                    } else {
                        "false"
                    }
                ),
                vec![],
            ),
        };
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(&path, &form, Some(&authorize(&token)), &cancel)
                    .await
            })
            .await?;
        Ok(())
    }

    /// Forget the session after an explicit logout, including recovery and game identity. With
    /// `accounts`, only a session that uses one of them is forgotten.
    pub fn logout(&self, accounts: Option<&[String]>) {
        if let Some(accounts) = accounts {
            let names: HashSet<String> = accounts.iter().map(|name| name.to_lowercase()).collect();
            let (related, lobby_account) = {
                let st = self.st();
                let related = names.contains(&st.attaching_account.to_lowercase())
                    || names.contains(&st.current_account.to_lowercase())
                    || names.contains(&st.protected_account.to_lowercase())
                    || (st.recovery_unverified
                        && st.current_account.is_empty()
                        && st.protected_account.is_empty());
                (related, st.lobby_account.clone())
            };
            if !related {
                if names.contains(&lobby_account.to_lowercase()) {
                    self.stay_connected("");
                }
                return;
            }
        }
        self.close();
        let session = {
            let mut st = self.st();
            st.protected_game.clear();
            st.protected_account.clear();
            st.current_account.clear();
            st.recovery_unverified = false;
            st.epoch
        };
        self.emit_state(session, "", "", Lane::Game, "idle", None);
    }

    /// Stop everything for good (the app is quitting).
    pub fn close(&self) {
        self.st().lobby_account.clear();
        self.cancel();
        self.stop_lobby();
        if let Some(timer) = self.st().ongoing_timer.take() {
            timer.abort();
        }
    }
}
