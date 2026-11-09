//! Lichess request admission and the request accounting (`withUsage`, `attributeTo`,
//! `meteredFetch`, `forgetUsage`).
//!
//! Admission: at most two requests are in flight, interactive requests (game moves and their
//! resign/abort/draw/takeback family) go before the rest, a 429 starts a cooldown that the next
//! request waits out (`Retry-After`, capped at 5 minutes, 60 seconds when absent) without holding
//! a lane, and that interactive requests wait out only when one of them was refused, a request
//! that is not a stream has 30 seconds to answer, and game exports hold a lane of their own
//! until their body is read, cancelled or fails. Nothing here retries: an ambiguous mutation is
//! never replayed.
//!
//! Accounting: each request is counted against the account and kind of the task that made it
//! (`with_usage`). The counters leave as `host:usage` events
//! `{ account, category, requests, bytes }`, the same shape the puzzle download reports, and the
//! host stores them. Traffic from an account whose epoch moved on (logout, removal, clearing) is
//! not counted.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Stream, TryStreamExt};
use reqwest::header::HeaderMap;
use serde_json::json;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, ErrorKind, Result};
use crate::host::{Host, Level};

/// A request may take this long to answer (its headers, and its body unless it is a stream).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ACTIVE: usize = 2;
const MAX_WAITING: usize = 500;
/// A backstop for an export body dropped without being read or cancelled.
const BULK_HOLD: Duration = Duration::from_secs(600);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
/// The shortest cooldown, in seconds: `Retry-After: 0` must not let a refused burst repeat at once.
const MIN_COOLDOWN: f64 = 1.0;

/// `UNATTRIBUTED`: traffic made outside any `with_usage` scope.
const UNATTRIBUTED_KIND: &str = "other";

/// Who a request is counted against: the account (lowercase), the kind, and the account's epoch
/// when the request started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageContext {
    pub account: String,
    pub kind: String,
    pub epoch: u64,
}

type UsageCell = Arc<Mutex<Option<UsageContext>>>;

tokio::task_local! {
    /// The usage context of the task that is running.
    static USAGE: UsageCell;
}

/// The context of the running task, if any `with_usage` scope or `attribute_to` set one.
pub fn current_usage() -> Option<UsageContext> {
    USAGE.try_with(|cell| lock(cell).clone()).ok().flatten()
}

/// A permit of the interactive/background lanes, from the moment it is queued; returned to the
/// pool (or taken out of the queue) when dropped, including by a caller dropped mid-admission.
struct Slot {
    policy: Arc<Policy>,
    id: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut lanes = lock(&self.policy.lanes);
        if lanes.running.remove(&self.id) {
            lanes.active -= 1;
        } else if let Some(at) = lanes.waiting.iter().position(|waiter| waiter.id == self.id) {
            lanes.waiting.remove(at);
        }
        pump(&mut lanes);
    }
}

struct Waiter {
    id: u64,
    priority: u8,
    start: oneshot::Sender<()>,
}

#[derive(Default)]
struct Lanes {
    active: usize,
    running: HashSet<u64>,
    waiting: Vec<Waiter>,
    next: u64,
    bulk_active: bool,
    bulk_waiting: VecDeque<(u64, oneshot::Sender<()>)>,
}

/// Hands the freed slots to the highest-priority waiters (stable among equal priorities).
fn pump(lanes: &mut Lanes) {
    lanes.waiting.sort_by_key(|a| std::cmp::Reverse(a.priority));
    while lanes.active < MAX_ACTIVE && !lanes.waiting.is_empty() {
        let waiter = lanes.waiting.remove(0);
        lanes.active += 1;
        lanes.running.insert(waiter.id);
        if waiter.start.send(()).is_err() {
            // The waiter gave up while it was being started: the slot is not used.
            lanes.running.remove(&waiter.id);
            lanes.active -= 1;
        }
    }
}

/// A place in the bulk lane's queue until it is answered.
struct BulkWait<'a> {
    policy: &'a Arc<Policy>,
    id: u64,
    armed: bool,
}

