//! Deterministic local fixtures for the Lichess tests: a scripted HTTP/1.1 server on 127.0.0.1
//! (every response closes its connection once the client hangs up; bodies can stream in chunks
//! or wait on a gate), a host
//! that records logs and events, and the in-memory store and token source the account code uses.

use kchess_contracts::{CoreEvent, HostAsk, HostReply, HostRequest};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use kchess_core::capabilities::Capabilities;
use kchess_core::error::Result as CoreResult;
use kchess_core::host::{Host, Level};
use kchess_core::lichess::accounts::{AccountRow, CachedValue, LichessGame, LichessStore};
use kchess_core::lichess::client::TokenSource;
use kchess_core::loopback::close_after_client;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

/// One request the fixture received.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn query(&self, name: &str) -> Option<String> {
        let query = self.target.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| value.to_string())
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// How the fixture answers one request.
pub enum Reply {
    /// Status, JSON body.
    Json(u16, String),
    /// Status, content type, body.
    Text(u16, &'static str, String),
    /// Status, extra headers, body.
    WithHeaders(u16, Vec<(&'static str, String)>, String),
    /// Status, extra headers, a body of raw bytes (a compressed answer).
    Bytes(u16, Vec<(&'static str, String)>, Vec<u8>),
    /// 200 NDJSON, sent chunk by chunk as the sender provides them; the body ends when the sender
    /// is dropped.
    Stream(mpsc::UnboundedReceiver<String>),
    /// A chunked response with a specified status and content type.
    StreamText(u16, &'static str, mpsc::UnboundedReceiver<String>),
    /// Waits until the gate opens, then answers with the inner reply.
    Gated(watch::Receiver<bool>, Box<Reply>),
    /// Never answers.
    Hang,
}

type Handler = Box<dyn FnMut(&Seen) -> Reply + Send>;

/// How long a test waits for a request before it fails: a ceiling for a request that never
/// comes, not a pause, so it can be generous. A loaded Windows runner filtering 127.0.0.1 took
/// over 5 s to deliver one (`explorer_assistance.rs`).
pub const REQUEST_WAIT: Duration = Duration::from_secs(30);

/// A running fixture: its origin, and every request it has received, in arrival order.
pub struct Fixture {
    pub base: String,
    /// Every request so far; a waiter wakes on each one.
    seen: Arc<watch::Sender<Vec<Seen>>>,
}

impl Fixture {
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.borrow().clone()
    }

    /// Waits until the requests so far satisfy `done`, and returns them. Fails the test after
    /// `REQUEST_WAIT`, naming `what` and the requests that did come, so a test never carries on
    /// in a state it did not set up.
    pub async fn wait_for(&self, what: &str, done: impl FnMut(&[Seen]) -> bool) -> Vec<Seen> {
        self.wait_for_within(REQUEST_WAIT, what, done).await
    }

    /// `wait_for` with its own limit.
    pub async fn wait_for_within(
        &self,
        limit: Duration,
        what: &str,
        mut done: impl FnMut(&[Seen]) -> bool,
    ) -> Vec<Seen> {
        let mut seen = self.seen.subscribe();
        match tokio::time::timeout(limit, seen.wait_for(|requests| done(requests))).await {
            Ok(Ok(requests)) => requests.clone(),
            _ => {
                let paths: Vec<String> = self.requests().iter().map(|r| r.target.clone()).collect();
                panic!("timed out after {limit:?} waiting for {what}; requested: {paths:?}")
            }
        }
    }

    /// Waits until `path` has been requested `count` times, and returns those requests; fails the
    /// test if it never is (see `wait_for`).
    pub async fn requested(&self, path: &str, count: usize) -> Vec<Seen> {
        let to = |requests: &[Seen]| -> Vec<Seen> {
            requests
                .iter()
                .filter(|seen| seen.path() == path)
                .cloned()
                .collect()
        };
        let what = format!("{count} request(s) to {path}");
        to(&self
            .wait_for(&what, |requests| to(requests).len() >= count)
            .await)
    }

    /// The requests whose path is `path`.
    pub fn to(&self, path: &str) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|seen| seen.path() == path)
            .collect()
    }
}

/// Serve `handler` on a free loopback port until the test ends.
pub async fn serve<F>(handler: F) -> Fixture
where
    F: FnMut(&Seen) -> Reply + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let seen = Arc::new(watch::Sender::new(Vec::new()));
    let handler: Arc<Mutex<Handler>> = Arc::new(Mutex::new(Box::new(handler)));
    let seen_for_task = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let handler = Arc::clone(&handler);
            let seen = Arc::clone(&seen_for_task);
            tokio::spawn(async move {
                let _ = handle(socket, handler, seen).await;
            });
        }
    });
    Fixture { base, seen }
}

