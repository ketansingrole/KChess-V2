use kchess_contracts::CoreEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use kchess_contracts::types::{OnlineConnection, OnlinePhase};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::super::accounts::Lichess;
use super::super::challenges::ChallengeInbox;
use super::super::client::{Auth, Lane as RequestLane};
use crate::error::{CoreError, ErrorKind, Result as CoreResult};
use crate::host::Level;
use crate::lichess::stream::Backoff;
use crate::time::now_ms;
use kchess_contracts::types::UsageKind;

use super::events::{is_challenge_event, validate_online_event};
use super::requests::{Emitter, Listen, Quiet, StreamRequest};
use super::state::{
    ANNOUNCED_MAX, Handle, Inner, LOST_CONNECTION, LOST_STREAM, Lane, MAX_RECONNECTS,
    ONGOING_DEBOUNCE, OnlineSession, State, lock,
};

mod lobby;
mod play;
mod sessions;
mod start;

/// One `online:state` event: which session, account, game and lane it is about, its phase, and
/// for a failure the message and kind the page shows.
struct StateUpdate<'a> {
    session: u64,
    account: &'a str,
    game_id: &'a str,
    lane: Lane,
    phase: OnlinePhase,
    message: Option<&'a str>,
    kind: Option<ErrorKind>,
}

impl OnlineSession {
    /// A session over `lichess` (its client, logins, store and host).
    pub fn new(lichess: Arc<Lichess>) -> OnlineSession {
        OnlineSession::with_backoff(lichess, Backoff::default())
    }

    /// `new`, with dropped streams reconnecting after `backoff` (the tests' shorter waits).
    pub fn with_backoff(lichess: Arc<Lichess>, backoff: Backoff) -> OnlineSession {
        let host = Arc::clone(lichess.host());
        let challenges = ChallengeInbox::new(Arc::clone(&host), Arc::new(now_ms));
        OnlineSession {
            inner: Arc::new(Inner {
                lichess,
                host,
                challenges,
                state: Mutex::new(State::new()),
                backoff,
                state_hook: Mutex::new(None),
            }),
        }
    }

    /// The pending challenges of the connected accounts (`ChallengeInbox`). The service forgets an
    /// account's challenges through it when the account is removed.
    pub fn challenges(&self) -> &ChallengeInbox {
        &self.inner.challenges
    }