impl Drop for BulkWait<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut lanes = lock(&self.policy.lanes);
        if let Some(at) = lanes
            .bulk_waiting
            .iter()
            .position(|(waiter, _)| *waiter == self.id)
        {
            lanes.bulk_waiting.remove(at);
        } else {
            // The lane was handed over before the wait ended.
            drop(lanes);
            self.policy.release_bulk();
        }
    }
}

/// Held for an export body: the bulk lane passes to the next export when this is released.
pub struct BulkHold {
    inner: Arc<HoldInner>,
    /// The backstop timer: it ends with the hold, so it never keeps the policy (and through it
    /// the host, the HTTP client and the usage database) alive for its ten minutes.
    backstop: tokio::task::AbortHandle,
}

struct HoldInner {
    policy: Arc<Policy>,
    released: AtomicBool,
}

impl HoldInner {
    fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            self.policy.release_bulk();
        }
    }
}

impl std::fmt::Debug for BulkHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BulkHold")
    }
}

impl Drop for BulkHold {
    fn drop(&mut self) {
        self.backstop.abort();
        self.inner.release();
    }
}

/// A response the policy admitted. `hold` is present for a game export whose body is still
/// to be read; dropping it (after the body is read, cancelled or failed) frees the lane.
#[derive(Debug)]
pub struct Admitted {
    pub response: reqwest::Response,
    pub hold: Option<BulkHold>,
}

#[derive(Default)]
struct Accounting {
    epochs: HashMap<String, u64>,
}

/// The shared admission state of one Lichess client. One per process; every client shares its cooldown and lanes.
pub struct Policy {
    host: Arc<dyn Host>,
    http: reqwest::Client,
    lifetime: CancellationToken,
    timeout: Duration,
    lanes: Mutex<Lanes>,
    /// A 429 answered to a background request (a sync, an export): background requests wait.
    cooldown_until: Mutex<Option<Instant>>,
    /// A 429 answered to an interactive request: every request waits.
    interactive_cooldown_until: Mutex<Option<Instant>>,
    accounting: Mutex<Accounting>,
}

use crate::tasks::lock;

/// A request Lichess did not answer within its deadline.
pub(crate) fn timed_out() -> CoreError {
    CoreError::new("Lichess didn’t answer in time.").with_kind(ErrorKind::TimedOut)
}

fn aborted() -> CoreError {
    CoreError::aborted("This operation was aborted")
}

/// Whether `url` is a bulk game export (`/api/games/user/` or `/api/games/export/`).
pub fn is_bulk_export(url: &str) -> bool {
    url.contains("/api/games/user/") || url.contains("/api/games/export/")
}

/// Whether `url` answers with a long-lived stream, which owns its own lifetime: the header
/// deadline does not apply to it, nor does the body deadline.
pub fn is_stream(url: &str, content_type: Option<&str>) -> bool {
    if content_type.is_some_and(|value| value.contains("ndjson")) {
        return true;
    }
    if url.contains("/api/stream/")
        || url.contains("/api/board/game/stream")
        || url.contains("/api/board/seek")
        || url.contains("/api/games/user")
        || url.contains("/api/puzzle/activity")
    {
        return true;
    }
    if let Some(at) = url.find("/api/tv/") {
        let rest = &url[at + "/api/tv/".len()..];
        if let Some(slash) = rest.find('/') {
            return slash > 0 && rest[slash..].starts_with("/feed");
        }
    }
    false
}

/// Whether `url` is a live stream (events, games, seeks, TV feeds), whose lines must arrive the
/// moment Lichess writes them: such requests ask for an uncompressed body, so no compressor on
/// the way buffers a move. Bulk exports stream too but are read whole, and stay compressed.
pub fn is_live_stream(url: &str) -> bool {
    is_stream(url, None)
        && !url.contains("/api/games/user")
        && !url.contains("/api/puzzle/activity")
}