async fn read_request(socket: &mut TcpStream) -> std::io::Result<Option<Seen>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(Some(Seen {
        method,
        target,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    }))
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

async fn handle(
    mut socket: TcpStream,
    handler: Arc<Mutex<Handler>>,
    seen: Arc<watch::Sender<Vec<Seen>>>,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut socket).await? else {
        return Ok(());
    };
    seen.send_modify(|requests| requests.push(request.clone()));
    let mut reply = (handler.lock().unwrap())(&request);
    loop {
        match reply {
            Reply::Gated(mut gate, inner) => {
                while !*gate.borrow() {
                    if gate.changed().await.is_err() {
                        break;
                    }
                }
                reply = *inner;
            }
            Reply::Json(status, body) => {
                return write_sized(
                    &mut socket,
                    status,
                    &[("Content-Type", "application/json".into())],
                    body.as_bytes(),
                )
                .await;
            }
            Reply::Text(status, content_type, body) => {
                return write_sized(
                    &mut socket,
                    status,
                    &[("Content-Type", content_type.into())],
                    body.as_bytes(),
                )
                .await;
            }
            Reply::WithHeaders(status, extra, body) => {
                let extra: Vec<(&str, String)> = extra;
                return write_sized(&mut socket, status, &extra, body.as_bytes()).await;
            }
            Reply::Bytes(status, extra, body) => {
                return write_sized(&mut socket, status, &extra, &body).await;
            }
            Reply::Stream(chunks) => {
                reply = Reply::StreamText(200, "application/x-ndjson", chunks);
            }
            Reply::StreamText(status, content_type, mut chunks) => {
                let head = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                    status_text(status)
                );
                socket.write_all(head.as_bytes()).await?;
                while let Some(piece) = chunks.recv().await {
                    let frame = format!("{:x}\r\n{}\r\n", piece.len(), piece);
                    if socket.write_all(frame.as_bytes()).await.is_err() {
                        return Ok(());
                    }
                }
                socket.write_all(b"0\r\n\r\n").await?;
                close_after_client(&mut socket).await;
                return Ok(());
            }
            Reply::Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                return Ok(());
            }
        }
    }
}

