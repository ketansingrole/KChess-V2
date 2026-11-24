//! The quiet event streams of the connected accounts no other stream covers: a game started on
//! any of them, on the website or another device, locks assistance as soon as Lichess says so.
//! 2026-11-24: only the selected account's stream was open, and the next look at the others
//! waited for the correspondence check, which can be off, so a game on another account went
//! unseen while help was lent.
//!
//! One event stream per account: the board's game (`controller`) or the lobby covers its own
//! account, and the watchers cover the rest. They start once a check of every account has
//! answered (`resume`, `ongoing`) or an account connects; Lichess sends every ongoing game when a
//! stream opens, so what the check found stays known. Help is locked while Lichess refuses one
//! for good (a login revoked), or one ended without ever opening: then nothing follows that
//! account. A stream that drops after it opened keeps it known, as the lobby's does; Lichess
//! announces the ongoing games again when it reconnects, and `ongoing` starts one that gave up.

use std::sync::Arc;

use kchess_contracts::types::OnlinePhase;
use kchess_domain::online_game::server::ServerEvent;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use kchess_contracts::error::Result as CoreResult;

use crate::lichess::online::requests::{Emitter, Listen, Quiet};
use crate::lichess::online::state::{Lane, OnlineSession, State};

/// How long a watcher waits before it opens: the stream the app asks for just after its check
/// (the lobby, `stay_connected`) takes the account first, so its events are not streamed twice.
const OPEN_AFTER: std::time::Duration = std::time::Duration::from_millis(500);

/// One account's watcher stream.
pub(crate) struct Watcher {
    pub(crate) token: CancellationToken,
}

/// The account another stream covers, by lowercase name: the board game's account while one is
/// on the board, else the lobby's while it runs.
fn covered(st: &State) -> Option<String> {
    if st.controller.is_some() {
        return Some(st.current_account.to_lowercase());
    }
    st.lobby
        .as_ref()
        .filter(|(_, token)| !token.is_cancelled())
        .map(|(account, _)| account.to_lowercase())
}

impl State {
    /// An account no stream follows: Lichess refused its watcher, or the watcher ended without
    /// ever opening.
    pub(crate) fn watching_unknown(&self) -> bool {
        !self.watch_refused.is_empty()
            || self.watchers.iter().any(|(account, watcher)| {
                watcher.token.is_cancelled() && !self.watch_established.contains(account)
            })
    }
}

impl OnlineSession {
    /// Watch every connected account, as the profile has them now.
    pub async fn watch_accounts(&self) -> CoreResult<()> {
        let accounts: Vec<String> = self
            .inner
            .lichess
            .store()
            .accounts()
            .await?
            .into_iter()
            .filter(|row| row.connected)
            .map(|row| row.username)
            .collect();
        self.watch(accounts);
        Ok(())
    }

    /// Watch these connected accounts: a watcher that ended or was refused is tried again.
    pub(super) fn watch(&self, accounts: Vec<String>) {
        {
            let mut st = self.st();
            st.watch_refused.clear();
            st.watchers
                .retain(|_, watcher| !watcher.token.is_cancelled());
            st.watched = accounts;
        }
        self.sync_watchers();
    }

    /// Start the watchers wanted now and stop the rest: every watched account but the one
    /// another stream covers. Called before a stream for an account opens, so its watcher is
    /// closed first (Lichess keeps one event stream per account).
    pub(super) fn sync_watchers(&self) {
        let started = {
            let mut st = self.st();
            let covered = covered(&st);
            let wanted: std::collections::BTreeMap<String, String> = st
                .watched
                .iter()
                .map(|account| (account.to_lowercase(), account.clone()))
                .filter(|(key, _)| Some(key) != covered.as_ref())
                .collect();
            st.watchers.retain(|key, watcher| {
                let keep = wanted.contains_key(key);
                if !keep {
                    watcher.token.cancel();
                }
                keep
            });
            st.watch_refused.retain(|key| wanted.contains_key(key));
            let watched: std::collections::BTreeSet<String> = st
                .watched
                .iter()
                .map(|account| account.to_lowercase())
                .collect();
            st.watch_established.retain(|key| watched.contains(key));
            let mut started = Vec::new();
            for (key, account) in wanted {
                if st.watchers.contains_key(&key) || st.watch_refused.contains(&key) {
                    continue;
                }
                let token = CancellationToken::new();
                st.watchers.insert(
                    key,
                    Watcher {
                        token: token.clone(),
                    },
                );
                started.push((account, token));
            }
            started
        };
        for (account, token) in started {
            let this = self.clone();
            tokio::spawn(async move {
                this.watch_task(&account, &token).await;
                // Ended on its own (gave up, refused): `watch` tries it again.
                token.cancel();
                this.refresh_assistance();
            });
        }
        self.refresh_assistance();
    }

    async fn watch_task(&self, account: &str, cancel: &CancellationToken) {
        let key = account.to_lowercase();
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(OPEN_AFTER) => {}
        }
        let auth = self.login(account).await;
        if cancel.is_cancelled() {
            return;
        }
        let Ok(Some(auth)) = auth else {
            self.watch_refused(&key, cancel);
            return;
        };
        let quiet: Quiet = {
            let (this, key, cancel) = (self.clone(), key.clone(), cancel.clone());
            Arc::new(move |phase, _| match phase {
                OnlinePhase::Connected => this.watch_established(&key, &cancel),
                OnlinePhase::AuthRequired => this.watch_refused(&key, &cancel),
                _ => {}
            })
        };
        let options = Listen {
            lane: Lane::Events,
            reconnect: true,
            finished: None,
            quiet: Some(quiet),
            session: 0,
            account: account.to_string(),
            game_id: String::new(),
            refused: None,
        };
        let emit: Emitter = {
            let (this, cancel) = (self.clone(), cancel.clone());
            Arc::new(move |event: Value| {
                if !cancel.is_cancelled() {
                    this.on_watched_event(&event);
                }
            })
        };
        let request = Self::events_request(&auth);
        self.listen(request, cancel.clone(), emit, options).await;
    }

    /// What another account's stream says: a real-time game started there locks assistance until
    /// it finishes. It opens on the board only when the player picks it (`open`); its challenges
    /// are listed with the ongoing games, as before.
    fn on_watched_event(&self, event: &Value) {
        match ServerEvent::read(event) {
            ServerEvent::GameStart { game } => {
                let game = game.unwrap_or_default();
                let game_id = game.game_id.clone().unwrap_or_default();
                self.challenge_became_game(&game_id);
                self.started_elsewhere(&game_id, game.correspondence());
                self.ongoing_changed();
            }
            ServerEvent::GameFinish { game } => {
                let game_id = game.and_then(|game| game.game_id).unwrap_or_default();
                self.finished_elsewhere(&game_id);
                self.ongoing_changed();
            }
            _ => {}
        }
    }

    fn watch_established(&self, key: &str, cancel: &CancellationToken) {
        if cancel.is_cancelled() {
            return;
        }
        if self.st().watch_established.insert(key.to_string()) {
            self.refresh_assistance();
        }
    }

    fn watch_refused(&self, key: &str, cancel: &CancellationToken) {
        if cancel.is_cancelled() {
            return;
        }
        if self.st().watch_refused.insert(key.to_string()) {
            self.refresh_assistance();
        }
    }
}
