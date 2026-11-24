//! Cancelling, attaching to, resuming and listing the online session's games.

use kchess_contracts::CoreEvent;
use kchess_contracts::types::OnlinePhase;
use kchess_domain::online_game::server::GameRef;
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
        self.end_session(true);
    }

    /// `cancel_inner`, for a new seek or challenge: a check for a game in progress that has not
    /// answered (or failed) is not reported as a game to reconnect to, which would replace the
    /// seek the player just started. The seek's event stream announces any game in progress, and
    /// assistance stays blocked until a check answers (`assistance_blocked`).
    pub(super) fn cancel_for_start(&self) {
        self.end_session(false);
    }

    fn end_session(&self, report_unverified: bool) {
        self.end_session_if(report_unverified, |_| true);
    }

    /// `end_session`, when `commit` (run first, under the same lock) says so: a recovery
    /// applies what it found and checks that no game took the board meanwhile in one step.
    fn end_session_if(
        &self,
        report_unverified: bool,
        commit: impl FnOnce(&mut crate::lichess::online::state::State) -> bool,
    ) -> bool {
        let (phase_report, cancel_pending) = {
            let mut st = self.st();
            if !commit(&mut st) {
                return false;
            }
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
            let report = if !st.protected_game.is_empty()
                || (report_unverified && st.recovery_unverified)
            {
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
                .map(|id| (id, st.current_account.clone()));
            (phase_report, cancel_pending)
        };
        // The accounts the ended streams covered are watched until a new stream covers them.
        self.sync_watchers();
        if let Some((session, account, game, phase, message)) = phase_report {
            self.emit_state(session, &account, &game, Lane::Game, phase, message);
        } else {
            // Nothing left to report, but the live game is gone: the engines may work again.
            self.refresh_assistance();
        }
        if let Some((id, account)) = cancel_pending {
            // The ended session's own challenge, by its id: a session started meanwhile (its
            // challenge has another id) still has it withdrawn, or it could be accepted.
            let this = self.clone();
            tokio::spawn(async move {
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
        true
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
            Err(_) if !current => Err(CoreError::aborted("Attachment cancelled.")),
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
        // A real-time game took the board meanwhile (another account's started as a challenge
        // was being accepted): it stays, with its clock running.
        if self.st().realtime_live() {
            return Err(CoreError::new(
                "Finish the game on the board before opening another.",
            ));
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
        let mut checking = Checking {
            began: self.st().elsewhere_changes,
            ..Checking::default()
        };
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
        let watched = accounts.clone();
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
        let total = accounts.len();
        for (index, account) in accounts.into_iter().enumerate() {
            checking.account = account.clone();
            checking.others_unchecked = index + 1 < total || first_error.is_some();
            match self
                .resume_account(attempt, &account, &protected_account, checking)
                .await
            {
                Ok(Some(found)) => {
                    // Each account's games are followed from now on, so one started after this
                    // check is seen too; the board's account has its own stream already.
                    self.watch(watched);
                    return Ok(Some(found));
                }
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
        let elsewhere = std::mem::take(&mut checking.elsewhere);
        let committed = self.end_session_if(true, |st| {
            // A game that took the board meanwhile (a seek's, from its event stream) is newer
            // than this check: it stays. Checked apart from the end, the end cancelled its
            // streams and lent help while it was played.
            if attempt != st.resume_attempt {
                return false;
            }
            // Every account was checked: this is what is played elsewhere now, and a finish
            // missed while the stream was down no longer keeps assistance locked. Kept before
            // the flags clear and the session ends (which reports the lock): kept after, help
            // was lent meanwhile during a game recovery had just found.
            st.found_elsewhere(elsewhere, checking.began, true);
            st.recovery_unverified = false;
            st.accounts_unchecked = false;
            true
        });
        if !committed {
            return Err(CoreError::aborted("Recovery cancelled."));
        }
        let (session, account) = {
            let mut st = self.st();
            st.protected_game.clear();
            (st.epoch, st.current_account.clone())
        };
        self.emit_state(session, &account, "", Lane::Game, OnlinePhase::Idle, None);
        self.start_lobby();
        self.watch(watched);
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
                    let game = GameRef::read(game);
                    !game.correspondence() && game.board_compatible()
                })
                .and_then(|game| game["gameId"].as_str())
                .map(str::to_string);
            // The other real-time games lock assistance until they end, whatever this check
            // finds for the board: those the Board API cannot play, and any beside the one the
            // board takes.
            checking.elsewhere.extend(
                now_playing
                    .iter()
                    .filter(|game| !GameRef::read(game).correspondence())
                    .filter_map(|game| game["gameId"].as_str())
                    .filter(|game| Some(*game) != id.as_deref())
                    .map(str::to_string),
            );
            if attempt != self.st().resume_attempt {
                return Err(CoreError::aborted("Recovery cancelled."));
            }
            let Some(id) = id else {
                return Ok(None);
            };
            {
                // The accounts not checked yet keep the games they were known to play.
                let found = checking.elsewhere.clone();
                self.st().found_elsewhere(found, checking.began, false);
            }
            // Still unverified as the old session ends, so help stays locked until the game is
            // on the board: verified first, the end of the old session lent it in between.
            // Ended as a start does, it is not reported as a game to reconnect to.
            self.cancel_for_start();
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
            {
                let mut st = self.st();
                st.recovery_unverified = false;
                st.accounts_unchecked = checking.others_unchecked;
            }
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
        let watched = accounts.clone();
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
        // What every account plays elsewhere, which also settles a game whose finish went
        // unheard (its account's stream was replaced by another's).
        let began = self.st().elsewhere_changes;
        let mut elsewhere = std::collections::BTreeSet::new();
        let mut complete = true;
        for account in accounts {
            let loaded: CoreResult<()> = async {
                let Some(auth) = self.login(&account).await? else {
                    complete = false;
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
                // Read as no games, it once let a check settle the games played elsewhere and
                // lend help in the middle of one; recovery refuses it too.
                let Some(now_playing) = playing.get("nowPlaying").and_then(Value::as_array) else {
                    return Err(CoreError::new(
                        "Lichess returned an invalid ongoing-game response.",
                    ));
                };
                for game in now_playing {
                    // Every real-time game locks assistance until it ends: one the board holds
                    // is left to the board below.
                    if !GameRef::read(game).correspondence()
                        && let Some(id) = game["gameId"].as_str()
                    {
                        elsewhere.insert(id.to_string());
                    }
                    games.push(ongoing_game(game, &account));
                }
                if challenges_wanted && !self.has_event_stream(&account) {
                    self.list_challenges_later(&account, auth);
                }
                Ok(())
            }
            .await;
            if let Err(cause) = loaded {
                complete = false;
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
        {
            let mut st = self.st();
            // The board's own game ends with the board; another account's beside it does not.
            elsewhere.retain(|id| !st.concerns(id));
            st.found_elsewhere(elsewhere, began, complete);
            if complete {
                st.accounts_unchecked = false;
            }
        }
        // Followed from now on; a watcher that gave up (offline for long) or was refused is
        // tried again.
        self.watch(watched);
        self.refresh_assistance();
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
    /// List `account`'s challenges in the background: the games answer without waiting for it
    /// (Lichess can take a request's whole deadline, 30 s, to list them), and what it finds reaches
    /// the app as events (`ChallengeReceived`, the inbox's `ChallengesUpdate`). One listing an
    /// account at a time; asked again while one runs, that one answers.
    fn list_challenges_later(&self, account: &str, auth: Auth) {
        if !crate::tasks::lock(&self.inner.listing_challenges).insert(account.to_string()) {
            return;
        }
        let this = self.clone();
        let account = account.to_string();
        tokio::spawn(async move {
            let _listed = crate::tasks::on_drop(|| {
                crate::tasks::lock(&this.inner.listing_challenges).remove(&account);
            });
            this.sync_challenges(&account, &auth).await;
        });
    }

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
            self.emit(CoreEvent::ChallengeReceived(Box::new(info)));
        }
    }
}