async fn write_sized(
    socket: &mut TcpStream,
    status: u16,
    extra: &[(&str, String)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", status_text(status));
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await?;
    close_after_client(socket).await;
    Ok(())
}

/// The NDJSON body of a list of game records (one per line).
pub fn ndjson(lines: &[Value]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Records every log line and event; answers the core's `host:request`s from `answers`.
/// Plays the browser for a sign-in: called with the URL the core opens.
pub type BrowserFn = Box<dyn Fn(String) + Send + Sync>;

#[derive(Default)]
pub struct Recording {
    pub logs: Mutex<Vec<(Level, String, String)>>,
    pub events: Mutex<Vec<(String, Value)>>,
    /// The traffic counted, as `{ account, category, requests, bytes }`.
    pub usage: Mutex<Vec<Value>>,
    /// Set once the capabilities exist, so the host can answer them.
    pub capabilities: Mutex<Option<Arc<Capabilities>>>,
    /// Replies to host requests by kind; anything missing gets an error.
    pub answers: Mutex<HashMap<String, Value>>,
    /// The URL a browser sign-in was asked to open.
    pub opened: Mutex<Vec<String>>,
    /// Called instead of answering when a sign-in page is opened (to play the browser).
    pub browser: Mutex<Option<BrowserFn>>,
}

impl Recording {
    pub fn logged(&self) -> String {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, scope, message)| format!("[{scope}] {message}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn usage_events(&self) -> Vec<Value> {
        self.usage.lock().unwrap().clone()
    }
}

impl kchess_core::usage::Meter for Recording {
    fn count(
        &self,
        account: &str,
        kind: kchess_contracts::types::UsageKind,
        requests: u64,
        bytes: u64,
    ) {
        self.usage.lock().unwrap().push(serde_json::json!({
            "account": account,
            "category": kind,
            "requests": requests,
            "bytes": bytes,
        }));
    }
}

impl Host for Recording {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push((level, scope.to_string(), message.to_string()));
    }

    fn emit(&self, event: CoreEvent) {
        let (event, payload) = (event.name(), event.into_payload());
        self.events
            .lock()
            .unwrap()
            .push((event.to_string(), payload.clone()));
        if event != "host:request" {
            return;
        }
        let Ok(ask) = serde_json::from_value::<HostAsk>(payload) else {
            return;
        };
        let capabilities = self.capabilities.lock().unwrap().clone();
        let answer = match ask.request {
            HostRequest::OpenExternal(url) => {
                self.opened.lock().unwrap().push(url.clone());
                if let Some(browser) = self.browser.lock().unwrap().as_ref() {
                    browser(url);
                }
                HostReply::done()
            }
            HostRequest::SecretsEncrypt(plain) => {
                HostReply::Value(Value::from(format!("enc:{plain}")))
            }
            request => match self.answers.lock().unwrap().get(request.kind()) {
                Some(value) => HostReply::Value(value.clone()),
                None => HostReply::Error(format!("No answer for {}.", request.kind())),
            },
        };
        if let Some(capabilities) = capabilities {
            tokio::spawn(async move {
                capabilities.reply(ask.id, answer);
            });
        }
    }
}

/// The default host answers: a working credential store and focus, and no browser.
pub fn default_answers() -> HashMap<String, Value> {
    let mut answers = HashMap::new();
    answers.insert("secrets.available".to_string(), Value::Bool(true));
    answers.insert("focus".to_string(), Value::Null);
    answers
}

/// Builds the capabilities of a recording host, and links the two.
pub fn capabilities_for(host: &Arc<Recording>) -> Arc<Capabilities> {
    let capabilities = Arc::new(Capabilities::new(Arc::clone(host) as Arc<dyn Host>));
    *host.capabilities.lock().unwrap() = Some(Arc::clone(&capabilities));
    capabilities
}

/// The HTTP client every test uses (no proxy, so the fixtures are reached directly).
pub fn http_client() -> reqwest::Client {
    kchess_core::lichess::client::http_client_builder()
        .no_proxy()
        .build()
        .expect("http client")
}

/// An in-memory `LichessStore`: the account rows, the games, the cursor and the API cache.
#[derive(Default)]
pub struct MemoryStore {
    pub cache_write_gate: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
    pub accounts: Mutex<Vec<AccountRow>>,
    pub games: Mutex<Vec<LichessGame>>,
    pub cursors: Mutex<HashMap<String, i64>>,
    pub pending: Mutex<HashMap<String, Vec<String>>>,
    pub cache: Mutex<HashMap<String, CachedValue>>,
    pub logins: Mutex<Vec<(String, String)>>,
    pub dismissed: Mutex<Vec<String>>,
    /// Reviews saved with their pages of games, or on their own (`save_reviews`).
    pub reviews: Mutex<Vec<Value>>,
    /// Games Lichess was asked about (`mark_checked`).
    pub checked: Mutex<Vec<String>>,
}

impl MemoryStore {
    pub fn with_accounts(rows: &[(&str, bool)]) -> Arc<MemoryStore> {
        let store = MemoryStore::default();
        *store.accounts.lock().unwrap() = rows
            .iter()
            .map(|(name, connected)| AccountRow {
                username: name.to_string(),
                connected: *connected,
                last_synced_at: None,
            })
            .collect();
        Arc::new(store)
    }
}

impl LichessStore for MemoryStore {
    fn accounts<'a>(&'a self) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<AccountRow>>> {
        Box::pin(async move { Ok(self.accounts.lock().unwrap().clone()) })
    }

    fn dismissed_friends<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move { Ok(self.dismissed.lock().unwrap().clone()) })
    }

    fn pending_game_ids<'a>(
        &'a self,
        account: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move {
            Ok(self
                .pending
                .lock()
                .unwrap()
                .get(&account.to_lowercase())
                .cloned()
                .unwrap_or_default())
        })
    }

    fn save_games_page<'a>(
        &'a self,
        account: &'a str,
        games: &'a [LichessGame],
        reviews: &'a [Value],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            if !current() {
                return Err(kchess_core::error::CoreError::new("Sync was cancelled."));
            }
            let mut stored = self.games.lock().unwrap();
            for game in games {
                if !stored
                    .iter()
                    .any(|known| known.id == game.id && known.account == account)
                {
                    stored.push(game.clone());
                }
            }
            self.reviews.lock().unwrap().extend(reviews.iter().cloned());
            Ok(())
        })
    }

    fn save_reviews<'a>(
        &'a self,
        _account: &'a str,
        reviews: &'a [Value],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<Value>>> {
        Box::pin(async move {
            if !current() {
                return Err(kchess_core::error::CoreError::new("Sync was cancelled."));
            }
            self.reviews.lock().unwrap().extend(reviews.iter().cloned());
            Ok(reviews.to_vec())
        })
    }

    fn mark_checked<'a>(
        &'a self,
        ids: &'a [String],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            if !current() {
                return Err(kchess_core::error::CoreError::aborted(
                    "Sync was cancelled.",
                ));
            }
            self.checked.lock().unwrap().extend(ids.iter().cloned());
            Ok(())
        })
    }

    fn save_games_cursor<'a>(
        &'a self,
        account: &'a str,
        synced_at: i64,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            if !current() {
                return Err(kchess_core::error::CoreError::new("Sync was cancelled."));
            }
            self.cursors
                .lock()
                .unwrap()
                .insert(account.to_lowercase(), synced_at);
            if let Some(row) = self
                .accounts
                .lock()
                .unwrap()
                .iter_mut()
                .find(|row| row.username.eq_ignore_ascii_case(account))
            {
                row.last_synced_at = Some(synced_at);
            }
            Ok(())
        })
    }

    fn read_api_cache<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Option<CachedValue>>> {
        Box::pin(async move { Ok(self.cache.lock().unwrap().get(key).cloned()) })
    }

    fn write_api_cache<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let gate = self.cache_write_gate.lock().unwrap().take();
            if let Some((started, release)) = gate {
                let _ = started.send(());
                release.await.expect("release cache write");
            }

            if !current() {
                return Err(kchess_core::error::CoreError::new(
                    "Account was logged out.",
                ));
            }
            self.cache.lock().unwrap().insert(
                key.to_string(),
                CachedValue {
                    value: value.clone(),
                    fetched_at: 1,
                },
            );
            Ok(())
        })
    }

    fn save_login<'a>(
        &'a self,
        username: &'a str,
        ciphertext: &'a str,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            if !current() {
                return Err(kchess_core::error::CoreError::aborted(
                    "Login was cancelled by logout.",
                ));
            }
            self.logins
                .lock()
                .unwrap()
                .push((username.to_string(), ciphertext.to_string()));
            let mut accounts = self.accounts.lock().unwrap();
            accounts.retain(|row| !row.username.eq_ignore_ascii_case(username));
            accounts.push(AccountRow {
                username: username.to_string(),
                connected: true,
                last_synced_at: None,
            });
            Ok(())
        })
    }
}

