//! The Lichess HTTP client: every request to Lichess (and to its explorer and tablebase) is a
//! [`Request`] sent through one [`Policy`], as some [`Auth`], in some [`Lane`].
//!
//! Failed responses become [`CoreError`]s with a kind, worded by [`LichessError::message`], so a
//! website's 404 page never reaches the UI; the technical form goes to the log.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use kchess_contracts::types::UsageKind;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use super::policy::Lane;
use super::policy::{Admitted, Policy, Usage};
use super::stream::{LineOptions, read_chunks, read_lines};
use kchess_contracts::error::{CoreError, ErrorKind, Result};

/// The Lichess website and API.
pub const BASE_URL: &str = "https://lichess.org";

/// Shared transport configuration. Streams have no total response deadline; the policy bounds
/// headers, ordinary calls bound their bodies, line-stream readers bound silence between chunks,
/// and chunk feeds wait until the body ends or is cancelled.
/// Every request names the app: Lichess answers one that names no program with a 403 page.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(super::policy::REQUEST_TIMEOUT)
}

/// A limit checked as decoded text arrives, before it is appended to the collected response.
#[derive(Clone, Copy)]
pub struct TextLimit<'a> {
    pub utf16_units: usize,
    pub message: &'a str,
}

/// A failed Lichess answer: its status, endpoint and the start of its body. Callers see it as a
/// [`CoreError`] whose kind says what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LichessError {
    pub status: u16,
    pub endpoint: String,
    pub detail: String,
}

impl LichessError {
    /// What kind of failure Lichess's answer is.
    pub fn kind(&self) -> ErrorKind {
        match self.status {
            401 => ErrorKind::Login,
            403 if self.mentions_login() => ErrorKind::Login,
            400 | 403 | 409 | 422 => ErrorKind::Refused,
            404 | 410 => ErrorKind::NotFound,
            429 => ErrorKind::RateLimited,
            _ => ErrorKind::Unavailable,
        }
    }

    /// What a person is told: Lichess's own reason when it gave one in words, otherwise what
    /// the kind means. Never the status, the endpoint or the raw body (`technical` has those).
    pub fn message(&self) -> String {
        let reason = self.reason();
        match self.kind() {
            ErrorKind::Login => {
                "Lichess no longer accepts this account’s login. Reconnect it.".into()
            }
            ErrorKind::Refused => reason.unwrap_or_else(|| "Lichess refused this.".into()),
            ErrorKind::NotFound => reason
                .filter(|reason| {
                    !reason
                        .trim_end_matches('.')
                        .eq_ignore_ascii_case("not found")
                })
                .unwrap_or_else(|| "Lichess has no such thing.".into()),
            ErrorKind::RateLimited => {
                "Lichess asked KChess to slow down. Try again in a minute.".into()
            }
            _ => "Lichess isn’t working right now. Try again shortly.".into(),
        }
    }

    /// For the log, never for a person.
    pub fn technical(&self) -> String {
        if self.detail.is_empty() {
            format!("Lichess {} from {}", self.status, self.endpoint)
        } else {
            format!(
                "Lichess {} from {}: {}",
                self.status, self.endpoint, self.detail
            )
        }
    }

    /// Lichess's reason in its own words: the `error` (or `message`) text of a JSON body, as a
    /// sentence. A body that is not one plain sentence gives none.
    fn reason(&self) -> Option<String> {
        let parsed: Option<Value> = serde_json::from_str(&self.detail).ok();
        let text = match &parsed {
            Some(Value::Object(fields)) => fields
                .get("error")
                .or_else(|| fields.get("message"))
                .and_then(Value::as_str)?
                .to_string(),
            Some(Value::String(text)) => text.clone(),
            Some(_) => return None,
            None => self.detail.clone(),
        };
        let text = text.trim();
        let plain =
            !text.is_empty() && text.chars().count() <= 160 && !text.contains(['{', '<', '[']);
        plain.then(|| {
            let mut chars = text.chars();
            let mut sentence: String = chars
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .collect();
            sentence.push_str(chars.as_str());
            if !sentence.ends_with(['.', '!', '?']) {
                sentence.push('.');
            }
            sentence
        })
    }

    /// A 403 about the token rather than the request: a missing scope or a revoked login.
    fn mentions_login(&self) -> bool {
        let detail = self.detail.to_ascii_lowercase();
        ["scope", "token", "oauth", "permission"]
            .iter()
            .any(|word| detail.contains(word))
    }
}

impl From<LichessError> for CoreError {
    fn from(error: LichessError) -> CoreError {
        CoreError::new(error.message()).with_kind(error.kind())
    }
}

