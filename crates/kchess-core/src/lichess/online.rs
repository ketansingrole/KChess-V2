//! The online-play session (`OnlineSession` in `crates/kchess-node/js/lichess.ts`): the account's
//! event stream (challenges and pairings), the game stream of the board, the idle lobby stream,
//! seeks and challenge creation, recovery after a restart, and the moves and actions of the live
//! game.
//!
//! Lichess's streams are authoritative: every state change comes from an event or a reply, and
//! reconnection retains the playing account. Every movement carries an epoch (`cancel` moves it),
//! so a late stream or reply from an old session is dropped. An ambiguous move or action is never
//! replayed: each mutation is sent once.
//!
//! What leaves this module, on the host's `emit`: `online:state`, `online:event`, `online:error`,
//! `online:lobby`, `online:ongoing-changed` (debounced 750 ms), `challenge:received` and
//! `challenges:update` (from the inbox). Payloads are the TypeScript shapes; absent optional
//! fields are left out, as `JSON.stringify` drops `undefined`.
//!
//! Deviations kept on purpose and documented: the stream opener is in this file (the shared
//! client has no open hook or form-body stream), so it calls the same policy and error mapping;
//! bodyless POSTs send an empty form body; the presence cache keeps finished answers only.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, now_ms, path_segment};
use super::challenges::{ChallengeInbox, ChallengeInfo, PlayerRef, TimeControl, is_game_id};
use super::client::{Failure, LichessError, TokenSource, authorize, lichess_error};
use super::stream::{LineOptions, read_lines};
use crate::error::CoreError;
use crate::host::{Host, Level};

/// `MAX_RECONNECTS`: how many times a dropped game or event stream is re-opened without progress.
const MAX_RECONNECTS: u32 = 6;
/// `ongoingChanged` debounce: streams that start or end together are reported once.
const ONGOING_DEBOUNCE: Duration = Duration::from_millis(750);
/// `presenceCache`: a status answer is reused this long (`ttl: 4000`), at most 100 of them.
const PRESENCE_TTL: Duration = Duration::from_millis(4000);
const PRESENCE_MAX: usize = 100;
/// The announced-games set keeps this many ids.
const ANNOUNCED_MAX: usize = 200;
const SEEK_REJECTED: &str = "Lichess rejected this public seek. The Board API allows Rapid and slower for matchmaking (Blitz and faster need a direct challenge). Pick a longer control or add a username.";
const LOST_CONNECTION: &str = "Lost connection to Lichess. Check your network and reopen the game.";

/// A failed call or stream, as the session reports it.
pub type Res<T> = std::result::Result<T, Failure>;

/// The session's streams (`lane`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Events,
    Game,
    Seek,
}

impl Lane {
    fn name(self) -> &'static str {
        match self {
            Lane::Events => "events",
            Lane::Game => "game",
            Lane::Seek => "seek",
        }
    }
}

/// `OnlineOptions`: what to play and how; the Board API body of a seek or challenge is built
/// from it (`gameBody`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineOptions {
    /// Ignored for correspondence games (`days`).
    pub minutes: f64,
    pub increment: f64,
    /// Correspondence: days per move instead of a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<f64>,
    /// Lichess variant key; standard when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// A direct challenge from this position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// `random`, `white` or `black`.
    pub color: String,
    pub rated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// What `start` answers: a challenge's id and URL, a seek in progress, or a correspondence game.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correspondence: Option<bool>,
}

/// A game `resume` reattached to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Resumed {
    pub id: String,
    pub account: String,
}

/// A game one of the connected accounts is playing (`OngoingGame`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    pub color: String,
    pub fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<Number>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tournament_id: Option<String>,
}

/// A private chat line of the live game (`ChatLine`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: String,
}

/// One user's status (`UserPresence`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playing_id: Option<String>,
}

/// `PresenceReport`: statuses by lower-cased username, and the request's round trip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    pub users: BTreeMap<String, UserPresence>,
    pub latency_ms: u64,
}

/// `OnlineAction`: what can be done to the live game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnlineAction {
    Resign,
    Abort,
    Takeback,
    DeclineTakeback,
    OfferDraw,
    AcceptDraw,
    DeclineDraw,
    ClaimVictory,
    ClaimDraw,
    Berserk,
}

