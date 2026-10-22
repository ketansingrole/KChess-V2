//! Cancelling, attaching to, resuming and listing the online session's games (moved from
//! `session.rs` without changes).

use serde_json::Value;

use crate::host::Level;
use crate::lichess::client::{Failure, authorize};

use crate::lichess::online::ongoing::{Checking, ongoing_game};
use crate::lichess::online::state::{
    Lane, OngoingGame, OnlineSession, Res, Resumed, aborted, core,
};

impl OnlineSession {
    /* ── Sessions ─────────────────────────────────────────────────────── */

    /// `cancel()` without the deferred lobby: the caller is about to claim the streams itself.
    /// Ends every stream, moves the epoch, and reports what remains protected.
    pub(super) fn cancel_inner(&self) {
        let (phase_report, cancel_pending) = {
            let mut st = self.st();
            st.attach_generation += 1;
            st.attaching_account.clear();
            let leaving_correspondence = st.live_correspondence
                && !st.live_game.is_empty()
                && st.protected_game == st.live_game;
            if leaving_correspondence {
                st.protected_game.clear();
            }
            st.resume_attempt += 1;
            st.presence.clear();
            st.epoch += 1;
            st.live_game.clear();
            st.live_correspondence = false;
            if let Some((_, token)) = st.lobby.take() {
                token.cancel();
            }
            if let Some(controller) = st.controller.take() {
                controller.token.cancel();
            }
            if let Some(controller) = st.game_controller.take() {
                controller.token.cancel();
            }
            let report = if !st.protected_game.is_empty() || st.recovery_unverified {
                Some((
                    "disconnected",
                    st.protected_game.clone(),
                    Some("Reconnect to verify the current game before using engine assistance."),
                ))
            } else if leaving_correspondence {
                Some(("idle", String::new(), None))
            } else {
                None
            };
            let epoch = st.epoch;
            let account = st.current_account.clone();
            let phase_report =
                report.map(|(phase, game, message)| (epoch, account, game, phase, message));
            let cancel_pending = if st.pending_id.is_empty() {
                None
            } else {
                let id = std::mem::take(&mut st.pending_id);
                Some((id, st.current_account.clone(), st.epoch))
            };
            (phase_report, cancel_pending)
        };
        if let Some((session, account, game, phase, message)) = phase_report {
            self.emit_state(session, &account, &game, Lane::Game, phase, message);
        }
        if let Some((id, account, epoch_at_cancel)) = cancel_pending {
            // Epoch-guarded: a challenge created by the new session after `cancel_inner`
            // bumped the epoch must not be cancelled by the old session's teardown.
            let this = self.clone();
            tokio::spawn(async move {
                if this.st().epoch != epoch_at_cancel {
                    return;
                }
                match this.token(&account).await {
                    Ok(Some(token)) => this.cancel_challenge(&id, &token).await,
                    Ok(None) => {}
                    Err(cause) => this.log(
                        Level::Debug,
                        format!("Pending challenge cancel failed: {id} {}", cause.message()),
                    ),
                }
            });
        }
    }

    /// `cancel()`: end the session's streams and its pending challenge. The idle lobby stream
    /// resumes afterwards, so challenges keep arriving.
    pub fn cancel(&self) {
        self.cancel_inner();
        self.schedule_lobby();
    }