/// The error of a failed response: the domain rule chooses the detail
/// (`kchess_domain::records::lichess`). `body` is the parsed JSON body, the raw text when the
/// body is not JSON, or None when it is empty.
pub fn lichess_error(status: u16, body: Option<Value>, endpoint: &str) -> LichessError {
    LichessError {
        status,
        endpoint: endpoint.to_string(),
        detail: kchess_domain::records::lichess::error_detail(
            Some(f64::from(status)),
            body.as_ref(),
        ),
    }
}

/// `application/x-www-form-urlencoded` text.
pub fn urlencoded<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> String {
    let Ok(mut url) = reqwest::Url::parse("http://localhost/") else {
        return String::new();
    };
    url.query_pairs_mut().extend_pairs(
        pairs
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref())),
    );
    url.query().unwrap_or("").to_string()
}

/// The stored login of an account. `Ok(None)` when the account has no login.
pub trait TokenSource: Send + Sync {
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, Result<Option<String>>>;
}

/// Who a request is made as: the login it carries (if any), and the account and app area its
/// traffic is counted against.
#[derive(Clone, PartialEq, Eq)]
pub struct Auth {
    usage: Usage,
    token: Option<String>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("usage", &self.usage)
            .field("signed_in", &self.token.is_some())
            .finish()
    }
}

impl Auth {
    /// The same caller with `token`'s login.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Auth {
        self.token = Some(token.into());
        self
    }

    /// The same caller without a login (counted against the same account).
    #[must_use]
    pub fn without_login(&self) -> Auth {
        Auth {
            usage: self.usage.clone(),
            token: None,
        }
    }

    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    /// The account the traffic is counted against (lowercase; empty when nobody's).
    pub fn account(&self) -> &str {
        &self.usage.account
    }
}

/// A request's body.
#[derive(Clone, Debug, Default)]
enum Body {
    #[default]
    Empty,
    Form(String),
    Text(String),
    Json(Vec<u8>),
}

/// One request to Lichess, built by [`LichessClient::get`] and its siblings and sent by one of
/// its readers. It is cancelled with the core unless given another token.
#[must_use]
pub struct Request<'c> {
    client: &'c LichessClient,
    method: reqwest::Method,
    path: String,
    query: Vec<(String, String)>,
    body: Body,
    accept: Option<&'static str>,
    auth: Auth,
    lane: Lane,
    cancel: CancellationToken,
    max_bytes: Option<(usize, &'static str)>,
}