/// Whether `url` is an interactive game request (move, resign, abort, takeback, draw, claim,
/// berserk) that goes before background reads.
pub fn is_foreground(url: &str) -> bool {
    const PREFIX: &str = "/api/board/game/";
    let Some(at) = url.find(PREFIX) else {
        return false;
    };
    let rest = &url[at + PREFIX.len() - 1..];
    const MIDDLE: [&str; 5] = ["/move/", "/resign/", "/abort/", "/takeback/", "/draw/"];
    if MIDDLE.iter().any(|segment| rest.contains(segment)) {
        return true;
    }
    const END: [&str; 5] = [
        "/resign",
        "/abort",
        "/claim-victory",
        "/claim-draw",
        "/berserk",
    ];
    END.iter()
        .any(|segment| rest.len() > segment.len() && rest.ends_with(segment))
}

/// The cooldown a 429 starts: `Retry-After` in seconds, at most 5 minutes; 60 seconds when the
/// header is missing or not a number, and at least a second when it says 0.
pub fn cooldown_for(headers: &HeaderMap) -> Duration {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse::<f64>().ok());
    match seconds {
        Some(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Duration::from_secs_f64(seconds.clamp(MIN_COOLDOWN, MAX_COOLDOWN.as_secs_f64()))
        }
        _ => DEFAULT_COOLDOWN,
    }
}

impl Policy {
    /// A failure for the log: what a person is told is worded elsewhere.
    pub(crate) fn log_warning(&self, message: &str) {
        self.host.log(Level::Warn, "lichess", message);
    }

    /// `host` receives the `host:usage` events and the warnings; `lifetime` ends the accounting
    /// when the core closes.
    pub fn new(
        host: Arc<dyn Host>,
        http: reqwest::Client,
        lifetime: CancellationToken,
    ) -> Arc<Policy> {
        Policy::with_timeout(host, http, lifetime, REQUEST_TIMEOUT)
    }

    /// As `new`, with another request deadline (tests use a short one).
    pub fn with_timeout(
        host: Arc<dyn Host>,
        http: reqwest::Client,
        lifetime: CancellationToken,
        timeout: Duration,
    ) -> Arc<Policy> {
        Arc::new(Policy {
            host,
            http,
            lifetime,
            timeout,
            lanes: Mutex::new(Lanes::default()),
            cooldown_until: Mutex::new(None),
            interactive_cooldown_until: Mutex::new(None),
            accounting: Mutex::new(Accounting::default()),
        })
    }

    /// The request deadline of this policy.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The epoch of `account` (lowercase). Usage started under an older epoch is dropped.
    pub fn epoch(&self, account: &str) -> u64 {
        lock(&self.accounting)
            .epochs
            .get(&account.to_lowercase())
            .copied()
            .unwrap_or(0)
    }

    /// `withUsage`: run `work` so every request it makes is counted against `account`.
    pub async fn with_usage<T, F>(&self, account: &str, kind: &str, work: F) -> T
    where
        F: Future<Output = T>,
    {
        let account = account.to_lowercase();
        let context = UsageContext {
            epoch: self.epoch(&account),
            account,
            kind: kind.to_string(),
        };
        USAGE.scope(Arc::new(Mutex::new(Some(context))), work).await
    }

    /// `attributeTo`: count the rest of the current task against `account`. Outside a
    /// `with_usage` scope there is no task context to set, so it does nothing there.
    pub fn attribute_to(&self, account: &str, kind: &str) {
        let account = account.to_lowercase();
        let context = UsageContext {
            epoch: self.epoch(&account),
            account,
            kind: kind.to_string(),
        };
        let _ = USAGE.try_with(|cell| *lock(cell) = Some(context));
    }

    fn record(&self, context: &UsageContext, requests: u64, bytes: u64) {
        if self.lifetime.is_cancelled() || (requests == 0 && bytes == 0) {
            return;
        }
        if context.epoch != self.epoch(&context.account) {
            return;
        }
        self.host.emit(
            "host:usage",
            json!({
                "account": context.account,
                "category": context.kind,
                "requests": requests,
                "bytes": bytes,
            }),
        );
    }