/// `OnlineHooks` of the service: the session reports these to the host through `emit`.
const CHALLENGE_TYPES: [&str; 3] = ["challenge", "challengeCanceled", "challengeDeclined"];

/// The session's mutable state. Never held across an await.
struct State {
    controller: Option<Handle>,
    game_controller: Option<Handle>,
    next_handle: u64,
    /// The game whose stream is open, while it lasts.
    live_game: String,
    /// Whether the open game is a correspondence game (another may replace it).
    live_correspondence: bool,
    protected_game: String,
    protected_account: String,
    recovery_unverified: bool,
    current_account: String,
    pending_id: String,
    epoch: u64,
    resume_attempt: u64,
    /// `RequestScope` generation of attachments: a newer attachment supersedes an older one.
    attach_generation: u64,
    attaching_account: String,
    /// Games whose start has been announced, so a reconnect does not repeat it.
    announced: Vec<String>,
    lobby_account: String,
    lobby: Option<(String, CancellationToken)>,
    ongoing_timer: Option<JoinHandle<()>>,
    presence: Vec<(String, Instant, PresenceReport)>,
}

impl State {
    fn new() -> State {
        State {
            controller: None,
            game_controller: None,
            next_handle: 0,
            live_game: String::new(),
            live_correspondence: false,
            protected_game: String::new(),
            protected_account: String::new(),
            recovery_unverified: true,
            current_account: String::new(),
            pending_id: String::new(),
            epoch: 0,
            resume_attempt: 0,
            attach_generation: 0,
            attaching_account: String::new(),
            announced: Vec::new(),
            lobby_account: String::new(),
            lobby: None,
            ongoing_timer: None,
            presence: Vec::new(),
        }
    }
}

/// One stream controller: `id` identifies it (the TypeScript compares controller identity).
#[derive(Clone)]
struct Handle {
    id: u64,
    token: CancellationToken,
}

/// What runs before each state is reported: given whether a live game (or the startup check) is in
/// progress and whether assistance is blocked.
type StateHook = Arc<dyn Fn(bool, bool) + Send + Sync>;

struct Inner {
    lichess: Arc<Lichess>,
    tokens: Arc<dyn TokenSource>,
    http: reqwest::Client,
    host: Arc<dyn Host>,
    challenges: ChallengeInbox,
    state: Mutex<State>,
    /// Run before each state is reported: a live game stops the engines and the watch, and the
    /// engines' busy state is refreshed.
    state_hook: Mutex<Option<StateHook>>,
}

/// An online session. Cheap to clone; clones share the state and the streams.
#[derive(Clone)]
pub struct OnlineSession {
    inner: Arc<Inner>,
}

