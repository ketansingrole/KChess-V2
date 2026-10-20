//! The online session's idle event stream.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::host::Level;

use crate::lichess::online::events::is_challenge_event;
use crate::lichess::online::requests::{Emitter, Listen, Quiet};
use crate::lichess::online::state::{Lane, OnlineSession, core};

impl OnlineSession {
    /* ── The idle event stream ─────────────────────────────────────────── */

    /// Keep `account`'s event stream open while nothing else is (empty: stop).
    pub fn stay_connected(&self, account: &str) {
        self.st().lobby_account = account.to_string();
        if account.is_empty() {
            self.stop_lobby();
        } else {
            self.start_lobby();
        }
    }

    pub(super) fn stop_lobby(&self) {
        if let Some((_, token)) = self.st().lobby.take() {
            token.cancel();
        }
    }

    pub(super) fn start_lobby(&self) {
        let (account, cancel) = {
            let mut st = self.st();
            let account = st.lobby_account.clone();
            if account.is_empty() || st.controller.is_some() {
                return;
            }
            let running = st
                .lobby
                .as_ref()
                .is_some_and(|(running, token)| *running == account && !token.is_cancelled());
            if running {
                return;
            }
            if let Some((_, token)) = st.lobby.take() {
                token.cancel();
            }
            let token = CancellationToken::new();
            st.lobby = Some((account.clone(), token.clone()));
            (account, token)
        };
        let this = self.clone();
        tokio::spawn(async move {
            this.lobby_task(account, cancel.clone()).await;
            // A stream that ended on its own (no login, refused, gave up) no longer counts as
            // running, so the next `stay_connected` opens a new one.
            cancel.cancel();
        });
    }

    async fn lobby_task(&self, account: String, cancel: CancellationToken) {
        let token = match self.token(&account).await {
            Ok(token) => token,
            Err(cause) => {
                self.log(
                    Level::Debug,
                    format!("Stored login is unavailable: {account} {}", cause.message()),
                );
                None
            }
        };
        if cancel.is_cancelled() {
            return;
        }
        let Some(token) = token else {
            self.lobby_state(
                &account,
                "auth-required",
                Some("Its Lichess login is unavailable. Reconnect it in Settings."),
            );
            return;
        };
        self.run_lobby(account, token, cancel).await;
    }

    fn lobby_state(&self, account: &str, phase: &str, message: Option<&str>) {
        let mut payload = json!({ "account": account, "phase": phase });
        if let Some(message) = message {
            payload["message"] = json!(message);
        }
        self.emit("online:lobby", payload);
    }

    async fn run_lobby(&self, account: String, token: String, cancel: CancellationToken) {
        let quiet_account = account.clone();
        let this_for_quiet = self.clone();
        let quiet: Quiet = Arc::new(move |phase, message| {
            this_for_quiet.lobby_state(&quiet_account, phase, message);
        });
        let options = Listen {
            lane: Lane::Events,
            reconnect: true,
            finished: None,
            quiet: Some(quiet),
            session: 0,
            account: account.clone(),
            game_id: String::new(),
        };
        let this = self.clone();
        let (for_account, cancel_for_emit) = (account.clone(), cancel.clone());
        let emit: Emitter = Arc::new(move |event: Value| {
            if cancel_for_emit.is_cancelled() {
                return;
            }
            let kind = event["type"].as_str().unwrap_or("").to_string();
            if kind == "gameStart" {
                this.challenge_became_game(event["game"]["gameId"].as_str().unwrap_or(""));
            }
            if is_challenge_event(&kind) {
                this.on_challenge(&for_account, &event);
            } else if kind == "gameStart"
                && event["game"]["speed"].as_str() != Some("correspondence")
            {
                let game_id = event["game"]["gameId"].as_str().unwrap_or("").to_string();
                let this = this.clone();
                let account = for_account.clone();
                tokio::spawn(async move {
                    let attached = crate::tasks::caught(this.attach(&account, &game_id))
                        .await
                        .unwrap_or_else(|_| Err(core("The game could not be opened.")));
                    if let Err(cause) = attached {
                        this.log(
                            Level::Warn,
                            format!(
                                "Could not open game: {account} {game_id} {}",
                                cause.message()
                            ),
                        );
                        this.report(&cause);
                    }
                });
            } else if kind == "gameStart" || kind == "gameFinish" {
                this.ongoing_changed();
            }
        });
        let request = Self::events_request(&token);
        self.listen(request, cancel, emit, options).await;
    }
}
