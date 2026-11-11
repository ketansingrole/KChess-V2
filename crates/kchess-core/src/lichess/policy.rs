//! Lichess request admission and accounting.
//!
//! Admission: at most two requests are in flight, interactive requests (game moves and their
//! resign/abort/draw/takeback family) go before the rest, a 429 starts a cooldown that the next
//! request waits out (`Retry-After`, capped at 5 minutes, 60 seconds when absent) without holding
//! a lane, and that interactive requests wait out only when one of them was refused, a request
//! that is not a stream has its deadline to answer, and game exports hold a lane of their own
//! until their body is read, cancelled or fails. Nothing here retries: an ambiguous mutation is
//! never replayed. The caller says which [`Lane`] a request belongs to.
//!
//! Accounting: each request and the bytes of its body are counted against the account and kind
//! of its [`Usage`], through its [`Meter`]. Traffic from an account whose epoch moved
//! on (logout, removal, clearing) is not counted.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Stream, TryStreamExt};
use kchess_contracts::types::UsageKind;
use reqwest::header::HeaderMap;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, ErrorKind, Result};
use crate::host::{Host, Level};
use crate::usage::{Meter, Unmetered};

/// A request may take this long to answer (its headers, and its body unless it is a stream).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ACTIVE: usize = 2;
/// The position lookups run side by side: the explorer answers each in a moment.
const MAX_LOOKUPS: usize = 16;
const MAX_WAITING: usize = 500;
/// A backstop for an export body dropped without being read or cancelled.
const BULK_HOLD: Duration = Duration::from_secs(600);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
/// The shortest cooldown, in seconds: `Retry-After: 0` must not let a refused burst repeat at once.
const MIN_COOLDOWN: f64 = 1.0;

/// Which kind of request this is: how it is admitted and how long its answer may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// A game action (move, resign, abort, takeback, draw, claim, berserk): it goes before the
    /// rest, and waits out only a cooldown an action of its kind caused.
    Interactive,
    /// An ordinary request, answered within the deadline.
    Background,
    /// A game export: one at a time, its body read without a deadline.
    Export,
    /// A long body read as it arrives, without a deadline.
    Feed,
    /// A live stream (events, games, seeks, TV feeds): no deadline, and an uncompressed body,
    /// since a compressor on the way would buffer the lines until it had a block.
    Live,
}

impl Lane {
    fn interactive(self) -> bool {
        self == Lane::Interactive
    }

    /// Whether the body is read without the request deadline.
    pub(crate) fn streamed(self) -> bool {
        matches!(self, Lane::Export | Lane::Feed | Lane::Live)
    }
}

/// Who a request is counted against: the account (lowercase), the kind, and the account's epoch
/// when the work started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub account: String,
    pub kind: UsageKind,
    pub epoch: u64,
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
    /// How many requests run at once.
    max_active: usize,
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
    while lanes.active < lanes.max_active && !lanes.waiting.is_empty() {
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
    meter: Arc<dyn Meter>,
    http: reqwest::Client,
    lifetime: CancellationToken,
    timeout: Duration,
    lanes: Mutex<Lanes>,
    /// When (by `now`, in milliseconds) a 429 answered to a background request (a sync, an
    /// export) ends: background requests wait until then.
    cooldown_until: Mutex<Option<i64>>,
    /// When a 429 answered to an interactive request ends: every request waits until then.
    interactive_cooldown_until: Mutex<Option<i64>>,
    /// The position lookups' policy: a request during a cooldown is refused instead of waiting
    /// it out (a saved answer is shown), they run side by side, and they are not metered.
    lookups: bool,
    now: Clock,
    accounting: Mutex<Accounting>,
}

/// A clock in milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Milliseconds of a monotonic clock.
fn monotonic_ms() -> i64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    i64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

use crate::tasks::lock;

/// A request Lichess did not answer within its deadline.
pub(crate) fn timed_out() -> CoreError {
    CoreError::new("Lichess didn’t answer in time.").with_kind(ErrorKind::TimedOut)
}