impl<'c> Request<'c> {
    pub fn query<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> Self {
        self.query.extend(
            pairs
                .iter()
                .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string())),
        );
        self
    }

    /// A form body. Lichess is asked for JSON: without that, some endpoints (`/api/import`)
    /// answer with a redirect to a web page.
    pub fn form<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> Self {
        self.body = Body::Form(urlencoded(pairs));
        self.accept.get_or_insert("application/json");
        self
    }

    /// A plain-text body, such as a comma-separated list of names or ids.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.body = Body::Text(text.into());
        self
    }

    /// A JSON body; the answer is asked for as JSON too.
    pub fn json(mut self, body: &Value) -> Self {
        self.body = Body::Json(serde_json::to_vec(body).unwrap_or_default());
        self.accept.get_or_insert("application/json");
        self
    }

    pub fn accept(mut self, accept: &'static str) -> Self {
        self.accept = Some(accept);
        self
    }

    pub fn lane(mut self, lane: Lane) -> Self {
        self.lane = lane;
        self
    }

    pub fn cancel(mut self, cancel: &CancellationToken) -> Self {
        self.cancel = cancel.clone();
        self
    }

    /// The most an answer read whole may hold, and what is said when it holds more.
    pub fn max_bytes(mut self, bytes: usize, message: &'static str) -> Self {
        self.max_bytes = Some((bytes, message));
        self
    }

    fn endpoint(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    fn url(&self) -> Result<reqwest::Url> {
        let mut url = self.client.url(&self.path)?;
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(
                self.query
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            );
        }
        Ok(url)
    }

    /// Builds and admits the request.
    async fn admit(&mut self) -> Result<Admitted> {
        let policy = &self.client.policy;
        let mut builder = policy.http().request(self.method.clone(), self.url()?);
        if let Some(accept) = self.accept {
            builder = builder.header(reqwest::header::ACCEPT, accept);
        }
        if let Some(token) = &self.auth.token {
            builder = builder.bearer_auth(token);
        }
        builder = match std::mem::take(&mut self.body) {
            Body::Empty => builder,
            Body::Form(text) => builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(text),
            Body::Text(text) => builder
                .header(reqwest::header::CONTENT_TYPE, "text/plain")
                .body(text),
            Body::Json(bytes) => builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes),
        };
        let request = builder
            .build()
            .map_err(|cause| CoreError::new(format!("Invalid Lichess request: {cause}")))?;
        policy
            .send(request, self.lane, &self.auth.usage, &self.cancel)
            .await
    }

    /// Lichess's failed answer: its technical form goes to the log, the person sees its message.
    fn failed(&self, error: LichessError) -> CoreError {
        self.client.policy.log_warning(&error.technical());
        error.into()
    }

    /// A failed answer's error, its body read within the deadline.
    async fn refused(&self, response: reqwest::Response) -> CoreError {
        let status = response.status().as_u16();
        match self.body_text(response, None, false).await {
            Ok(text) => self.failed(lichess_error(status, body_value(&text), &self.endpoint())),
            Err(error) => error,
        }
    }

    /// The body as text: within the request deadline, unless it is `streamed` (a stream bounds
    /// its own silence, or waits until the body ends).
    async fn body_text(
        &self,
        response: reqwest::Response,
        limit: Option<TextLimit<'_>>,
        streamed: bool,
    ) -> Result<String> {
        let max = self.max_bytes.unwrap_or((MAX_BODY, LARGE_BODY));
        let read = read_body(response, &self.cancel, max, limit);
        if streamed {
            return read.await;
        }
        match tokio::time::timeout(self.client.policy.timeout(), read).await {
            Ok(answer) => answer,
            Err(_) => Err(super::policy::timed_out()),
        }
    }

    /// The answer as text. An empty body is a valid answer here. With a `limit`, the decoded size
    /// is bounded before the response finishes.
    pub async fn read_text(mut self, limit: Option<TextLimit<'_>>) -> Result<String> {
        let Admitted { response, hold } = self.admit().await?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            drop(hold);
            return Err(self.refused(response).await);
        }
        // Lichess redirects some requests (an export without an account) to a web page, which
        // would otherwise reach the caller as if it were the data asked for.
        let asked = self.client.url(&self.path)?;
        let page = response.url().path() != asked.path()
            && response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/html"));
        if page {
            drop(hold);
            return Err(self.failed(LichessError {
                status,
                endpoint: self.endpoint(),
                detail: "unexpected page response".to_string(),
            }));
        }
        let streamed = self.lane.streamed()
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("ndjson"));
        let text = self.body_text(response, limit, streamed).await;
        drop(hold);
        text
    }

    /// The answer as JSON: an empty body is an error that names the path.
    pub async fn read_json<T: DeserializeOwned>(self) -> Result<T> {
        let path = self.path.clone();
        let endpoint = self.endpoint();
        let text = self.read_text(None).await?;
        if text.trim().is_empty() {
            return Err(LichessError {
                status: 200,
                endpoint,
                detail: "empty response".to_string(),
            }
            .into());
        }
        serde_json::from_str(&text).map_err(|cause| {
            CoreError::new(format!(
                "Lichess sent an unreadable response from {path}: {cause}"
            ))
        })
    }

    /// Sends the request; the answer's body is not read for data (Lichess answers `204`).
    pub async fn send(self) -> Result<()> {
        self.read_text(None).await.map(|_| ())
    }

    /// An NDJSON answer, read line by line as it arrives. `on_open` runs once the answer's
    /// status is good, before the first line.
    pub async fn lines<F>(mut self, on_open: impl FnOnce(), on_line: F) -> Result<()>
    where
        F: FnMut(&str) -> Result<()>,
    {
        self.accept = Some("application/x-ndjson");
        let Admitted { response, hold } = self.admit().await?;
        if !response.status().is_success() {
            drop(hold);
            return Err(self.refused(response).await);
        }
        on_open();
        let body = self
            .client
            .policy
            .metered(response.bytes_stream(), &self.auth.usage);
        let options = LineOptions {
            idle: self.client.stream_idle,
            ..LineOptions::default()
        };
        let outcome = read_lines(Box::pin(body), Some(&self.cancel), &options, on_line).await;
        // The export lane (if any) is freed once the body is over, whatever its outcome.
        drop(hold);
        outcome
    }

    /// Every line of an NDJSON answer, read to the end.
    pub async fn read_lines(self) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        self.lines(
            || {},
            |line| {
                lines.push(line.to_string());
                Ok(())
            },
        )
        .await?;
        Ok(lines)
    }

    /// The answer read in chunks as it arrives (a PGN feed).
    pub async fn chunks<F>(mut self, on_chunk: F) -> Result<()>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let Admitted { response, hold } = self.admit().await?;
        if !response.status().is_success() {
            drop(hold);
            return Err(self.refused(response).await);
        }
        let body = self
            .client
            .policy
            .metered(response.bytes_stream(), &self.auth.usage);
        let outcome = read_chunks(Box::pin(body), &self.cancel, on_chunk).await;
        drop(hold);
        outcome
    }
}

/// The Lichess client. Cheap to clone; every clone shares the policy (cooldown and lanes).
#[derive(Clone)]
pub struct LichessClient {
    base: String,
    policy: Arc<Policy>,
    /// How long an NDJSON stream may stay silent before it counts as stalled.
    stream_idle: Duration,
}

