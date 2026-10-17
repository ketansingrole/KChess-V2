use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::super::accounts::{Lichess, now_ms};
use super::super::challenges::ChallengeInbox;
use super::super::client::{
    Failure, LichessError, TokenSource, authorize, lichess_error, read_with_deadline,
};
use super::super::stream::{LineOptions, read_lines};
use crate::error::CoreError;
use crate::host::Level;

use super::events::{is_challenge_event, validate_online_event};
use super::requests::{Emitter, Listen, Quiet, StreamRequest, body_value, refs};
use super::state::{
    ANNOUNCED_MAX, Handle, Inner, LOST_CONNECTION, Lane, MAX_RECONNECTS, ONGOING_DEBOUNCE,
    OnlineSession, Res, SEEK_REJECTED, State, core, lock,
};

mod lobby;
mod play;
mod sessions;
mod start;

impl OnlineSession {
    /// A session over `lichess` (its client, store and host), reading tokens from `tokens`.
    /// `http` must be the client `lichess` was built with, so the streams share its settings.
    pub fn new(
        lichess: Arc<Lichess>,
        tokens: Arc<dyn TokenSource>,
        http: reqwest::Client,
    ) -> OnlineSession {
        let host = Arc::clone(lichess.host());
        let challenges = ChallengeInbox::new(Arc::clone(&host), Arc::new(now_ms));
        OnlineSession {
            inner: Arc::new(Inner {
                lichess,
                tokens,
                http,
                host,
                challenges,
                state: Mutex::new(State::new()),
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

    fn emit(&self, event: &str, payload: Value) {
        self.inner.host.emit(event, payload);
    }

    fn log(&self, level: Level, message: String) {
        self.inner.host.log(level, "lichess", &message);
    }

    fn emit_error(&self, message: &str) {
        self.emit("online:error", json!(message));
    }

    fn emit_state(
        &self,
        session: u64,
        account: &str,
        game_id: &str,
        lane: Lane,
        phase: &str,
        message: Option<&str>,
    ) {
        let mut payload = json!({
            "session": session,
            "account": account,
            "gameId": game_id,
            "lane": lane.name(),
            "phase": phase,
        });
        if let Some(message) = message {
            payload["message"] = json!(message);
        }
        let live_game = phase == "checking" || (!game_id.is_empty() && phase != "idle");
        self.run_state_hook(live_game, self.assistance_blocked());
        self.emit("online:state", payload);
    }

    /// `report(cause)`: an error the user should see, unless it is a cancellation.
    fn report(&self, cause: &Failure) {
        if cause.is_aborted() {
            return;
        }
        match cause {
            Failure::Lichess(LichessError {
                status: 400,
                endpoint,
                ..
            }) if endpoint == "POST /api/board/seek" => self.emit_error(SEEK_REJECTED),
            _ => self.emit_error(&cause.message()),
        }
    }

    /// The token of `account`, if it has a login (`getToken`).
    async fn token(&self, account: &str) -> Res<Option<String>> {
        self.inner
            .tokens
            .token(account)
            .await
            .map_err(Failure::from)
    }

    fn policy(&self) -> Arc<crate::lichess::policy::Policy> {
        Arc::clone(self.inner.lichess.client().policy())
    }

    /// Runs `work` with the usage of `account` (`withUsage`), on a spawned task.
    fn spawn_for(
        &self,
        account: &str,
        kind: &'static str,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let policy = self.policy();
        let account = account.to_string();
        tokio::spawn(async move {
            policy.with_usage(&account, kind, work).await;
        });
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
    /// runs once the answer's status is good. A failed answer is a Lichess error naming the
    /// request; a stream ends with the body or with cancellation.
    async fn read_stream(
        &self,
        request: &StreamRequest,
        cancel: &CancellationToken,
        on_open: &mut (dyn FnMut() + Send),
        on_line: &mut (dyn FnMut(&str) -> Result<(), CoreError> + Send),
    ) -> Res<()> {
        let client = self.inner.lichess.client();
        let mut url = reqwest::Url::parse(&format!("{}{}", client.base(), request.path))
            .map_err(|cause| core(format!("Invalid Lichess URL: {cause}")))?;
        if !request.query.is_empty() {
            url.query_pairs_mut().extend_pairs(
                request
                    .query
                    .iter()
                    .map(|(key, value)| (*key, value.as_str())),
            );
        }
        let mut builder = self
            .inner
            .http
            .request(request.method.clone(), url)
            .header(reqwest::header::ACCEPT, "application/x-ndjson");
        if let Some(auth) = &request.auth {
            builder = builder.header(reqwest::header::AUTHORIZATION, auth.as_str());
        }
        if let Some(form) = &request.form {
            builder = builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(crate::lichess::client::urlencoded(&refs(form)));
        }
        let built = builder
            .build()
            .map_err(|cause| core(format!("Invalid Lichess request: {cause}")))?;
        let crate::lichess::policy::Admitted { response, hold } =
            self.policy().send(built, cancel).await?;
        let status = response.status();
        if !status.is_success() {
            let text =
                read_with_deadline(response, cancel, false, self.policy().timeout(), None).await?;
            drop(hold);
            return Err(Failure::Lichess(lichess_error(
                status.as_u16(),
                body_value(&text),
                &format!("{} {}", request.method, request.path),
            )));
        }
        on_open();
        let policy = self.policy();
        let body = policy.metered(response.bytes_stream());
        let outcome = read_lines(
            Box::pin(body),
            Some(cancel),
            &LineOptions::default(),
            |line| on_line(line),
        )
        .await;
        drop(hold);
        outcome.map_err(Failure::from)
    }

    /// The state report of a listen: the quiet lobby's own report, or the game connection's when
    /// the session it started under is still the current one.
    fn listen_state(
        &self,
        options: &Listen,
        cancel: &CancellationToken,
        phase: &str,
        message: Option<&str>,
    ) {
        if cancel.is_cancelled() {
            return;
        }
        if let Some(quiet) = &options.quiet {
            quiet(phase, message);
        } else if options.session == self.st().epoch {
            self.emit_state(
                options.session,
                &options.account,
                &options.game_id,
                options.lane,
                phase,
                message,
            );
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
            self.listen_state(
                &options,
                &cancel,
                "disconnected",
                Some("Lost connection to Lichess. Reconnect to recover the game."),
            );
            if options.quiet.is_none() && !cancel.is_cancelled() {
                self.emit_error(LOST_CONNECTION);
            }
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
        self.listen_state(options, cancel, "connecting", None);
        while !cancel.is_cancelled() {
            let mut received = false;
            // A malformed line ends the read the way a thrown error does: the line error wins.
            let line_cancel = cancel.child_token();
            let mut line_error: Option<String> = None;
            let mut on_open = || {
                if options.quiet.is_some() {
                    self.listen_state(options, cancel, "connected", None);
                }
            };
            let mut on_line = |line: &str| -> Result<(), CoreError> {
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
                        self.listen_state(options, cancel, "connected", None);
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
            let outcome = match line_error.take() {
                Some(message) => Err(core(message)),
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
                        format!("Stream failed: {account} {game_id} {}", cause.message()),
                    );
                    let status = cause.status();
                    let permanent = status.is_some_and(|code| code < 500 && code != 429);
                    if !options.reconnect || permanent {
                        let phase = if matches!(status, Some(401) | Some(403)) {
                            "auth-required"
                        } else {
                            "disconnected"
                        };
                        self.listen_state(options, cancel, phase, Some(&cause.message()));
                        if options.quiet.is_none() {
                            self.report(&cause);
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
                    "disconnected",
                    Some("The seek ended. Try finding a game again."),
                );
                return;
            }
            failures = if received { 1 } else { failures + 1 };
            if failures > MAX_RECONNECTS {
                self.log(
                    Level::Warn,
                    format!(
                        "Lichess stream gave up reconnecting: {} {} failures={failures}",
                        options.account, options.game_id
                    ),
                );
                self.listen_state(
                    options,
                    cancel,
                    "disconnected",
                    Some("Lost connection to Lichess. Reconnect to recover the game."),
                );
                if options.quiet.is_none() {
                    self.emit_error(LOST_CONNECTION);
                }
                return;
            }
            self.listen_state(
                options,
                cancel,
                "reconnecting",
                Some("Connection interrupted. Reconnecting…"),
            );
            // Backing off ends early when the session is cancelled; the loop then exits.
            let wait = Duration::from_millis((500 * 2u64.pow(failures)).min(15_000));
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(wait) => {}
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
            game_id: if lane == Lane::Game {
                st.live_game.clone()
            } else {
                String::new()
            },
        }
    }

    fn events_request(token: &str) -> StreamRequest {
        StreamRequest {
            method: reqwest::Method::GET,
            path: "/api/stream/event".into(),
            query: Vec::new(),
            auth: Some(authorize(token)),
            form: None,
        }
    }

    /// The event stream of the account being played (`eventStream`): challenges go to the inbox,
    /// a real-time game starting opens it on the board, and the rest is passed on.
    fn start_event_stream(&self, token: String, handle: Handle) {
        let account = self.st().current_account.clone();
        let options = self.listen_options(Lane::Events, true, None, None);
        let this = self.clone();
        let (for_account, for_token, cancel) =
            (account.clone(), token.clone(), handle.token.clone());
        let emit: Emitter = Arc::new(move |event| {
            this.on_event_stream(&for_account, &for_token, &cancel, event);
        });
        let this = self.clone();
        let request = Self::events_request(&token);
        let cancel = handle.token;
        self.spawn_for(&account, "play", async move {
            this.listen(request, cancel, emit, options).await;
        });
    }

    fn on_event_stream(
        &self,
        account: &str,
        token: &str,
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
            let correspondence = event["game"]["speed"].as_str() == Some("correspondence");
            let (live, live_correspondence) = {
                let st = self.st();
                (st.live_game.clone(), st.live_correspondence)
            };
            if game_id != live {
                // Another game is on the board; correspondence games open only when asked for.
                if correspondence || (!live.is_empty() && !live_correspondence) {
                    self.ongoing_changed();
                    return;
                }
                self.st().pending_id.clear();
                self.begin_game(&game_id, token);
            }
            self.announce(&game_id, event);
            return;
        }
        if kind == "gameFinish" {
            self.ongoing_changed();
            let game_id = event["game"]["gameId"].as_str().unwrap_or("").to_string();
            let (live, protected) = {
                let st = self.st();
                (st.live_game.clone(), st.protected_game.clone())
            };
            if game_id != live && game_id != protected {
                return;
            }
        }
        self.emit("online:event", event);
    }

    fn on_challenge(&self, account: &str, event: &Value) {
        if let Some(info) = self.inner.challenges.ingest(account, event) {
            self.emit(
                "challenge:received",
                serde_json::to_value(info).unwrap_or(Value::Null),
            );
        }
    }

    /// `announce`: a game's start reaches the window once, however often the stream reconnects.
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
        self.emit("online:event", event);
    }

    /// `ongoingChanged`: report the change once the burst of stream events has passed.
    fn ongoing_changed(&self) {
        let this = self.clone();
        let mut st = self.st();
        if let Some(timer) = st.ongoing_timer.take() {
            timer.abort();
        }
        st.ongoing_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(ONGOING_DEBOUNCE).await;
            this.emit("online:ongoing-changed", Value::Null);
        }));
    }
}