    /// `forgetUsage`: traffic still arriving for these accounts is no longer counted.
    pub fn forget_usage(&self, accounts: &[String]) {
        let mut accounting = lock(&self.accounting);
        for name in accounts {
            *accounting.epochs.entry(name.to_lowercase()).or_insert(0) += 1;
        }
    }

    /// The body of a response, passed through with its bytes counted against the task that
    /// made the request (`meteredFetch`).
    pub fn metered<S, B, E>(
        self: &Arc<Self>,
        body: S,
    ) -> impl Stream<Item = std::result::Result<B, E>>
    where
        S: Stream<Item = std::result::Result<B, E>>,
        B: AsRef<[u8]>,
    {
        let policy = Arc::clone(self);
        let context = current_usage().unwrap_or_else(unattributed);
        body.inspect_ok(move |chunk| policy.record(&context, 0, chunk.as_ref().len() as u64))
    }

    /// Admit `request` under the lanes, the cooldown and the deadline, and count it.
    /// `cancel` ends the wait, the request and (for a stream) the body hand-off, with an abort.
    pub async fn send(
        self: &Arc<Self>,
        request: reqwest::Request,
        cancel: &CancellationToken,
    ) -> Result<Admitted> {
        let bulk = is_bulk_export(request.url().as_str());
        self.send_in(request, bulk, cancel).await
    }

    /// `send` for one game from the export endpoint: a small answer that does not wait for the
    /// bulk lane, which a running sync holds until its whole download ends.
    pub async fn send_single(
        self: &Arc<Self>,
        request: reqwest::Request,
        cancel: &CancellationToken,
    ) -> Result<Admitted> {
        self.send_in(request, false, cancel).await
    }

    async fn send_in(
        self: &Arc<Self>,
        request: reqwest::Request,
        bulk: bool,
        cancel: &CancellationToken,
    ) -> Result<Admitted> {
        if cancel.is_cancelled() {
            return Err(aborted());
        }
        let url = request.url().to_string();
        let context = current_usage().unwrap_or_else(unattributed);
        let hold = if bulk {
            Some(self.acquire_bulk(cancel).await?)
        } else {
            None
        };
        let response = match self.admitted(request, &url, cancel).await {
            Ok(response) => response,
            Err(cause) => {
                drop(hold);
                return Err(cause);
            }
        };
        self.record(&context, 1, 0);
        if !response.status().is_success() {
            // A failed export does not keep the lane (`releaseWithBody`).
            drop(hold);
            return Ok(Admitted {
                response,
                hold: None,
            });
        }
        Ok(Admitted { response, hold })
    }

    async fn admitted(
        self: &Arc<Self>,
        mut request: reqwest::Request,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response> {
        let interactive = is_foreground(url);
        let _slot = self.admit(interactive, cancel).await?;
        if is_live_stream(url) {
            request.headers_mut().insert(
                reqwest::header::ACCEPT_ENCODING,
                reqwest::header::HeaderValue::from_static("identity"),
            );
        }
        let fut = self.http.execute(request);
        // The deadline covers the headers; `read_body` extends it to the body of a non-stream.
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(aborted()),
            answer = tokio::time::timeout(self.timeout, fut) => match answer {
                Ok(Ok(response)) => response,
                Ok(Err(cause)) => {
                    // The cause is for the log: it names the address, which says nothing to a player.
                    // Its sources too: reqwest's own text is the same for a refused, reset or
                    // timed-out connection, and only the OS error beneath says which it was.
                    self.host.log(Level::Warn, "request-policy", &format!("Lichess request failed: {}", with_sources(&cause)));
                    return Err(CoreError::new("Couldn’t reach Lichess").with_kind(ErrorKind::Offline));
                }
                Err(_) => return Err(timed_out()),
            },
        };
        if response.status().as_u16() == 429 {
            let cooldown = cooldown_for(response.headers());
            let until = if interactive {
                &self.interactive_cooldown_until
            } else {
                &self.cooldown_until
            };
            *lock(until) = Some(Instant::now() + cooldown);
            let retry = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .to_string();
            let path: String = url_path(url).chars().take(80).collect();
            self.host.log(
                Level::Warn,
                "request-policy",
                &format!(
                    "Lichess rate limited: {path} retryAfter={retry} cooldownMs={}",
                    cooldown.as_millis()
                ),
            );
        }
        Ok(response)
    }

