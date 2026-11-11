//! Cancelling, attaching to, resuming and listing the online session's games.

use kchess_contracts::CoreEvent;
use kchess_contracts::types::OnlinePhase;
use serde_json::Value;

use crate::host::Level;
use crate::lichess::client::Auth;
use kchess_contracts::error::{CoreError, ErrorKind, Result as CoreResult};

use crate::lichess::online::ongoing::{Checking, ongoing_game};
use crate::lichess::online::state::{Lane, OngoingGame, OnlineSession, Resumed};

impl OnlineSession {
    /* ── Sessions ─────────────────────────────────────────────────────── */

    /// `cancel()` without the deferred lobby: the caller is about to claim the streams itself.
    /// Ends every stream, moves the epoch, and reports what remains protected.
    pub(super) fn cancel_inner(&self) {
        let (phase_report, cancel_pending) = {
            let mut st = self.st();
            st.attach_generation += 1;
            st.attaching_account.clear();
            let leaving_correspondence = st
                .live
                .as_ref()
                .is_some_and(|game| game.correspondence && st.protected_game == game.id);
            if leaving_correspondence {
                st.protected_game.clear();
            }
            st.resume_attempt += 1;
            st.presence.clear();
            st.epoch += 1;
            st.live = None;
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
                    OnlinePhase::Disconnected,
                    st.protected_game.clone(),
                    Some("Reconnect to verify the current game before using engine assistance."),
                ))
            } else if leaving_correspondence {
                Some((OnlinePhase::Idle, String::new(), None))
            } else {
                None
            };
            let epoch = st.epoch;
            let account = st.current_account.clone();
            let phase_report =
                report.map(|(phase, game, message)| (epoch, account, game, phase, message));
            let cancel_pending = st
                .pending_challenge
                .take()
                .map(|id| (id, st.current_account.clone(), st.epoch));
            (phase_report, cancel_pending)
        };
        if let Some((session, account, game, phase, message)) = phase_report {
            self.emit_state(session, &account, &game, Lane::Game, phase, message);
        } else {
            // Nothing left to report, but the live game is gone: the engines may work again.
            self.refresh_assistance();
        }
        if let Some((id, account, epoch_at_cancel)) = cancel_pending {
            // Epoch-guarded: a challenge created by the new session after `cancel_inner`
            // bumped the epoch must not be cancelled by the old session's teardown.
            let this = self.clone();
            tokio::spawn(async move {
                if this.st().epoch != epoch_at_cancel {
                    return;
                }
                match this.login(&account).await {
                    Ok(Some(auth)) => this.cancel_challenge(&id, &auth).await,
                    Ok(None) => {}
                    Err(cause) => this.log(
                        Level::Debug,
                        format!("Pending challenge cancel failed: {id} {}", cause.message),
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

    /// Start the lobby once the current call has finished.
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
    pub async fn attach(&self, account: &str, id: &str) -> CoreResult<()> {
        if self.st().is_live(id) {
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
                Err(CoreError::aborted("Attachment cancelled."))
            }
            outcome => outcome,
        }
    }

    fn attachment_current(&self, generation: u64) -> bool {
        self.st().attach_generation == generation
    }

    async fn attach_inner(&self, account: &str, id: &str, generation: u64) -> CoreResult<()> {
        let auth = self.login(account).await?;
        if !self.attachment_current(generation) {
            return Err(CoreError::aborted("Attachment cancelled."));
        }
        let Some(auth) = auth else {
            return Err(crate::lichess::accounts::login_unavailable(account));
        };
        // Another caller may have attached this game while credentials were loading.
        if self.st().is_live(id) {
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
        self.start_event_stream(auth.clone(), handle);
        self.begin_game(id, &auth);
        Ok(())
    }

    /// Reattach to a real-time game in progress on any connected account (`resume`).
    pub async fn resume(&self) -> CoreResult<Option<Resumed>> {
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
            OnlinePhase::Checking,
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
                    // Only a refused login asks to reconnect: a 403 for another reason did,
                    // and reconnecting changed nothing.
                    let phase = if checking.auth_required || cause.kind == Some(ErrorKind::Login) {
                        OnlinePhase::AuthRequired
                    } else {
                        OnlinePhase::Disconnected
                    };
                    self.emit_state(
                        session,
                        &account,
                        &protected_game,
                        Lane::Game,
                        phase,
                        Some(&cause.message),
                    );
                }
                Err(cause)
            }
        }
    }

    async fn resume_body(
        &self,
        attempt: u64,
        checking: &mut Checking,
    ) -> CoreResult<Option<Resumed>> {
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
            return Err(CoreError::aborted("Recovery cancelled."));
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
        let mut first_error: Option<CoreError> = None;
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
            return Err(CoreError::aborted("Recovery cancelled."));
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
        self.emit_state(session, &account, "", Lane::Game, OnlinePhase::Idle, None);
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
    ) -> CoreResult<Option<Resumed>> {
        let checked: CoreResult<Option<Resumed>> = async {
            let auth = self.login(account).await?;
            if attempt != self.st().resume_attempt {
                return Err(CoreError::aborted("Recovery cancelled."));
            }
            let Some(auth) = auth else {
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
                    OnlinePhase::AuthRequired,
                    Some("Lichess login is unavailable. Reconnect in Settings."),
                );
                return Err(crate::lichess::accounts::login_unavailable(account));
            };
            let playing: Value = self
                .inner
                .lichess
                .client()
                .get("/api/account/playing", &auth)
                .read_json()
                .await?;
            let Some(now_playing) = playing.get("nowPlaying").and_then(Value::as_array) else {
                return Err(CoreError::new(
                    "Lichess returned an invalid ongoing-game response.",
                ));
            };
            // Correspondence games last for days: they open only when the player picks one. A
            // game the Board API cannot play stays where it was started.
            let id = now_playing
                .iter()
                .find(|game| {
                    game["speed"].as_str() != Some("correspondence")
                        && crate::lichess::online::ongoing::board_playable(game)
                })
                .and_then(|game| game["gameId"].as_str())
                .map(str::to_string);
            if attempt != self.st().resume_attempt {
                return Err(CoreError::aborted("Recovery cancelled."));
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
            self.start_event_stream(auth.clone(), handle);
            self.begin_game(&id, &auth);
            Ok(Some(Resumed {
                id,
                account: account.to_string(),
            }))
        }
        .await;
        match checked {
            Ok(found) => Ok(found),
            Err(cause) => {
                if attempt != self.st().resume_attempt || cause.aborted {
                    return Err(cause);
                }
                let protected = !protected_account.is_empty()
                    && account.eq_ignore_ascii_case(protected_account);
                if protected {
                    return Err(cause);
                }
                self.log(
                    Level::Warn,
                    format!("Game recovery check failed: {account} {}", cause.message),
                );
                checking.error = Some(cause);
                Ok(None)
            }
        }
    }

    /// Every game the connected accounts are playing, most urgent first (`ongoing`).
    pub async fn ongoing(&self) -> CoreResult<Vec<OngoingGame>> {
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
        // An account without an open event stream hears of no challenge: list them instead.
        let challenges_wanted = self
            .inner
            .lichess
            .store()
            .receives_challenges()
            .await
            .unwrap_or(false);
        let mut games = Vec::new();
        let mut first_error: Option<CoreError> = None;
        for account in accounts {
            let loaded: CoreResult<()> = async {
                let Some(auth) = self.login(&account).await? else {
                    return Ok(());
                };
                let playing: Value = self
                    .inner
                    .lichess
                    .client()
                    .get("/api/account/playing", &auth)
                    .query(&[("nb", "50")])
                    .read_json()
                    .await?;
                for game in playing
                    .get("nowPlaying")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    games.push(ongoing_game(game, &account));
                }
                if challenges_wanted && !self.has_event_stream(&account) {
                    self.sync_challenges(&account, &auth).await;
                }
                Ok(())
            }
            .await;
            if let Err(cause) = loaded {
                if cause.aborted {
                    continue;
                }
                self.log(
                    Level::Warn,
                    format!("Could not load ongoing games: {account} {}", cause.message),
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
    pub async fn open(&self, account: &str, id: &str) -> CoreResult<()> {
        {
            let st = self.st();
            if st.is_live(id) {
                return Ok(());
            }
            if st.realtime_live() {
                return Err(CoreError::new(
                    "Finish the live game on the board before opening another.",
                ));
            }
            // A real-time game left by a dropped connection stays protected until it is verified
            // over; attaching another would replace its protection with the other game's.
            if !st.protected_game.is_empty()
                && st.protected_game != id
                && st.protected_game != st.live_id()
            {
                return Err(CoreError::new(
                    "Reconnect to your current game before opening another.",
                ));
            }
        }
        self.attach(account, id).await
    }

    /// Whether `account`'s event stream is open (the idle lobby or a game's), so it keeps the
    /// inbox current by itself.
    fn has_event_stream(&self, account: &str) -> bool {
        let st = self.st();
        let lobby = st.lobby.as_ref().is_some_and(|(running, token)| {
            running.eq_ignore_ascii_case(account) && !token.is_cancelled()
        });
        lobby || (st.controller.is_some() && st.current_account.eq_ignore_ascii_case(account))
    }

    /// Bring `account`'s challenges in the inbox up to date from `GET /api/challenge`; a failure
    /// leaves them as they were.
    async fn sync_challenges(&self, account: &str, auth: &Auth) {
        let listed = self
            .inner
            .lichess
            .client()
            .get("/api/challenge", auth)
            .read_json::<Value>()
            .await;
        let listed = match listed {
            Ok(listed) => listed,
            Err(cause) => {
                if !cause.aborted {
                    self.log(
                        Level::Warn,
                        format!("Could not load challenges: {account} {}", cause.message),
                    );
                }
                return;
            }
        };
        let Some(fresh) = self.inner.challenges.sync(account, &listed) else {
            self.log(
                Level::Warn,
                format!("Lichess returned an invalid challenge list: {account}"),
            );
            return;
        };
        for info in fresh {
            self.emit(CoreEvent::ChallengeReceived(
                serde_json::to_value(info).unwrap_or(Value::Null),
            ));
        }
    }
}