/// One request of a stream: the method, the path, the query, the `Authorization` header value
/// and a form body (for the seek).
#[derive(Clone)]
struct StreamRequest {
    method: reqwest::Method,
    path: String,
    query: Vec<(&'static str, String)>,
    auth: Option<String>,
    form: Option<Vec<(&'static str, String)>>,
}

type Emitter = Arc<dyn Fn(Value) + Send + Sync>;
type Quiet = Arc<dyn Fn(&str, Option<&str>) + Send + Sync>;

/// How one `listen` runs (`ListenOptions`), with the session facts it captured at the start.
#[derive(Clone)]
struct Listen {
    lane: Lane,
    reconnect: bool,
    /// Set by the emitter when the game is over: a clean end of stream does not reconnect.
    finished: Option<Arc<AtomicBool>>,
    /// Reports the stream's health instead of the game connection (the idle lobby stream).
    quiet: Option<Quiet>,
    session: u64,
    account: String,
    game_id: String,
}

fn lock(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn aborted(message: &str) -> Failure {
    Failure::Core(CoreError::aborted(message))
}

fn core(message: impl Into<String>) -> Failure {
    Failure::Core(CoreError::new(message))
}

/// `Number(x)` as JavaScript writes it into a URL or form: integers without a fraction.
pub fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// `String.prototype.slice(0, max)` over UTF-16 units.
fn truncate(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let width = ch.len_utf16();
        if units + width > max {
            break;
        }
        units += width;
        out.push(ch);
    }
    out
}

fn is_challenge_event(kind: &str) -> bool {
    CHALLENGE_TYPES.contains(&kind)
}

/// `validateOnlineEvent(raw)`: the event, or None for an unknown type or a blank line.
fn validate_online_event(raw: &Value) -> std::result::Result<Option<Value>, String> {
    match kchess_domain::records::call("validateOnlineEvent", std::slice::from_ref(raw)) {
        Some(Ok(Value::Null)) | None => Ok(None),
        Some(Ok(event)) => Ok(Some(event)),
        Some(Err(message)) => Err(message),
    }
}

/// `isGameInProgress(status)`.
fn game_in_progress(status: &str) -> bool {
    matches!(
        kchess_domain::records::call("isGameInProgress", &[json!(status)]),
        Some(Ok(Value::Bool(true)))
    )
}

/// `perfFor(minutes, increment)`.
fn perf_for(minutes: f64, increment: f64) -> String {
    match kchess_domain::records::call("perfFor", &[json!(minutes), json!(increment)]) {
        Some(Ok(Value::String(perf))) => perf,
        _ => String::new(),
    }
}

fn rule_bool(method: &str, minutes: f64, increment: f64) -> bool {
    matches!(
        kchess_domain::records::call(method, &[json!(minutes), json!(increment)]),
        Some(Ok(Value::Bool(true)))
    )
}

/// The body of a Lichess error response, as `throwLichessErrors` reads it.
fn body_value(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

fn read_failure(cause: reqwest::Error) -> Failure {
    core(cause.to_string())
}

/// `gameBody(options)`: the form fields of a seek or challenge.
fn game_body(options: &OnlineOptions) -> Vec<(&'static str, String)> {
    let has_fen = options.fen.as_deref().is_some_and(|fen| !fen.is_empty());
    let variant = if has_fen && options.variant.as_deref() != Some("chess960") {
        Some("fromPosition")
    } else {
        options.variant.as_deref()
    };
    let variant = variant.filter(|variant| !variant.is_empty() && *variant != "standard");
    let mut body = vec![
        ("rated", options.rated.to_string()),
        ("color", options.color.clone()),
    ];
    if let Some(variant) = variant {
        body.push(("variant", variant.to_string()));
    }
    if let Some(fen) = &options.fen {
        body.push(("fen", fen.clone()));
    }
    body
}

fn refs<'a>(form: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    form.iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect()
}

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

    fn policy(&self) -> Arc<super::policy::Policy> {
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
                .body(super::client::urlencoded(&refs(form)));
        }
        let built = builder
            .build()
            .map_err(|cause| core(format!("Invalid Lichess request: {cause}")))?;
        let super::policy::Admitted { response, hold } = self.policy().send(built, cancel).await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.map_err(read_failure)?;
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
    async fn listen(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
        emit: Emitter,
        options: Listen,
    ) {
        let mut failures: u32 = 0;
        self.listen_state(&options, &cancel, "connecting", None);
        while !cancel.is_cancelled() {
            let mut received = false;
            // A malformed line ends the read the way a thrown error does: the line error wins.
            let line_cancel = cancel.child_token();
            let mut line_error: Option<String> = None;
            let mut on_open = || {
                if options.quiet.is_some() {
                    self.listen_state(&options, &cancel, "connected", None);
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
                        self.listen_state(&options, &cancel, "connected", None);
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
                .read_stream(&request, &line_cancel, &mut on_open, &mut on_line)
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
                        self.listen_state(&options, &cancel, phase, Some(&cause.message()));
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
                    &options,
                    &cancel,
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
                    &options,
                    &cancel,
                    "disconnected",
                    Some("Lost connection to Lichess. Reconnect to recover the game."),
                );
                if options.quiet.is_none() {
                    self.emit_error(LOST_CONNECTION);
                }
                return;
            }
            self.listen_state(
                &options,
                &cancel,
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

    fn stop_lobby(&self) {
        if let Some((_, token)) = self.st().lobby.take() {
            token.cancel();
        }
    }

    fn start_lobby(&self) {
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
            let token = match this.token(&account).await {
                Ok(token) => token,
                Err(cause) => {
                    this.log(
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
                this.lobby_state(
                    &account,
                    "auth-required",
                    Some("Its Lichess login is unavailable. Reconnect it in Settings."),
                );
                return;
            };
            this.run_lobby(account, token, cancel).await;
        });
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
            if is_challenge_event(&kind) {
                this.on_challenge(&for_account, &event);
            } else if kind == "gameStart"
                && event["game"]["speed"].as_str() != Some("correspondence")
            {
                let game_id = event["game"]["gameId"].as_str().unwrap_or("").to_string();
                let this = this.clone();
                let account = for_account.clone();
                tokio::spawn(async move {
                    if let Err(cause) = this.attach(&account, &game_id).await {
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

    /* ── Sessions ─────────────────────────────────────────────────────── */

    /// `cancel()` without the deferred lobby: the caller is about to claim the streams itself.
    /// Ends every stream, moves the epoch, and reports what remains protected.
    fn cancel_inner(&self) {
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
                Some((id, st.current_account.clone()))
            };
            (phase_report, cancel_pending)
        };
        if let Some((session, account, game, phase, message)) = phase_report {
            self.emit_state(session, &account, &game, Lane::Game, phase, message);
        }
        if let Some((id, account)) = cancel_pending {
            let this = self.clone();
            tokio::spawn(async move {
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
    fn schedule_lobby(&self) {
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
            .accounts()?
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
            .accounts()?
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
        games.sort_by(|a, b| b.is_my_turn.cmp(&a.is_my_turn));
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

    /// `start(options)`: a challenge to a player, a public seek, or a correspondence game.
    pub async fn start(&self, options: &OnlineOptions) -> Res<StartResult> {
        {
            let st = self.st();
            if !st.live_correspondence
                && (!st.live_game.is_empty() || !st.protected_game.is_empty())
            {
                return Err(core(
                    "Reconnect to your current game before finding another.",
                ));
            }
        }
        self.cancel_inner();
        let target = options.target.as_deref().unwrap_or("").trim().to_string();
        let correspondence = options.days.is_some();
        let perf = perf_for(options.minutes, options.increment);
        let label = format!(
            "{}+{}",
            js_number(options.minutes),
            js_number(options.increment)
        );
        // Validate before any stream opens, so a rejected request leaves nothing running.
        let has_fen = options.fen.as_deref().is_some_and(|fen| !fen.is_empty());
        let rejection = if has_fen && (target.is_empty() || options.rated) {
            Some(
                "A game from a set-up position must be a casual challenge to a player.".to_string(),
            )
        } else if !correspondence
            && !target.is_empty()
            && !rule_bool("canDirectChallenge", options.minutes, options.increment)
        {
            Some(format!(
                "Lichess blocks {perf} ({label}) even for direct challenges. The Board API allows Blitz and slower — pick at least 3 minutes of estimated play (time + 40 × increment)."
            ))
        } else if !correspondence
            && target.is_empty()
            && !rule_bool("canBoardSeek", options.minutes, options.increment)
        {
            Some(format!(
                "Lichess blocks {perf} ({label}) for public seeks. The Board API allows Rapid and slower for matchmaking — add a username to challenge directly (Blitz allowed) or pick a longer control."
            ))
        } else {
            None
        };
        if let Some(message) = rejection {
            self.schedule_lobby();
            return Err(core(message));
        }
        let handle = {
            let mut st = self.st();
            let handle = self.next_handle(&mut st);
            st.controller = Some(handle.clone());
            handle
        };
        match self
            .start_inner(options, &target, correspondence, &perf, &label, &handle)
            .await
        {
            Ok(result) => Ok(result),
            Err(cause) => {
                // Don't leave the event stream running for a game the UI never started.
                handle.token.cancel();
                let released = {
                    let mut st = self.st();
                    if st.controller.as_ref().is_some_and(|c| c.id == handle.id) {
                        st.controller = None;
                        true
                    } else {
                        false
                    }
                };
                if released {
                    self.start_lobby();
                }
                Err(cause)
            }
        }
    }

    async fn start_inner(
        &self,
        options: &OnlineOptions,
        target: &str,
        correspondence: bool,
        perf: &str,
        label: &str,
        handle: &Handle,
    ) -> Res<StartResult> {
        let connected: Vec<String> = self
            .inner
            .lichess
            .store()
            .accounts()?
            .into_iter()
            .filter(|row| row.connected)
            .map(|row| row.username)
            .collect();
        if handle.token.is_cancelled() {
            return Err(aborted("This operation was aborted"));
        }
        if connected.is_empty() {
            return Err(core("Connect your Lichess account in Settings first."));
        }
        // An explicit account must match exactly: never play as a different one.
        let account = match options.account.as_deref().filter(|name| !name.is_empty()) {
            Some(wanted) => connected
                .iter()
                .find(|name| name.to_lowercase() == wanted.to_lowercase())
                .cloned()
                .ok_or_else(|| {
                    core(format!(
                        "@{wanted} is not connected. Connect it in Settings."
                    ))
                })?,
            None => connected[0].clone(),
        };
        self.st().current_account = account.clone();
        let token = self.token(&account).await?;
        if handle.token.is_cancelled() {
            return Err(aborted("This operation was aborted"));
        }
        let Some(token) = token else {
            return Err(core(
                "Your Lichess login is unavailable. Reconnect in Settings.",
            ));
        };
        if correspondence {
            // Correspondence games run for days: nothing waits here; the idle stream reports acceptance.
            let created = self
                .create_correspondence(options, &account, &token, &handle.token, target)
                .await?;
            handle.token.cancel();
            {
                let mut st = self.st();
                if st.controller.as_ref().is_some_and(|c| c.id == handle.id) {
                    st.controller = None;
                }
            }
            self.start_lobby();
            return Ok(StartResult {
                id: created.id,
                url: created.url,
                seeking: None,
                correspondence: Some(true),
            });
        }
        self.start_event_stream(token.clone(), handle.clone());
        if !target.is_empty() {
            let mut form: Vec<(&'static str, String)> = vec![
                ("clock.limit", js_number(options.minutes * 60.0)),
                ("clock.increment", js_number(options.increment)),
            ];
            form.extend(game_body(options));
            let path = format!("/api/challenge/{}", path_segment(target));
            // Cancellation does not cut the creation short: Lichess may already have created the
            // challenge, and its id is cancelled below once the answer arrives
            // (`online-session.test.ts`, "cancels a late challenge creation").
            let creation = CancellationToken::new();
            let reply = self
                .post_form(&path, &refs(&form), &account, &token, &creation)
                .await;
            let challenge = match reply {
                Ok(challenge) => challenge,
                Err(_) if handle.token.is_cancelled() => {
                    return Err(aborted("This operation was aborted"));
                }
                Err(cause) => {
                    if matches!(cause.status(), Some(400)) {
                        return Err(core(format!(
                            "Lichess rejected this {perf} ({label}) challenge: {}. The Board API allows Blitz and slower for direct challenges.",
                            cause.message()
                        )));
                    }
                    return Err(cause);
                }
            };
            let id = challenge["id"].as_str().unwrap_or("").to_string();
            // A server may finish creating the challenge just as cancellation arrives.
            if handle.token.is_cancelled() {
                self.cancel_challenge(&id, &token).await;
                return Err(aborted("This operation was aborted"));
            }
            if self.st().live_game.is_empty() {
                self.st().pending_id = id.clone();
            }
            return Ok(StartResult {
                id: Some(id),
                url: challenge["url"].as_str().map(str::to_string),
                seeking: None,
                correspondence: None,
            });
        }
        // A public seek: its stream is the game controller's, and it does not reconnect.
        let seek = {
            let mut st = self.st();
            let seek = self.next_handle(&mut st);
            st.game_controller = Some(seek.clone());
            seek
        };
        let mut form: Vec<(&'static str, String)> = vec![
            ("time", js_number(options.minutes)),
            ("increment", js_number(options.increment)),
        ];
        form.extend(game_body(options));
        let request = StreamRequest {
            method: reqwest::Method::POST,
            path: "/api/board/seek".into(),
            query: Vec::new(),
            auth: Some(authorize(&token)),
            form: Some(form),
        };
        let emit_self = self.clone();
        let emit: Emitter = Arc::new(move |event| emit_self.emit("online:event", event));
        let options_listen = self.listen_options(Lane::Seek, false, None, None);
        let listen_self = self.clone();
        self.spawn_for(&account, "play", async move {
            listen_self
                .listen(request, seek.token, emit, options_listen)
                .await;
        });
        Ok(StartResult {
            id: None,
            url: None,
            seeking: Some(true),
            correspondence: None,
        })
    }

    async fn create_correspondence(
        &self,
        options: &OnlineOptions,
        account: &str,
        token: &str,
        cancel: &CancellationToken,
        target: &str,
    ) -> Res<StartResult> {
        let mut form: Vec<(&'static str, String)> =
            vec![("days", js_number(options.days.unwrap_or_default()))];
        form.extend(game_body(options));
        let path = if target.is_empty() {
            "/api/board/seek".to_string()
        } else {
            format!("/api/challenge/{}", path_segment(target))
        };
        let created = self
            .post_form(&path, &refs(&form), account, token, cancel)
            .await?;
        Ok(StartResult {
            id: created["id"].as_str().map(str::to_string),
            url: created["url"].as_str().map(str::to_string),
            seeking: None,
            correspondence: None,
        })
    }

    /// A form POST as `account`, under the request's cancellation. The answer is JSON.
    async fn post_form(
        &self,
        path: &str,
        form: &[(&str, &str)],
        account: &str,
        token: &str,
        cancel: &CancellationToken,
    ) -> Res<Value> {
        let auth = authorize(token);
        let lichess = Arc::clone(&self.inner.lichess);
        let path = path.to_string();
        let form: Vec<(String, String)> = form
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        let cancel = cancel.clone();
        self.policy()
            .with_usage(account, "play", async move {
                let pairs: Vec<(&str, &str)> = form
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect();
                lichess
                    .client()
                    .post_form::<Value>(&path, &pairs, Some(&auth), &cancel)
                    .await
            })
            .await
    }

    /// `cancelChallenge`: a failed cancel is logged, not raised.
    async fn cancel_challenge(&self, id: &str, token: &str) {
        let path = format!("/api/challenge/{}/cancel", path_segment(id));
        let cancel = self.inner.lichess.lifetime().clone();
        if let Err(cause) = self
            .inner
            .lichess
            .client()
            .post_form::<Value>(&path, &[], Some(&authorize(token)), &cancel)
            .await
        {
            self.log(
                Level::Debug,
                format!("Challenge cancel failed: {id} {}", cause.message()),
            );
        }
    }

    /// Accept an incoming challenge as the account it was sent to; a real-time game opens at once.
    pub async fn accept_challenge(&self, challenge: &ChallengeInfo) -> Res<()> {
        let live = matches!(challenge.time_control, TimeControl::Clock { .. });
        if live {
            let st = self.st();
            if !st.live_game.is_empty() && !st.live_correspondence {
                return Err(core(
                    "Finish the game on the board before accepting another.",
                ));
            }
        }
        let token = self.account_token(&challenge.account).await?;
        let path = format!("/api/challenge/{}/accept", path_segment(&challenge.id));
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&challenge.account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(&path, &[], Some(&authorize(&token)), &cancel)
                    .await
            })
            .await?;
        // The game has the challenge's id. Open it now rather than wait for the event stream,
        // which may belong to another connected account.
        if live {
            self.attach(&challenge.account, &challenge.id).await
        } else {
            self.ongoing_changed();
            Ok(())
        }
    }

    /// Decline an incoming challenge with a reason Lichess shows the challenger.
    pub async fn decline_challenge(&self, challenge: &ChallengeInfo, reason: &str) -> Res<()> {
        let token = self.account_token(&challenge.account).await?;
        let path = format!("/api/challenge/{}/decline", path_segment(&challenge.id));
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&challenge.account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(
                        &path,
                        &[("reason", reason)],
                        Some(&authorize(&token)),
                        &cancel,
                    )
                    .await
            })
            .await?;
        Ok(())
    }

    /// Withdraw an outgoing challenge.
    pub async fn withdraw_challenge(&self, challenge: &ChallengeInfo) -> Res<()> {
        let token = self.account_token(&challenge.account).await?;
        {
            let mut st = self.st();
            if st.pending_id == challenge.id {
                st.pending_id.clear();
            }
        }
        let path = format!("/api/challenge/{}/cancel", path_segment(&challenge.id));
        let cancel = self.inner.lichess.lifetime().clone();
        self.policy()
            .with_usage(&challenge.account, "play", async {
                self.inner
                    .lichess
                    .client()
                    .post_form::<Value>(&path, &[], Some(&authorize(&token)), &cancel)
                    .await
            })
            .await?;
        Ok(())
    }

    /// The token of `account`, or the reconnect message when it has none.
    async fn account_token(&self, account: &str) -> Res<String> {
        self.token(account).await?.ok_or_else(|| {
            core(format!(
                "@{account}'s Lichess login is unavailable. Reconnect in Settings."
            ))
        })
    }

    /// Start the board's stream of game `id` (`openGame`). The state is set before this returns,
    /// so the game counts as played at once; the stream itself runs on a task.
    fn begin_game(&self, id: &str, token: &str) {
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
    fn run_state_hook(&self, live_game: bool, blocked: bool) {
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
        let account = {
            let st = self.st();
            if id != st.live_game {
                return Err(core("Reconnect to this game before sending an action."));
            }
            st.current_account.clone()
        };
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
        self.st().current_account = account.clone();
        let token = self
            .token(&account)
            .await?
            .ok_or_else(|| core("Lichess login unavailable."))?;
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

/// `checking` of a resume: the account being checked, whether it lacked a login, and the failure
/// to report if no account has a game.
#[derive(Default)]
struct Checking {
    account: String,
    auth_required: bool,
    error: Option<Failure>,
}

/// `finished()` of a game controller: the stream it belongs to is still the current one.
fn finished_check(st: &State, handle_id: u64) -> bool {
    st.game_controller
        .as_ref()
        .is_some_and(|controller| controller.id == handle_id)
}

fn ongoing_game(game: &Value, account: &str) -> OngoingGame {
    let opponent = &game["opponent"];
    let ai = opponent["ai"]
        .as_f64()
        .filter(|level| *level != 0.0)
        .is_some();
    let name = if ai {
        format!("Stockfish level {}", js_value_text(&opponent["ai"]))
    } else {
        opponent["username"]
            .as_str()
            .unwrap_or("Opponent")
            .to_string()
    };
    OngoingGame {
        game_id: game["gameId"].as_str().unwrap_or("").to_string(),
        account: account.to_string(),
        opponent: PlayerRef {
            name,
            rating: opponent["rating"].as_number().cloned(),
            title: None,
            provisional: None,
            online: None,
        },
        color: game["color"].as_str().unwrap_or("").to_string(),
        fen: game["fen"].as_str().unwrap_or("").to_string(),
        last_move: game["lastMove"]
            .as_str()
            .filter(|mv| !mv.is_empty())
            .map(str::to_string),
        is_my_turn: game["isMyTurn"].as_bool().unwrap_or(false),
        seconds_left: game["secondsLeft"].as_number().cloned(),
        variant: game["variant"]["key"]
            .as_str()
            .unwrap_or("standard")
            .to_string(),
        speed: game["speed"].as_str().unwrap_or("").to_string(),
        rated: game["rated"].as_bool().unwrap_or(false),
        tournament_id: game["tournamentId"].as_str().map(str::to_string),
    }
}

fn js_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_numbers_print_as_javascript_does() {
        assert_eq!(js_number(600.0), "600");
        assert_eq!(js_number(0.25), "0.25");
        assert_eq!(js_number(0.0), "0");
    }

    #[test]
    fn truncation_counts_utf16_units() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("a😀b", 2), "a");
    }

    #[test]
    fn game_body_follows_the_board_api_rules() {
        let mut options = OnlineOptions {
            minutes: 10.0,
            increment: 0.0,
            days: None,
            variant: Some("chess960".into()),
            fen: None,
            color: "random".into(),
            rated: false,
            target: None,
            account: None,
        };
        assert_eq!(
            game_body(&options),
            vec![
                ("rated", "false".to_string()),
                ("color", "random".to_string()),
                ("variant", "chess960".to_string()),
            ]
        );
        options.variant = Some("standard".into());
        options.fen = Some("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1".into());
        let body = game_body(&options);
        assert!(body.contains(&("variant", "fromPosition".to_string())));
        assert!(body.iter().any(|(key, _)| *key == "fen"));
    }
}