    /// What is left of the cooldown a request of this kind waits out. A background request that
    /// Lichess rate-limited (a game sync during a blitz game) must not hold up the player's moves,
    /// which have limits of their own.
    fn cooldown_left(&self, interactive: bool) -> Option<Duration> {
        let now = Instant::now();
        let mut until = *lock(&self.interactive_cooldown_until);
        if !interactive {
            until = until.max(*lock(&self.cooldown_until));
        }
        until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|left| !left.is_zero())
    }

    /// A lane for a request, once its cooldown is over. The cooldown is waited out without a lane,
    /// so requests sleeping through it never keep the ones it does not apply to from running.
    async fn admit(
        self: &Arc<Self>,
        interactive: bool,
        cancel: &CancellationToken,
    ) -> Result<Slot> {
        let priority = if interactive { 2 } else { 1 };
        loop {
            if let Some(left) = self.cooldown_left(interactive) {
                tokio::select! {
                    () = tokio::time::sleep(left) => {}
                    () = cancel.cancelled() => return Err(aborted()),
                }
            }
            let slot = self.acquire_slot(priority, cancel).await?;
            // A 429 that arrived while this waited for its lane starts another wait.
            if self.cooldown_left(interactive).is_none() {
                return Ok(slot);
            }
        }
    }

    async fn acquire_slot(
        self: &Arc<Self>,
        priority: u8,
        cancel: &CancellationToken,
    ) -> Result<Slot> {
        let (id, receive) = {
            let mut lanes = lock(&self.lanes);
            if cancel.is_cancelled() {
                return Err(aborted());
            }
            if lanes.waiting.len() >= MAX_WAITING {
                return Err(CoreError::new(
                    "Too many Lichess requests are pending. Retry shortly.",
                ));
            }
            lanes.next += 1;
            let id = lanes.next;
            let (start, receive) = oneshot::channel();
            lanes.waiting.push(Waiter {
                id,
                priority,
                start,
            });
            pump(&mut lanes);
            (id, receive)
        };
        // Owns the place in the queue or the started slot: a cancelled or dropped wait gives it back.
        let slot = Slot {
            policy: Arc::clone(self),
            id,
        };
        tokio::select! {
            biased;
            answer = receive => match answer {
                Ok(()) => Ok(slot),
                Err(_) => Err(aborted()),
            },
            _ = cancel.cancelled() => Err(aborted()),
        }
    }

    async fn acquire_bulk(self: &Arc<Self>, cancel: &CancellationToken) -> Result<BulkHold> {
        let (id, receive) = {
            let mut lanes = lock(&self.lanes);
            if cancel.is_cancelled() {
                return Err(aborted());
            }
            if !lanes.bulk_active {
                lanes.bulk_active = true;
                return Ok(self.hold());
            }
            lanes.next += 1;
            let id = lanes.next;
            let (start, receive) = oneshot::channel();
            lanes.bulk_waiting.push_back((id, start));
            (id, receive)
        };
        // A cancelled or dropped wait leaves the queue, or passes on a lane already handed to it.
        let mut waiting = BulkWait {
            policy: self,
            id,
            armed: true,
        };
        let answer = tokio::select! {
            biased;
            answer = receive => answer,
            _ = cancel.cancelled() => return Err(aborted()),
        };
        waiting.armed = false;
        match answer {
            Ok(()) => Ok(self.hold()),
            Err(_) => Err(aborted()),
        }
    }

    fn hold(self: &Arc<Self>) -> BulkHold {
        let inner = Arc::new(HoldInner {
            policy: Arc::clone(self),
            released: AtomicBool::new(false),
        });
        let held = Arc::downgrade(&inner);
        let backstop = tokio::spawn(async move {
            tokio::time::sleep(BULK_HOLD).await;
            if let Some(inner) = held.upgrade() {
                inner.release();
            }
        })
        .abort_handle();
        BulkHold { inner, backstop }
    }

    /// Pass the export lane to the next export that is waiting, or free it.
    fn release_bulk(&self) {
        let mut lanes = lock(&self.lanes);
        while let Some((_, start)) = lanes.bulk_waiting.pop_front() {
            if start.send(()).is_ok() {
                return;
            }
        }
        lanes.bulk_active = false;
    }
}