impl LichessClient {
    pub fn new(policy: Arc<Policy>) -> LichessClient {
        LichessClient::with_base(BASE_URL, policy)
    }

    /// A client for another origin: the explorer, the tablebase, or a test fixture.
    pub fn with_base(base: &str, policy: Arc<Policy>) -> LichessClient {
        LichessClient {
            base: base.trim_end_matches('/').to_string(),
            policy,
            stream_idle: super::stream::IDLE,
        }
    }

    /// The same client with another stream idle deadline (the tests' stalled feeds).
    #[must_use]
    pub fn with_stream_idle(mut self, idle: Duration) -> LichessClient {
        self.stream_idle = idle;
        self
    }

    pub fn policy(&self) -> &Arc<Policy> {
        &self.policy
    }

    /// No trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// A caller without a login whose traffic is counted as `kind`, against nobody.
    pub fn anonymous(&self, kind: UsageKind) -> Auth {
        self.counted("", kind)
    }

    /// A caller without a login whose traffic is counted as `kind` against `account`. The
    /// account's epoch is read now: traffic after it is forgotten is not counted.
    pub fn counted(&self, account: &str, kind: UsageKind) -> Auth {
        Auth {
            usage: self.policy.usage(account, kind),
            token: None,
        }
    }

    fn url(&self, path: &str) -> Result<reqwest::Url> {
        reqwest::Url::parse(&format!("{}{path}", self.base))
            .map_err(|cause| CoreError::new(format!("Invalid Lichess URL: {cause}")))
    }

    pub fn request(&self, method: reqwest::Method, path: &str, auth: &Auth) -> Request<'_> {
        Request {
            client: self,
            method,
            path: path.to_string(),
            query: Vec::new(),
            body: Body::Empty,
            accept: None,
            auth: auth.clone(),
            lane: Lane::Background,
            cancel: self.policy.lifetime().clone(),
            max_bytes: None,
        }
    }

    pub fn get(&self, path: &str, auth: &Auth) -> Request<'_> {
        self.request(reqwest::Method::GET, path, auth)
    }

    pub fn post(&self, path: &str, auth: &Auth) -> Request<'_> {
        self.request(reqwest::Method::POST, path, auth)
    }

    pub fn delete(&self, path: &str, auth: &Auth) -> Request<'_> {
        self.request(reqwest::Method::DELETE, path, auth)
    }
}

/// A failed answer's body as the domain rule reads it: the parsed JSON, the raw text when it is
/// not JSON, or nothing for an empty body.
fn body_value(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

/// The most an ordinary (non-stream) Lichess answer may hold.
const MAX_BODY: usize = 64 * 1024 * 1024;
const LARGE_BODY: &str = "Lichess sent an unexpectedly large response.";

/// The whole body as text (lossy UTF-8), refused once it passes `max` bytes or `limit`'s UTF-16
/// units, abortable through `cancel`.
pub(crate) async fn read_body(
    response: reqwest::Response,
    cancel: &CancellationToken,
    (max, too_large): (usize, &'static str),
    limit: Option<TextLimit<'_>>,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(CoreError::new(too_large));
    }
    let mut text = String::new();
    let mut units = 0usize;
    read_chunks(Box::pin(response.bytes_stream()), cancel, |chunk: &str| {
        if text.len() + chunk.len() > max {
            return Err(CoreError::new(too_large));
        }
        if let Some(limit) = limit {
            units = units.saturating_add(chunk.encode_utf16().count());
            if units > limit.utf16_units {
                return Err(CoreError::new(limit.message));
            }
        }
        text.push_str(chunk);
        Ok(())
    })
    .await?;
    Ok(text)
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A one-shot local server answering `answer`; its URL.
    async fn serve(answer: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = [0u8; 1024];
            let _ = socket.read(&mut head).await;
            let _ = socket.write_all(&answer).await;
        });
        format!("http://{address}/")
    }

    async fn body(answer: &str, max: usize) -> Result<String> {
        let url = serve(answer.as_bytes().to_vec()).await;
        let response = reqwest::get(url).await.unwrap();
        read_body(response, &CancellationToken::new(), (max, LARGE_BODY), None).await
    }

    /// An ordinary call's body is bounded, with or without a declared length.
    #[tokio::test]
    async fn an_oversized_body_is_refused() {
        let ok = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        assert_eq!(body(ok, 5).await.unwrap(), "hello");
        let declared = "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhello!";
        assert!(body(declared, 5).await.is_err());
        let chunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
                       3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n";
        assert!(body(chunked, 5).await.is_err());
        assert_eq!(body(chunked, 6).await.unwrap(), "abcdef");
    }
}