/// Tokens by account, as `getToken` would read them (already decrypted).
#[derive(Default)]
pub struct MemoryTokens {
    pub tokens: Mutex<HashMap<String, String>>,
}

impl MemoryTokens {
    pub fn with(tokens: &[(&str, &str)]) -> Arc<MemoryTokens> {
        let source = MemoryTokens::default();
        for (name, token) in tokens {
            source
                .tokens
                .lock()
                .unwrap()
                .insert(name.to_lowercase(), token.to_string());
        }
        Arc::new(source)
    }
}

impl TokenSource for MemoryTokens {
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, CoreResult<Option<String>>> {
        Box::pin(async move {
            Ok(self
                .tokens
                .lock()
                .unwrap()
                .get(&account.to_lowercase())
                .cloned())
        })
    }
}

/// A game record as Lichess sends it, for the sync fixtures.
pub fn game_json(id: &str, created_at: i64, white: &str, black: &str) -> Value {
    serde_json::json!({
        "id": id,
        "createdAt": created_at,
        "lastMoveAt": created_at + 1000,
        "rated": true,
        "speed": "blitz",
        "perf": "blitz",
        "status": "mate",
        "winner": "white",
        "players": {
            "white": { "user": { "name": white }, "rating": 1500, "ratingDiff": 8 },
            "black": { "user": { "name": black }, "rating": 1400, "ratingDiff": -8 }
        },
        "opening": { "name": "Sicilian" },
        "moves": "e4 c5"
    })
}

/// A `Lichess` service over `base` with the given storage and tokens, for the watch, lookup, study,
/// puzzle and review tests.
pub fn lichess_service(
    base: &str,
    host: &Arc<Recording>,
    store: Arc<dyn LichessStore>,
    tokens: Arc<dyn TokenSource>,
) -> Arc<kchess_core::lichess::accounts::Lichess> {
    lichess_service_with_idle(
        base,
        host,
        store,
        tokens,
        kchess_core::lichess::stream::IDLE,
    )
}

/// `lichess_service` whose streams count as stalled after `idle` of silence.
pub fn lichess_service_with_idle(
    base: &str,
    host: &Arc<Recording>,
    store: Arc<dyn LichessStore>,
    tokens: Arc<dyn TokenSource>,
    idle: Duration,
) -> Arc<kchess_core::lichess::accounts::Lichess> {
    use kchess_core::lichess::client::LichessClient;
    use kchess_core::lichess::policy::Policy;
    let policy = Policy::with_timeout(
        Arc::clone(host) as Arc<dyn Host>,
        Arc::clone(host) as Arc<dyn kchess_core::usage::Meter>,
        http_client(),
        tokio_util::sync::CancellationToken::new(),
        Duration::from_secs(5),
    );
    let client = LichessClient::with_base(base, policy).with_stream_idle(idle);
    Arc::new(kchess_core::lichess::accounts::Lichess::new(
        client,
        capabilities_for(host),
        store,
        tokens,
        Arc::clone(host) as Arc<dyn Host>,
    ))
}

/// The fields of an `application/x-www-form-urlencoded` body, decoded.
pub fn form_fields(body: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://fixture/?{body}"))
        .map(|url| {
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// The value of one decoded form field.
pub fn form_field(body: &str, name: &str) -> Option<String> {
    form_fields(body)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}