    fn st(&self) -> MutexGuard<'_, State> {
        lock(&self.inner.state)
    }

    fn emit(&self, event: CoreEvent) {
        self.inner.host.emit(event);
    }

    fn log(&self, level: Level, message: String) {
        self.inner.host.log(level, "lichess", &message);
    }

    fn emit_error(&self, message: &str) {
        self.emit(CoreEvent::OnlineError(json!(message)));
    }

    fn emit_state(
        &self,
        session: u64,
        account: &str,
        game_id: &str,
        lane: Lane,
        phase: OnlinePhase,
        message: Option<&str>,
    ) {
        self.emit_state_update(StateUpdate {
            session,
            account,
            game_id,
            lane,
            phase,
            message,
            kind: None,
        });
    }

    /// `emit_state`, and with `kind` the kind of failure that caused it, so the page offers what
    /// can help.
    fn emit_state_update(&self, update: StateUpdate<'_>) {
        let StateUpdate {
            session,
            account,
            game_id,
            lane,
            phase,
            message,
            kind,
        } = update;
        let payload = json!(OnlineConnection {
            session,
            account: account.to_string(),
            game_id: game_id.to_string(),
            lane,
            phase,
            message: message.map(str::to_string),
            kind,
        });
        let live_game = super::state::reports_live_game(phase, game_id);
        self.run_state_hook(live_game, self.assistance_blocked());
        self.emit(CoreEvent::OnlineState(payload));
    }

    /// An error the user should see, unless it is a cancellation; a refusal of a request
    /// whose listen names its own message shows that instead.
    fn report(&self, cause: &CoreError, refused: Option<&str>) {
        if cause.aborted {
            return;
        }
        match refused {
            Some(message) if cause.kind == Some(ErrorKind::Refused) => self.emit_error(message),
            _ => self.emit_error(&cause.message),
        }
    }

    /// `account`'s login for play, when it has one.
    async fn login(&self, account: &str) -> CoreResult<Option<Auth>> {
        let lichess = &self.inner.lichess;
        Ok(lichess.stored_login(account).await?.map(|token| {
            lichess
                .client()
                .counted(account, UsageKind::Play)
                .with_token(token)
        }))
    }

    fn next_handle(&self, st: &mut State) -> Handle {
        st.next_handle += 1;
        Handle {
            id: st.next_handle,
            token: CancellationToken::new(),
        }
    }

    /* ── Streams ───────────────────────────────────────────────────────── */

    /// Opens one request of a stream and feeds every line to `on_line` as it arrives. `on_open`
    /// runs once the answer's status is good. A stream ends with the body or with cancellation.
    async fn read_stream(
        &self,
        request: &StreamRequest,
        cancel: &CancellationToken,
        on_open: &mut (dyn FnMut() + Send),
        on_line: &mut (dyn FnMut(&str) -> CoreResult<()> + Send),
    ) -> CoreResult<()> {
        let mut built = self
            .inner
            .lichess
            .client()
            .request(request.method.clone(), &request.path, &request.auth)
            .query(&request.query)
            .lane(RequestLane::Live)
            .cancel(cancel);
        if let Some(form) = &request.form {
            built = built.form(form);
        }
        built.lines(on_open, on_line).await
    }

    /// The state report of a listen: the quiet lobby's own report, or the game connection's when
    /// the session it started under is still the current one. Stale sessions report nothing.
    fn listen_state(
        &self,
        options: &Listen,
        cancel: &CancellationToken,
        phase: OnlinePhase,
        message: Option<&str>,
    ) {
        self.listen_report(options, cancel, phase, message, None);
    }

    /// The stream failed for good: its state carries the failure's kind.
    fn listen_failed(
        &self,
        options: &Listen,
        cancel: &CancellationToken,
        phase: OnlinePhase,
        cause: &CoreError,
    ) {
        let kind = cause.kind.unwrap_or(ErrorKind::Unavailable);
        self.listen_report(options, cancel, phase, Some(&cause.message), Some(kind));
    }

    fn listen_report(
        &self,
        options: &Listen,
        cancel: &CancellationToken,
        phase: OnlinePhase,
        message: Option<&str>,
        kind: Option<ErrorKind>,
    ) {
        if cancel.is_cancelled() {
            return;
        }
        if let Some(quiet) = &options.quiet {
            quiet(phase, message);
        } else if options.session == self.st().epoch {
            self.emit_state_update(StateUpdate {
                session: options.session,
                account: &options.account,
                game_id: &options.game_id,
                lane: options.lane,
                phase,
                message,
                kind,
            });
        }
    }

    /// The stream is gone and will not come back by itself: the game's page offers to reconnect.
    fn listen_lost(&self, options: &Listen, cancel: &CancellationToken) {
        self.listen_state(
            options,
            cancel,
            OnlinePhase::Disconnected,
            Some(LOST_STREAM),
        );
        if options.quiet.is_none() && !cancel.is_cancelled() {
            self.emit_error(LOST_CONNECTION);
        }
    }

    /// `listen(open, signal, emit, options)`: keeps one stream open, validates its events and
    /// reconnects with backoff as `options` says, until it ends, fails permanently or is cancelled.
    /// A panic ends it as a lost connection, so the game or lobby does not wait on a dead stream.
    async fn listen(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
        emit: Emitter,
        options: Listen,
    ) {
        let listened = crate::tasks::caught(self.listen_loop(&request, &cancel, &emit, &options));
        if listened.await.is_err() {
            self.listen_lost(&options, &cancel);
        }
    }

    async fn listen_loop(
        &self,
        request: &StreamRequest,
        cancel: &CancellationToken,
        emit: &Emitter,
        options: &Listen,
    ) {
        let mut failures: u32 = 0;
        self.listen_state(options, cancel, OnlinePhase::Connecting, None);
        while !cancel.is_cancelled() {
            let mut received = false;
            // A malformed line ends the read the way a thrown error does: the line error wins.
            let line_cancel = cancel.child_token();
            let mut line_error: Option<String> = None;
            let mut on_open = || {
                if options.quiet.is_some() {
                    self.listen_state(options, cancel, OnlinePhase::Connected, None);
                }
            };
            let mut on_line = |line: &str| -> CoreResult<()> {
                if cancel.is_cancelled() || line_error.is_some() {
                    return Ok(());
                }
                received = true;
                let parsed = match serde_json::from_str::<Value>(line) {
                    Ok(raw) => validate_online_event(&raw),
                    Err(cause) => Err(cause.to_string()),
                };
                match parsed {
                    Ok(Some(event)) => {
                        self.listen_state(options, cancel, OnlinePhase::Connected, None);
                        emit(event);
                    }
                    Ok(None) => {}
                    Err(message) => {
                        line_error = Some(message);
                        line_cancel.cancel();
                    }
                }
                Ok(())
            };
            let outcome = self
                .read_stream(request, &line_cancel, &mut on_open, &mut on_line)
                .await;
            // Lichess replays pending challenges and ongoing games on every connect, so a line the
            // checks refuse comes back each time: a connection that ended on one made no progress,
            // or the stream would reconnect about once a second forever.
            let malformed = line_error.is_some();
            let outcome = match line_error.take() {
                Some(message) => Err(CoreError::new(message)),
                None => outcome,
            };
            let account = options.account.clone();
            let game_id = options.game_id.clone();
            match outcome {
                Ok(()) => {}
                Err(cause) => {
                    if cancel.is_cancelled() {
                        return;
                    }
                    self.log(
                        Level::Debug,
                        format!("Stream failed: {account} {game_id} {}", cause.message),
                    );
                    // Lichess refused the stream for a reason reconnecting does not change.
                    let permanent = matches!(
                        cause.kind,
                        Some(ErrorKind::Login | ErrorKind::Refused | ErrorKind::NotFound)
                    );
                    if !options.reconnect || permanent {
                        let phase = if cause.kind == Some(ErrorKind::Login) {
                            OnlinePhase::AuthRequired
                        } else {
                            OnlinePhase::Disconnected
                        };
                        self.listen_failed(options, cancel, phase, &cause);
                        if options.quiet.is_none() {
                            self.report(&cause, options.refused);
                        }
                        return;
                    }
                }
            }
            if cancel.is_cancelled()
                || options
                    .finished
                    .as_ref()
                    .is_some_and(|finished| finished.load(Ordering::SeqCst))
            {
                return;
            }
            if !options.reconnect {
                self.listen_state(
                    options,
                    cancel,
                    OnlinePhase::Disconnected,
                    Some("The seek ended. Try finding a game again."),
                );
                return;
            }
            failures = if received && !malformed {
                1
            } else {
                failures + 1
            };
            if failures > MAX_RECONNECTS {
                self.log(
                    Level::Warn,
                    format!(
                        "Lichess stream gave up reconnecting: {} {} failures={failures}",
                        options.account, options.game_id
                    ),
                );
                self.listen_lost(options, cancel);
                return;
            }
            self.listen_state(
                options,
                cancel,
                OnlinePhase::Reconnecting,
                Some("Connection interrupted. Reconnecting…"),
            );
            // Backing off ends early when the session is cancelled; the loop then exits.
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(self.inner.backoff.after(failures)) => {}
            }
        }
    }

    /// The listen options of a stream, with the session facts read now (as `listen` reads them
    /// when it is called).
    fn listen_options(
        &self,
        lane: Lane,
        reconnect: bool,
        finished: Option<Arc<AtomicBool>>,
        quiet: Option<Quiet>,
    ) -> Listen {
        let st = self.st();
        Listen {
            lane,
            reconnect,
            finished,
            quiet,
            session: st.epoch,
            account: st.current_account.clone(),
            game_id: st.lane_game(lane).to_string(),
            refused: None,
        }
    }

    fn events_request(auth: &Auth) -> StreamRequest {
        StreamRequest::get("/api/stream/event", auth)
    }

    /// The event stream of the account being played: challenges go to the inbox,
    /// a real-time game starting opens it on the board, and the rest is passed on.
    fn start_event_stream(&self, auth: Auth, handle: Handle) {
        let account = self.st().current_account.clone();
        let options = self.listen_options(Lane::Events, true, None, None);
        let this = self.clone();
        let request = Self::events_request(&auth);
        let cancel = handle.token.clone();
        let emit: Emitter = Arc::new(move |event| {
            this.on_event_stream(&account, &auth, &cancel, event);
        });
        let this = self.clone();
        let cancel = handle.token;
        tokio::spawn(async move {
            this.listen(request, cancel, emit, options).await;
        });
    }

    fn on_event_stream(
        &self,
        account: &str,
        auth: &Auth,
        cancel: &CancellationToken,
        event: Value,
    ) {
        if cancel.is_cancelled() {
            return;
        }
        let kind = event["type"].as_str().unwrap_or("").to_string();
        if is_challenge_event(&kind) {
            self.on_challenge(account, &event);
            return;
        }
        if kind == "gameStart" {
            let game_id = event["game"]["gameId"].as_str().unwrap_or("").to_string();
            self.challenge_became_game(&game_id);
            // A game the Board API cannot play stays where it was started; it is listed only.
            if !super::ongoing::board_playable(&event["game"]) {
                self.ongoing_changed();
                return;
            }
            let correspondence = super::ongoing::correspondence(&event["game"]);
            let started = self.st().started(&game_id, correspondence);
            match started {
                super::state::Started::OnBoard => {}
                super::state::Started::ListedOnly => {
                    self.ongoing_changed();
                    return;
                }
                super::state::Started::Opens => {
                    self.st().pending_challenge = None;
                    self.begin_game(&game_id, auth);
                }
            }
            self.announce(&game_id, event);
            return;
        }
        if kind == "gameFinish" {
            self.ongoing_changed();
            let game_id = event["game"]["gameId"].as_str().unwrap_or("").to_string();
            {
                let mut st = self.st();
                if !st.concerns(&game_id) {
                    return;
                }
                // Lichess may report the finish here before the game's own stream does, and the
                // window offers a new game on it: end the game now, as its final state would.
                st.end_live(&game_id);
                if st.protected_game == game_id {
                    st.protected_game.clear();
                }
            }
            self.refresh_assistance();
        }
        self.emit(CoreEvent::OnlineEvent(event));
    }

    fn on_challenge(&self, account: &str, event: &Value) {
        if let Some(info) = self.inner.challenges.ingest(account, event) {
            self.emit(CoreEvent::ChallengeReceived(
                serde_json::to_value(info).unwrap_or(Value::Null),
            ));
        }
    }

    /// An accepted challenge becomes the game of the same id; Lichess sends no other event for
    /// it, so it leaves the inbox here.
    pub(super) fn challenge_became_game(&self, game_id: &str) {
        if !game_id.is_empty() {
            self.inner.challenges.remove(game_id);
        }
    }

    /// A game's start reaches the window once, however often the stream reconnects.
    fn announce(&self, game_id: &str, event: Value) {
        {
            let mut st = self.st();
            if st.announced.iter().any(|known| known == game_id) {
                return;
            }
            st.announced.push(game_id.to_string());
            if st.announced.len() > ANNOUNCED_MAX {
                st.announced.remove(0);
            }
        }
        self.emit(CoreEvent::OnlineEvent(event));
    }

    /// Report the change once the burst of stream events has passed.
    fn ongoing_changed(&self) {
        let this = self.clone();
        let mut st = self.st();
        if let Some(timer) = st.ongoing_timer.take() {
            timer.abort();
        }
        st.ongoing_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(ONGOING_DEBOUNCE).await;
            this.emit(CoreEvent::OngoingChanged(Value::Null));
        }));
    }
}