fn unattributed() -> UsageContext {
    UsageContext {
        account: String::new(),
        kind: UNATTRIBUTED_KIND.to_string(),
        epoch: 0,
    }
}

/// The path of a URL, without its origin or query (for logs).
/// `cause` followed by each error beneath it, as `a: b: c`.
fn with_sources(cause: &dyn std::error::Error) -> String {
    let mut text = cause.to_string();
    let mut source = cause.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

fn url_path(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(url, |at| &url[at + 3..]);
    let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let path = &after_scheme[path_start..];
    path.split(['?', '#']).next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// reqwest says "error sending request" for every failed connection; the log carries what
    /// lies beneath, so a failure on CI says whether it was refused, reset or out of ports.
    #[tokio::test]
    async fn a_failed_request_is_logged_with_its_causes() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .expect("a free port");
        let cause = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err("nothing listens there");
        let text = with_sources(&cause);
        assert!(text.starts_with(&cause.to_string()), "{text}");
        assert!(text.len() > cause.to_string().len(), "{text}");
    }

    #[test]
    fn classifies_interactive_and_stream_urls() {
        assert!(is_foreground(
            "https://lichess.org/api/board/game/abcdefgh/move/e2e4"
        ));
        assert!(is_foreground(
            "https://lichess.org/api/board/game/abcdefgh/resign"
        ));
        assert!(!is_foreground("https://lichess.org/api/account"));
        assert!(is_bulk_export(
            "https://lichess.org/api/games/user/Alice?max=5"
        ));
        assert!(is_bulk_export("https://lichess.org/api/games/export/_ids"));
        assert!(!is_bulk_export("https://lichess.org/api/user/Alice"));
        assert!(is_stream("https://lichess.org/api/stream/event", None));
        assert!(is_stream("https://lichess.org/api/tv/rapid/feed", None));
        assert!(!is_stream("https://lichess.org/api/tv/feed", None));
        assert!(is_live_stream("https://lichess.org/api/stream/event"));
        assert!(is_live_stream(
            "https://lichess.org/api/board/game/stream/abcd1234"
        ));
        assert!(is_live_stream("https://lichess.org/api/tv/rapid/feed"));
        assert!(!is_live_stream(
            "https://lichess.org/api/games/user/someone?max=50"
        ));
        assert!(!is_live_stream("https://lichess.org/api/puzzle/activity"));
        assert!(!is_live_stream("https://lichess.org/api/account"));
        assert!(is_stream(
            "https://lichess.org/api/account",
            Some("application/x-ndjson")
        ));
        assert!(!is_stream(
            "https://lichess.org/api/account",
            Some("application/json")
        ));
    }

    #[test]
    fn cooldown_follows_retry_after() {
        let mut headers = HeaderMap::new();
        assert_eq!(cooldown_for(&headers), Duration::from_secs(60));
        headers.insert(reqwest::header::RETRY_AFTER, "20".parse().unwrap());
        assert_eq!(cooldown_for(&headers), Duration::from_secs(20));
        headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(cooldown_for(&headers), Duration::from_secs(300));
        headers.insert(reqwest::header::RETRY_AFTER, "soon".parse().unwrap());
        assert_eq!(cooldown_for(&headers), Duration::from_secs(60));
        // "Retry at once" used to become the 60-second default.
        headers.insert(reqwest::header::RETRY_AFTER, "0".parse().unwrap());
        assert_eq!(cooldown_for(&headers), Duration::from_secs(1));
    }

    /// A game sync rate-limited during a blitz game set one cooldown for every request, and the
    /// next move slept through it (60 s and more) holding a lane: the player lost on time. Now a
    /// background 429 holds back background requests only, and they wait without a lane.
    #[tokio::test]
    async fn a_rate_limited_sync_does_not_hold_up_moves() {
        let policy = quiet_policy();
        let cancel = CancellationToken::new();
        *lock(&policy.cooldown_until) = Some(Instant::now() + Duration::from_secs(60));
        let mut sync = Box::pin(policy.admit(false, &cancel));
        let mut other = Box::pin(policy.admit(false, &cancel));
        queue(&mut sync).await;
        queue(&mut other).await;
        assert_eq!(lock(&policy.lanes).active, 0, "waiting without a lane");
        let first = tokio::time::timeout(Duration::from_secs(1), policy.admit(true, &cancel))
            .await
            .expect("a move is admitted at once")
            .expect("slot");
        drop(first);
        // A 429 on a move holds back everything.
        *lock(&policy.interactive_cooldown_until) = Some(Instant::now() + Duration::from_secs(60));
        let mut moved = Box::pin(policy.admit(true, &cancel));
        queue(&mut moved).await;
        cancel.cancel();
        assert!(sync.await.is_err() && other.await.is_err() && moved.await.is_err());
    }

    struct Quiet;

    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: serde_json::Value) {}
    }

    fn quiet_policy() -> Arc<Policy> {
        Policy::new(
            Arc::new(Quiet),
            reqwest::Client::new(),
            CancellationToken::new(),
        )
    }

    /// Polls `future` once, so it queues, without waiting for it.
    async fn queue<F: Future + Unpin>(future: &mut F) {
        tokio::select! {
            biased;
            _ = future => panic!("admitted at once"),
            () = std::future::ready(()) => {}
        }
    }

    /// A request dropped after its slot was granted but before it ran again (a client that went
    /// away, its call aborted) gives the slot back instead of keeping it forever.
    #[tokio::test]
    async fn a_dropped_wait_returns_a_slot_granted_to_it() {
        let policy = quiet_policy();
        let cancel = CancellationToken::new();
        let first = policy.acquire_slot(0, &cancel).await.expect("first slot");
        let second = policy.acquire_slot(0, &cancel).await.expect("second slot");
        let mut third = Box::pin(policy.acquire_slot(0, &cancel));
        queue(&mut third).await;
        drop(first);
        assert_eq!(lock(&policy.lanes).running.len(), 2, "granted to the third");
        drop(third);
        drop(second);
        let lanes = lock(&policy.lanes);
        assert_eq!(lanes.active, 0);
        assert!(lanes.running.is_empty() && lanes.waiting.is_empty());
    }

    /// The same for the bulk lane: handed to a wait that is then dropped, it passes on.
    #[tokio::test]
    async fn a_dropped_wait_passes_on_the_bulk_lane_handed_to_it() {
        let policy = quiet_policy();
        let cancel = CancellationToken::new();
        let first = policy.acquire_bulk(&cancel).await.expect("the lane");
        let mut second = Box::pin(policy.acquire_bulk(&cancel));
        queue(&mut second).await;
        drop(first);
        drop(second);
        let lanes = lock(&policy.lanes);
        assert!(!lanes.bulk_active, "the lane is free");
        assert!(lanes.bulk_waiting.is_empty());
    }

    /// A finished export ends its backstop timer: before, the timer held the policy (and a
    /// closed core's host, HTTP client and usage database with it) for ten minutes.
    #[tokio::test]
    async fn a_released_export_lets_the_policy_go() {
        let policy = quiet_policy();
        let cancel = CancellationToken::new();
        let hold = policy.acquire_bulk(&cancel).await.expect("the lane");
        assert_eq!(Arc::strong_count(&policy), 2, "the hold keeps the policy");
        drop(hold);
        tokio::task::yield_now().await;
        assert_eq!(Arc::strong_count(&policy), 1, "nothing else keeps it");
        assert!(!lock(&policy.lanes).bulk_active, "the lane is free");
    }

    #[test]
    fn logs_only_the_path() {
        assert_eq!(
            url_path("https://lichess.org/api/user/Alice?token=x"),
            "/api/user/Alice"
        );
    }
}