use crate::tasks::aborted;

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

    /// `host` receives the warnings and `meter` the traffic; `lifetime` ends the accounting when
    /// the core closes.
    pub fn new(
        host: Arc<dyn Host>,
        meter: Arc<dyn Meter>,
        http: reqwest::Client,
        lifetime: CancellationToken,
    ) -> Arc<Policy> {
        Policy::with_timeout(host, meter, http, lifetime, REQUEST_TIMEOUT)
    }

    /// As `new`, with another request deadline (tests use a short one).
    pub fn with_timeout(
        host: Arc<dyn Host>,
        meter: Arc<dyn Meter>,
        http: reqwest::Client,
        lifetime: CancellationToken,
        timeout: Duration,
    ) -> Arc<Policy> {
        Policy::build(
            host,
            meter,
            http,
            lifetime,
            timeout,
            false,
            Arc::new(monotonic_ms),
        )
    }

    /// The policy of one lookup origin (the explorer, the tablebase): a request during a
    /// cooldown fails at once with how long is left, by `now`.
    pub fn for_lookups(
        host: Arc<dyn Host>,
        http: reqwest::Client,
        lifetime: CancellationToken,
        timeout: Duration,
        now: Clock,
    ) -> Arc<Policy> {
        // A lookup counts no traffic (`record`).
        Policy::build(
            host,
            Arc::new(Unmetered),
            http,
            lifetime,
            timeout,
            true,
            now,
        )
    }

    fn build(
        host: Arc<dyn Host>,
        meter: Arc<dyn Meter>,
        http: reqwest::Client,
        lifetime: CancellationToken,
        timeout: Duration,
        lookups: bool,
        now: Clock,
    ) -> Arc<Policy> {
        Arc::new(Policy {
            host,
            meter,
            http,
            lifetime,
            timeout,
            lanes: Mutex::new(Lanes {
                max_active: if lookups { MAX_LOOKUPS } else { MAX_ACTIVE },
                ..Lanes::default()
            }),
            cooldown_until: Mutex::new(None),
            interactive_cooldown_until: Mutex::new(None),
            lookups,
            now,
            accounting: Mutex::new(Accounting::default()),
        })
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Ends with the core: requests are cancelled by it unless given another token.
    pub fn lifetime(&self) -> &CancellationToken {
        &self.lifetime
    }

    /// The epoch of `account` (lowercase). Usage started under an older epoch is dropped.
    pub fn epoch(&self, account: &str) -> u64 {
        lock(&self.accounting)
            .epochs
            .get(&account.to_lowercase())
            .copied()
            .unwrap_or(0)
    }

    /// The usage of work for `account` and `kind` starting now.
    pub fn usage(&self, account: &str, kind: UsageKind) -> Usage {
        let account = account.to_lowercase();
        Usage {
            epoch: self.epoch(&account),
            account,
            kind,
        }
    }

    fn record(&self, usage: &Usage, requests: u64, bytes: u64) {
        if self.lookups || self.lifetime.is_cancelled() || (requests == 0 && bytes == 0) {
            return;
        }
        if usage.epoch != self.epoch(&usage.account) {
            return;
        }
        self.meter
            .count(&usage.account, usage.kind, requests, bytes);
    }

    /// Traffic still arriving for these accounts is no longer counted.
    pub fn forget_usage(&self, accounts: &[String]) {
        let mut accounting = lock(&self.accounting);
        for name in accounts {
            *accounting.epochs.entry(name.to_lowercase()).or_insert(0) += 1;
        }
    }

    /// The body of a response, passed through with its bytes counted against `usage`.
    pub fn metered<S, B, E>(
        self: &Arc<Self>,
        body: S,
        usage: &Usage,
    ) -> impl Stream<Item = std::result::Result<B, E>>
    where
        S: Stream<Item = std::result::Result<B, E>>,
        B: AsRef<[u8]>,
    {
        let policy = Arc::clone(self);
        let usage = usage.clone();
        body.inspect_ok(move |chunk| policy.record(&usage, 0, chunk.as_ref().len() as u64))
    }

    /// Admit `request` in `lane` under the lanes, the cooldown and the deadline, and count it
    /// against `usage`. `cancel` ends the wait, the request and (for a stream) the body hand-off,
    /// with an abort.
    pub async fn send(
        self: &Arc<Self>,
        request: reqwest::Request,
        lane: Lane,
        usage: &Usage,
        cancel: &CancellationToken,
    ) -> Result<Admitted> {
        if cancel.is_cancelled() {
            return Err(aborted());
        }
        let hold = if lane == Lane::Export {
            Some(self.acquire_bulk(cancel).await?)
        } else {
            None
        };
        let response = match self.admitted(request, lane, cancel).await {
            Ok(response) => response,
            Err(cause) => {
                drop(hold);
                return Err(cause);
            }
        };
        self.record(usage, 1, 0);
        if !response.status().is_success() {
            // A failed export does not keep the lane.
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
        lane: Lane,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response> {
        let interactive = lane.interactive();
        if self.lookups
            && let Some(left) = self.cooldown_left(interactive)
        {
            return Err(rate_limited(left));
        }
        let url = request.url().to_string();
        let _slot = self.admit(interactive, cancel).await?;
        if lane == Lane::Live {
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
            *lock(until) = Some((self.now)().saturating_add(millis(cooldown)));
            if self.lookups {
                return Err(rate_limited(cooldown));
            }
            let retry = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .to_string();
            let path: String = url_path(&url).chars().take(80).collect();
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
        let mut until = *lock(&self.interactive_cooldown_until);
        if !interactive {
            until = until.max(*lock(&self.cooldown_until));
        }
        let left = until? - (self.now)();
        (left > 0).then(|| Duration::from_millis(left.unsigned_abs()))
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

/// A lookup refused while its origin's cooldown runs: the message says how long is left.
fn rate_limited(left: Duration) -> CoreError {
    let seconds = left.as_millis().div_ceil(1000).max(1);
    let unit = if seconds == 1 { "second" } else { "seconds" };
    CoreError::new(format!("Too many lookups. Try again in {seconds} {unit}."))
        .with_kind(ErrorKind::RateLimited)
}

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

    use crate::test_host::TestHost;

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
        *lock(&policy.cooldown_until) = Some(monotonic_ms() + 60_000);
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
        *lock(&policy.interactive_cooldown_until) = Some(monotonic_ms() + 60_000);
        let mut moved = Box::pin(policy.admit(true, &cancel));
        queue(&mut moved).await;
        cancel.cancel();
        assert!(sync.await.is_err() && other.await.is_err() && moved.await.is_err());
    }

    fn quiet_policy() -> Arc<Policy> {
        Policy::new(
            TestHost::quiet(),
            Arc::new(Unmetered),
            reqwest::Client::new(),
            CancellationToken::new(),
        )
    }

    /// Polls `future` once, so it queues, without waiting for it.
    async fn queue<F: std::future::Future + Unpin>(future: &mut F) {
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