    /// `queueMicrotask(startLobby)`: start the lobby once the current call has finished.
    pub(super) fn schedule_lobby(&self) {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let this = self.clone();
                handle.spawn(async move { this.start_lobby() });
            }
            Err(_) => self.start_lobby(),
        }
    }

    /// Open a known game of `account` on the board: its event stream, then its game stream.
    pub async fn attach(&self, account: &str, id: &str) -> Res<()> {
        if self.st().live_game == id {
            return Ok(());
        }
        let generation = {
            let mut st = self.st();
            st.attach_generation += 1;
            st.attach_generation
        };
        self.st().attaching_account = account.to_string();
        let outcome = self.attach_inner(account, id, generation).await;
        let current = self.attachment_current(generation);
        if current {
            self.st().attaching_account.clear();
        }
        match outcome {
            Err(cause) if !current => {
                let _ = cause;
                Err(aborted("Attachment cancelled."))
            }
            outcome => outcome,
        }
    }

    fn attachment_current(&self, generation: u64) -> bool {
        self.st().attach_generation == generation
    }

    async fn attach_inner(&self, account: &str, id: &str, generation: u64) -> Res<()> {
        let token = self.token(account).await?;
        if !self.attachment_current(generation) {
            return Err(aborted("Attachment cancelled."));
        }
        let Some(token) = token else {
            return Err(core(format!(
                "@{account}'s Lichess login is unavailable. Reconnect it in Settings."
            )));
        };
        // Another caller may have attached this game while credentials were loading.
        if self.st().live_game == id {
            return Ok(());
        }
        self.cancel_inner();
        let handle = {
            let mut st = self.st();
            st.current_account = account.to_string();
            let handle = self.next_handle(&mut st);
            st.controller = Some(handle.clone());
            handle
        };
        self.start_event_stream(token.clone(), handle);
        self.begin_game(id, &token);
        Ok(())
    }

    /// Reattach to a real-time game in progress on any connected account (`resume`).
    pub async fn resume(&self) -> Res<Option<Resumed>> {
        let attempt = {
            let mut st = self.st();
            st.attach_generation += 1;
            st.attaching_account.clear();
            st.resume_attempt += 1;
            st.recovery_unverified = true;
            st.resume_attempt
        };
        let mut checking = Checking::default();
        let (session, account, game) = {
            let st = self.st();
            (
                st.epoch,
                st.current_account.clone(),
                st.protected_game.clone(),
            )
        };
        checking.account = account.clone();
        self.emit_state(
            session,
            &account,
            &game,
            Lane::Game,
            "checking",
            Some("Checking Lichess for an ongoing game…"),
        );
        let outcome = self.resume_body(attempt, &mut checking).await;
        match outcome {
            Ok(found) => Ok(found),
            Err(cause) => {
                if attempt == self.st().resume_attempt {
                    let (session, protected_account, protected_game) = {
                        let st = self.st();
                        (
                            st.epoch,
                            st.protected_account.clone(),
                            st.protected_game.clone(),
                        )
                    };
                    let account = if protected_account.is_empty() {
                        checking.account.clone()
                    } else {
                        protected_account
                    };
                    let phase = if checking.auth_required
                        || matches!(cause.status(), Some(401) | Some(403))
                    {
                        "auth-required"
                    } else {
                        "disconnected"
                    };
                    self.emit_state(
                        session,
                        &account,
                        &protected_game,
                        Lane::Game,
                        phase,
                        Some(&cause.message()),
                    );
                }
                Err(cause)
            }
        }
    }

    async fn resume_body(&self, attempt: u64, checking: &mut Checking) -> Res<Option<Resumed>> {
        let mut accounts: Vec<String> = self
            .inner
            .lichess
            .store()
            .accounts()
            .await?
            .into_iter()
            .filter(|row| row.connected)
            .map(|row| row.username)
            .collect();
        if attempt != self.st().resume_attempt {
            return Err(aborted("Recovery cancelled."));
        }
        // A protected game's account is checked first.
        let protected_account = {
            let st = self.st();
            if st.protected_game.is_empty() {
                String::new()
            } else {
                st.protected_account.clone()
            }
        };
        if !protected_account.is_empty() {
            accounts.retain(|name| !name.eq_ignore_ascii_case(&protected_account));
            accounts.insert(0, protected_account.clone());
        }
        let mut first_error: Option<Failure> = None;
        for account in accounts {
            checking.account = account.clone();
            match self
                .resume_account(attempt, &account, &protected_account, checking)
                .await
            {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) => {}
                Err(cause) => return Err(cause),
            }
            // The per-account outcome is folded below through `first_error`.
            if let Some(error) = checking.error.take() {
                first_error.get_or_insert(error);
            }
        }
        if attempt != self.st().resume_attempt {
            return Err(aborted("Recovery cancelled."));
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.st().recovery_unverified = false;
        self.cancel_inner();
        let (session, account) = {
            let mut st = self.st();
            st.protected_game.clear();
            (st.epoch, st.current_account.clone())
        };
        self.emit_state(session, &account, "", Lane::Game, "idle", None);
        self.start_lobby();
        Ok(None)
    }

    /// One account of a recovery check. A failure of an account other than the protected one is
    /// recorded in `checking.error` and the next account is tried; the protected account's failure
    /// (and a cancellation) ends the recovery.
    async fn resume_account(
        &self,
        attempt: u64,
        account: &str,
        protected_account: &str,
        checking: &mut Checking,
    ) -> Res<Option<Resumed>> {
        let checked: Res<Option<Resumed>> = async {
            let token = self.token(account).await?;
            if attempt != self.st().resume_attempt {
                return Err(aborted("Recovery cancelled."));
            }
            let Some(token) = token else {
                checking.auth_required = true;
                let (session, game) = {
                    let st = self.st();
                    (st.epoch, st.protected_game.clone())
                };
                self.emit_state(
                    session,
                    account,
                    &game,
                    Lane::Game,
                    "auth-required",
                    Some("Lichess login is unavailable. Reconnect in Settings."),
                );
                return Err(core(format!(
                    "@{account}'s Lichess login is unavailable. Reconnect in Settings."
                )));
            };
            let playing: Value = self
                .inner
                .lichess
                .client()
                .policy()
                .with_usage(account, "play", async {
                    self.inner
                        .lichess
                        .client()
                        .get_json::<Value>(
                            "/api/account/playing",
                            &[],
                            Some(&authorize(&token)),
                            self.inner.lichess.lifetime(),
                        )
                        .await
                })
                .await?;
            let Some(now_playing) = playing.get("nowPlaying").and_then(Value::as_array) else {
                return Err(core("Lichess returned an invalid ongoing-game response."));
            };
            // Correspondence games last for days: they open only when the player picks one.
            let id = now_playing
                .iter()
                .find(|game| game["speed"].as_str() != Some("correspondence"))
                .and_then(|game| game["gameId"].as_str())
                .map(str::to_string);
            if attempt != self.st().resume_attempt {
                return Err(aborted("Recovery cancelled."));
            }
            let Some(id) = id else {
                return Ok(None);
            };
            self.st().recovery_unverified = false;
            self.cancel_inner();
            {
                let mut st = self.st();
                st.current_account = account.to_string();
            }
            let handle = {
                let mut st = self.st();
                let handle = self.next_handle(&mut st);
                st.controller = Some(handle.clone());
                handle
            };
            self.start_event_stream(token.clone(), handle);
            self.begin_game(&id, &token);
            Ok(Some(Resumed {
                id,
                account: account.to_string(),
            }))
        }
        .await;
        match checked {
            Ok(found) => Ok(found),
            Err(cause) => {
                if attempt != self.st().resume_attempt || cause.is_aborted() {
                    return Err(cause);
                }
                let protected = !protected_account.is_empty()
                    && account.eq_ignore_ascii_case(protected_account);
                if protected {
                    return Err(cause);
                }
                self.log(
                    Level::Warn,
                    format!("Game recovery check failed: {account} {}", cause.message()),
                );
                checking.error = Some(cause);
                Ok(None)
            }
        }
    }

    /// Every game the connected accounts are playing, most urgent first (`ongoing`).
    pub async fn ongoing(&self) -> Res<Vec<OngoingGame>> {
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
        let mut games = Vec::new();
        let mut first_error: Option<Failure> = None;
        for account in accounts {
            let loaded: Res<()> = async {
                let Some(token) = self.token(&account).await? else {
                    return Ok(());
                };
                let playing: Value = self
                    .policy()
                    .with_usage(&account, "play", async {
                        self.inner
                            .lichess
                            .client()
                            .get_json::<Value>(
                                "/api/account/playing",
                                &[("nb", "50".to_string())],
                                Some(&authorize(&token)),
                                self.inner.lichess.lifetime(),
                            )
                            .await
                    })
                    .await?;
                for game in playing
                    .get("nowPlaying")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    games.push(ongoing_game(game, &account));
                }
                Ok(())
            }
            .await;
            if let Err(cause) = loaded {
                if cause.is_aborted() {
                    continue;
                }
                self.log(
                    Level::Warn,
                    format!(
                        "Could not load ongoing games: {account} {}",
                        cause.message()
                    ),
                );
                first_error.get_or_insert(cause);
            }
        }
        if let Some(error) = first_error.filter(|_| games.is_empty()) {
            return Err(error);
        }
        games.sort_by_key(|a| std::cmp::Reverse(a.is_my_turn));
        Ok(games)
    }

    /// Open one of `ongoing()`'s games; a live real-time game on the board is never replaced.
    pub async fn open(&self, account: &str, id: &str) -> Res<()> {
        {
            let st = self.st();
            if st.live_game == id {
                return Ok(());
            }
            if !st.live_game.is_empty() && !st.live_correspondence {
                return Err(core(
                    "Finish the live game on the board before opening another.",
                ));
            }
        }
        self.attach(account, id).await
    }
}
