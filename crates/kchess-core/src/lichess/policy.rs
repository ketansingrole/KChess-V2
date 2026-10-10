//! Lichess request admission (`crates/kchess-node/js/requestPolicy.ts`) and the request accounting of
//! `crates/kchess-node/js/usage.ts` (`withUsage`, `attributeTo`, `meteredFetch`, `recordUsage`,
//! `forgetUsage`).
//!
//! Admission: at most two requests are in flight, interactive requests (game moves and their
//! resign/abort/draw/takeback family) go before the rest, a 429 starts a cooldown that the next
//! request waits out (`Retry-After`, capped at 5 minutes, 60 seconds when absent), a request
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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::{Stream, TryStreamExt};
use reqwest::header::HeaderMap;
use serde_json::json;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// A request may take this long to answer (its headers, and its body unless it is a stream).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ACTIVE: usize = 2;
const MAX_WAITING: usize = 500;
/// A backstop for an export body dropped without being read or cancelled.
const BULK_HOLD: Duration = Duration::from_secs(600);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);

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
    /// The `AsyncLocalStorage` of `usage.ts`: the context of the task that is running.
    static USAGE: UsageCell;
}

/// The context of the running task, if any `with_usage` scope or `attribute_to` set one.
pub fn current_usage() -> Option<UsageContext> {
    USAGE.try_with(|cell| lock(cell).clone()).ok().flatten()
}

/// A permit of the interactive/background lanes; returned to the pool when dropped.
struct Slot {
    policy: Arc<Policy>,
    id: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut lanes = lock(&self.policy.lanes);
        if lanes.running.remove(&self.id) {
            lanes.active -= 1;
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
    lanes.waiting.sort_by(|a, b| b.priority.cmp(&a.priority));
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

/// Held for an export body: the bulk lane passes to the next export when this is released.
pub struct BulkHold {
    inner: Arc<HoldInner>,
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

/// The shared admission state of one Lichess client (`serviceState` of `requestPolicy.ts` and
/// `usage.ts`). One per process; every client shares its cooldown and lanes.
pub struct Policy {
    host: Arc<dyn Host>,
    http: reqwest::Client,
    lifetime: CancellationToken,
    timeout: Duration,
    lanes: Mutex<Lanes>,
    cooldown_until: Mutex<Option<Instant>>,
    accounting: Mutex<Accounting>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panicked holder leaves the counters consistent enough to keep serving; poisoning must
    // not take every Lichess request down with it.
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn aborted() -> CoreError {
    CoreError::aborted("This operation was aborted")
}

/// Whether `url` is a bulk game export (`/api/games/user/` or `/api/games/export/`).
pub fn is_bulk_export(url: &str) -> bool {
    url.contains("/api/games/user/") || url.contains("/api/games/export/")
}

/// Whether `url` answers with a long-lived stream, which owns its own lifetime: the header
/// deadline does not apply to it (`requestPolicy.ts`'s stream list), nor does the body deadline.
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
/// header is missing or not a positive number.
pub fn cooldown_for(headers: &HeaderMap) -> Duration {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse::<f64>().ok());
    match seconds {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            Duration::from_secs_f64(seconds.min(MAX_COOLDOWN.as_secs_f64()))
        }
        _ => DEFAULT_COOLDOWN,
    }
}

impl Policy {
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

    /// `recordUsage`: count `requests` and `bytes` for `account` now.
    pub fn record_usage(&self, account: &str, kind: &str, requests: u64, bytes: u64) {
        let account = account.to_lowercase();
        let context = UsageContext {
            epoch: self.epoch(&account),
            account,
            kind: kind.to_string(),
        };
        self.record(&context, requests, bytes);
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
        if cancel.is_cancelled() {
            return Err(aborted());
        }
        let url = request.url().to_string();
        let context = current_usage().unwrap_or_else(unattributed);
        let hold = if is_bulk_export(&url) {
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
        request: reqwest::Request,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response> {
        let priority = if is_foreground(url) { 2 } else { 1 };
        let _slot = self.acquire_slot(priority, cancel).await?;
        let cooldown = *lock(&self.cooldown_until);
        if let Some(until) = cooldown {
            let now = Instant::now();
            if until > now {
                tokio::select! {
                    _ = tokio::time::sleep(until - now) => {}
                    _ = cancel.cancelled() => return Err(aborted()),
                }
            }
        }
        let fut = self.http.execute(request);
        // The deadline covers the headers; `read_body` extends it to the body of a non-stream.
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(aborted()),
            answer = tokio::time::timeout(self.timeout, fut) => match answer {
                Ok(Ok(response)) => response,
                Ok(Err(cause)) => return Err(CoreError::new(cause.to_string())),
                Err(_) => return Err(CoreError::new("Lichess request timed out. Retry or reconnect.")),
            },
        };
        if response.status().as_u16() == 429 {
            let cooldown = cooldown_for(response.headers());
            *lock(&self.cooldown_until) = Some(Instant::now() + cooldown);
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
        tokio::select! {
            biased;
            answer = receive => match answer {
                Ok(()) => Ok(Slot { policy: Arc::clone(self), id }),
                Err(_) => Err(aborted()),
            },
            _ = cancel.cancelled() => {
                let mut lanes = lock(&self.lanes);
                if let Some(at) = lanes.waiting.iter().position(|waiter| waiter.id == id) {
                    lanes.waiting.remove(at);
                } else if lanes.running.remove(&id) {
                    // Started just before the cancellation: give the slot back.
                    lanes.active -= 1;
                    pump(&mut lanes);
                }
                Err(aborted())
            }
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
        tokio::select! {
            biased;
            answer = receive => match answer {
                Ok(()) => Ok(self.hold()),
                Err(_) => Err(aborted()),
            },
            _ = cancel.cancelled() => {
                let mut lanes = lock(&self.lanes);
                if let Some(at) = lanes.bulk_waiting.iter().position(|(waiter, _)| *waiter == id) {
                    lanes.bulk_waiting.remove(at);
                    Err(aborted())
                } else {
                    // The lane was handed over just before the cancellation.
                    drop(lanes);
                    self.release_bulk();
                    Err(aborted())
                }
            }
        }
    }

    fn hold(self: &Arc<Self>) -> BulkHold {
        let inner = Arc::new(HoldInner {
            policy: Arc::clone(self),
            released: AtomicBool::new(false),
        });
        let backstop = Arc::clone(&inner);
        tokio::spawn(async move {
            tokio::time::sleep(BULK_HOLD).await;
            backstop.release();
        });
        BulkHold { inner }
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
fn url_path(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(url, |at| &url[at + 3..]);
    let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let path = &after_scheme[path_start..];
    path.split(['?', '#']).next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn logs_only_the_path() {
        assert_eq!(
            url_path("https://lichess.org/api/user/Alice?token=x"),
            "/api/user/Alice"
        );
    }
}
