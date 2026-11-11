//! The Lichess HTTP client: the base URL,
//! `authorize`, `urlencoded`, `unwrap` and the error mapping of `throwLichessErrors`, over the
//! admission policy of `policy.rs`.
//!
//! Every call names its account's token through a `TokenSource`, so the client never reads the
//! store itself. Failed responses become `LichessError`s whose message is built by the domain
//! rule `records::lichess` (`lichessError`), so a website's 404 page never reaches the UI.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::policy::{Admitted, Policy, is_stream};
use super::stream::{LineOptions, read_chunks, read_lines};
use crate::error::{CoreError, ErrorKind, Result as CoreResult};

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

/// An NDJSON request (`Client::ndjson`): `body` is the comma-separated id list of the `_ids`
/// exports, sent as `text/plain`.
pub struct Ndjson<'a> {
    pub method: reqwest::Method,
    pub path: &'a str,
    pub query: &'a [(&'a str, String)],
    pub body: Option<String>,
    pub auth: Option<&'a str>,
}

/// A failed Lichess call: the API's own error (with its status and endpoint), or a failure of
/// the core itself (a rejected request, a stalled stream, an abort).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Lichess(LichessError),
    Core(CoreError),
}

pub type Result<T> = std::result::Result<T, Failure>;

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Failure {
        Failure::Core(error)
    }
}

impl From<Failure> for CoreError {
    fn from(failure: Failure) -> CoreError {
        match failure {
            Failure::Lichess(error) => CoreError::new(error.message()).with_kind(error.kind()),
            Failure::Core(error) => error,
        }
    }
}

impl Failure {
    /// The HTTP status when the failure is Lichess's answer.
    pub fn status(&self) -> Option<u16> {
        match self {
            Failure::Lichess(error) => Some(error.status),
            Failure::Core(_) => None,
        }
    }

    /// Whether the failure is an abort (`AbortError` on the JavaScript side).
    pub fn is_aborted(&self) -> bool {
        matches!(self, Failure::Core(error) if error.aborted)
    }

    /// The message shown to the user.
    pub fn message(&self) -> String {
        match self {
            Failure::Lichess(error) => error.message(),
            Failure::Core(error) => error.message.clone(),
        }
    }
}

/// `LichessError`: a failed Lichess call, its status, endpoint and the start of its body.
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

    /// `Lichess ${status} from ${endpoint}: ${detail}`: for the log, never for a person.
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

/// The error of a failed response: the domain rule chooses the detail
/// (`kchess_domain::records::lichess`). `body` is the parsed JSON body, the raw text when the
/// body is not JSON, or None when it is empty.
pub fn lichess_error(status: u16, body: Option<Value>, endpoint: &str) -> LichessError {
    LichessError {
        status,
        endpoint: endpoint.to_string(),
        detail: kchess_domain::records::lichess::error_detail(Some(f64::from(status)), body.as_ref()),
    }
}

/// `authorize(token)`: the `Authorization` header value for an account's token.
pub fn authorize(token: &str) -> String {
    format!("Bearer {token}")
}

/// `urlencoded(body)`: `application/x-www-form-urlencoded` text, as `URLSearchParams` writes it.
pub fn urlencoded(pairs: &[(&str, &str)]) -> String {
    let Ok(mut url) = reqwest::Url::parse("http://localhost/") else {
        return String::new();
    };
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url.query().unwrap_or("").to_string()
}

/// The token of an account, read by the caller (`getToken` in TypeScript). `Ok(None)` when the
/// account has no login. Implemented over the store and the host's credential store later.
pub trait TokenSource: Send + Sync {
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, CoreResult<Option<String>>>;
}

/// The Lichess client. Cheap to clone; every clone shares the policy (cooldown and lanes).
#[derive(Clone)]
pub struct LichessClient {
    base: String,
    http: reqwest::Client,
    policy: Arc<Policy>,
    /// How long an NDJSON stream may stay silent before it counts as stalled.
    stream_idle: std::time::Duration,
}

impl LichessClient {
    /// Lichess's failed answer: its technical form goes to the log, the person sees `message`.
    fn failed(&self, error: LichessError) -> Failure {
        self.policy.log_warning(&error.technical());
        Failure::Lichess(error)
    }

    pub fn new(policy: Arc<Policy>, http: reqwest::Client) -> LichessClient {
        LichessClient::with_base(BASE_URL, policy, http)
    }

    /// A client for another origin (the deterministic fixtures of the tests).
    pub fn with_base(base: &str, policy: Arc<Policy>, http: reqwest::Client) -> LichessClient {
        LichessClient {
            base: base.trim_end_matches('/').to_string(),
            http,
            policy,
            stream_idle: super::stream::IDLE,
        }
    }

    /// The same client with another stream idle deadline (the tests' stalled feeds).
    #[must_use]
    pub fn with_stream_idle(mut self, idle: std::time::Duration) -> LichessClient {
        self.stream_idle = idle;
        self
    }

    pub fn policy(&self) -> &Arc<Policy> {
        &self.policy
    }

    /// The origin requests go to (no trailing slash).
    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> CoreResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.base))
            .map_err(|cause| CoreError::new(format!("Invalid Lichess URL: {cause}")))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        }
        Ok(url)
    }

    /// Build and admit one request.
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        body: Option<(&str, String)>,
        cancel: &CancellationToken,
    ) -> Result<Admitted> {
        let url = self.url(path, query)?;
        let mut builder = self.http.request(method, url);
        for (name, value) in headers {
            builder = builder.header(*name, value.as_str());
        }
        if let Some((content_type, text)) = body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(text);
        }
        let request = builder
            .build()
            .map_err(|cause| CoreError::new(format!("Invalid Lichess request: {cause}")))?;
        Ok(self.policy.send(request, cancel).await?)
    }

    /// `unwrap(client.GET(path))` for a JSON body: a non-2xx answer is a `LichessError`, an
    /// empty body is one too, and the body is read within the request deadline.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<T> {
        let headers = auth_header(auth);
        let admitted = self
            .call(reqwest::Method::GET, path, query, &headers, None, cancel)
            .await?;
        let text = self.read_text(admitted, "GET", path, cancel, None).await?;
        parse_json(&text, path)
    }

    /// `unwrap(client.POST(path))` with a form body (`urlencoded`). It asks for JSON: without that,
    /// some endpoints (`/api/import`) answer with a redirect to a web page.
    pub async fn post_form<T: DeserializeOwned>(
        &self,
        path: &str,
        form: &[(&str, &str)],
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<T> {
        let mut headers = vec![("Accept", "application/json".to_string())];
        headers.extend(auth_header(auth));
        let body = Some(("application/x-www-form-urlencoded", urlencoded(form)));
        let admitted = self
            .call(reqwest::Method::POST, path, &[], &headers, body, cancel)
            .await?;
        let text = self.read_text(admitted, "POST", path, cancel, None).await?;
        parse_json(&text, path)
    }

    /// `POST path` with a plain-text body, such as the comma-separated names of `/api/users`.
    pub async fn post_text<T: DeserializeOwned>(
        &self,
        path: &str,
        text: String,
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<T> {
        let headers = auth_header(auth);
        let body = Some(("text/plain", text));
        let admitted = self
            .call(reqwest::Method::POST, path, &[], &headers, body, cancel)
            .await?;
        let text = self.read_text(admitted, "POST", path, cancel, None).await?;
        parse_json(&text, path)
    }

    /// `DELETE path`, whose body is not read for data (Lichess answers `204`).
    pub async fn delete(
        &self,
        path: &str,
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let headers = auth_header(auth);
        let admitted = self
            .call(reqwest::Method::DELETE, path, &[], &headers, None, cancel)
            .await?;
        self.read_text(admitted, "DELETE", path, cancel, None)
            .await
            .map(|_| ())
    }

    /// A text response (`parseAs: 'text'`), such as a game exported as PGN. An empty body is a
    /// valid answer here, as it is for `unwrap`. With a `limit`, the decoded size is bounded
    /// before the response finishes.
    pub async fn get_text(
        &self,
        path: &str,
        query: &[(&str, String)],
        accept: &str,
        auth: Option<&str>,
        cancel: &CancellationToken,
        limit: Option<TextLimit<'_>>,
    ) -> Result<String> {
        let mut headers = vec![("Accept", accept.to_string())];
        headers.extend(auth_header(auth));
        let admitted = self
            .call(reqwest::Method::GET, path, query, &headers, None, cancel)
            .await?;
        self.read_text(admitted, "GET", path, cancel, limit).await
    }

    /// One game from `POST /api/games/export/_ids`, which Lichess answers without an account;
    /// `/game/export/{id}` sends a request without one to its sign-up page. An empty answer means
    /// Lichess has no such game. The single game does not wait for the bulk export lane.
    pub async fn export_one(
        &self,
        id: &str,
        query: &[(&str, String)],
        accept: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let path = "/api/games/export/_ids";
        let request = self
            .http
            .post(self.url(path, query)?)
            .header("Accept", accept)
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(id.to_string())
            .build()
            .map_err(|cause| CoreError::new(format!("Invalid Lichess request: {cause}")))?;
        let admitted = self.policy.send_single(request, cancel).await?;
        self.read_text(admitted, "POST", path, cancel, None).await
    }

    /// An NDJSON response, read line by line as it arrives (`parseAs: 'stream'`). `body` is the
    /// comma-separated id list of the `_ids` exports, sent as `text/plain`.
    pub async fn ndjson<F: FnMut(&str) -> Result<()>>(
        &self,
        request: Ndjson<'_>,
        cancel: &CancellationToken,
        on_line: F,
    ) -> Result<()> {
        let Ndjson {
            method,
            path,
            query,
            body,
            auth,
        } = request;
        let mut headers = vec![("Accept", "application/x-ndjson".to_string())];
        headers.extend(auth_header(auth));
        let body = body.map(|text| ("text/plain", text));
        let admitted = self
            .call(method.clone(), path, query, &headers, body, cancel)
            .await?;
        let Admitted { response, hold } = admitted;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text =
                read_with_deadline(response, cancel, false, self.policy.timeout(), None).await?;
            return Err(self.failed(lichess_error(
                status,
                body_value(&text),
                &format!("{method} {path}"),
            )));
        }
        let body = self.policy.metered(response.bytes_stream());
        let options = LineOptions {
            idle: self.stream_idle,
            ..LineOptions::default()
        };
        let outcome = read_lines(Box::pin(body), Some(cancel), &options, on_line).await;
        // The export lane (if any) is freed once the body is over, whatever its outcome.
        drop(hold);
        outcome
    }

    /// A GET response read in chunks as it arrives (a PGN feed, `parseAs: 'stream'` without
    /// lines). Errors are the same as `ndjson`'s.
    pub async fn stream_chunks<F: FnMut(&str) -> Result<()>>(
        &self,
        path: &str,
        auth: Option<&str>,
        cancel: &CancellationToken,
        on_chunk: F,
    ) -> Result<()> {
        let headers = auth_header(auth);
        let Admitted { response, hold } = self
            .call(reqwest::Method::GET, path, &[], &headers, None, cancel)
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text =
                read_with_deadline(response, cancel, false, self.policy.timeout(), None).await?;
            return Err(self.failed(lichess_error(
                status,
                body_value(&text),
                &format!("GET {path}"),
            )));
        }
        let body = self.policy.metered(response.bytes_stream());
        let outcome = read_chunks(Box::pin(body), cancel, on_chunk).await;
        drop(hold);
        outcome
    }

    /// Read a response body to text, checking its status. A non-stream body shares the
    /// request's deadline; a stream has its own idle deadline.
    async fn read_text(
        &self,
        admitted: Admitted,
        method: &str,
        path: &str,
        cancel: &CancellationToken,
        limit: Option<TextLimit<'_>>,
    ) -> Result<String> {
        let Admitted { response, hold } = admitted;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        // Lichess redirects some requests (an export without an account) to a web page, which
        // would otherwise reach the caller as if it were the data asked for.
        let asked = self.url(path, &[])?;
        let page = (200..300).contains(&status)
            && response.url().path() != asked.path()
            && content_type
                .as_deref()
                .is_some_and(|value| value.starts_with("text/html"));
        if page {
            drop(hold);
            return Err(self.failed(LichessError {
                status,
                endpoint: format!("{method} {path}"),
                detail: "unexpected page response".to_string(),
            }));
        }
        let stream = (200..300).contains(&status) && is_stream(path, content_type.as_deref());
        let text = read_with_deadline(response, cancel, stream, self.policy.timeout(), limit).await;
        drop(hold);
        let text = text?;
        if !(200..300).contains(&status) {
            return Err(self.failed(lichess_error(
                status,
                body_value(&text),
                &format!("{method} {path}"),
            )));
        }
        Ok(text)
    }
}

fn auth_header(auth: Option<&str>) -> Vec<(&'static str, String)> {
    auth.map(|token| vec![("Authorization", token.to_string())])
        .unwrap_or_default()
}

/// `body_value` of `throwLichessErrors`: the parsed JSON, the raw text when it is not JSON, or
/// nothing for an empty body.
fn body_value(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

/// `unwrap`: an empty body is an error that names the path; otherwise the JSON is parsed.
pub(crate) fn parse_json<T: DeserializeOwned>(text: &str, path: &str) -> Result<T> {
    if text.trim().is_empty() {
        return Err(Failure::Lichess(LichessError {
            status: 200,
            endpoint: path.to_string(),
            detail: "empty response".to_string(),
        }));
    }
    serde_json::from_str(text).map_err(|cause| {
        Failure::Core(CoreError::new(format!(
            "Lichess sent an unreadable response from {path}: {cause}"
        )))
    })
}

/// The most an ordinary (non-stream) Lichess answer may hold.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// The body as text, refused once it passes `max` bytes.
async fn capped_body(response: reqwest::Response, max: usize) -> Result<String> {
    use futures_util::StreamExt as _;
    let dropped = || {
        Failure::Core(
            CoreError::new("The connection to Lichess dropped.").with_kind(ErrorKind::Offline),
        )
    };
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(Failure::Core(CoreError::new(
            "Lichess sent an unexpectedly large response.",
        )));
    }
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| dropped())?;
        if body.len() + chunk.len() > max {
            return Err(Failure::Core(CoreError::new(
                "Lichess sent an unexpectedly large response.",
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Read the whole body within the request deadline (a stream is exempt: line readers bound
/// their own silence and chunk readers wait until the body ends), abortable through `cancel`.
pub(crate) async fn read_with_deadline(
    response: reqwest::Response,
    cancel: &CancellationToken,
    stream: bool,
    timeout: Duration,
    limit: Option<TextLimit<'_>>,
) -> Result<String> {
    let read = async move {
        // Ordinary calls decode as reqwest's `text()` does without its charset feature (lossy
        // UTF-8 of the whole body), but no further than `MAX_BODY`: the deadline alone let a
        // misbehaving endpoint send hundreds of megabytes first.
        if !stream && limit.is_none() {
            return tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Failure::Core(CoreError::aborted("This operation was aborted"))),
                body = capped_body(response, MAX_BODY) => body,
            };
        }
        let mut text = String::new();
        let mut units = 0usize;
        read_chunks(Box::pin(response.bytes_stream()), cancel, |chunk| {
            if let Some(limit) = limit {
                units = units.saturating_add(chunk.encode_utf16().count());
                if units > limit.utf16_units {
                    return Err(Failure::Core(CoreError::new(limit.message)));
                }
            }
            text.push_str(chunk);
            Ok(())
        })
        .await?;
        Ok(text)
    };
    if stream {
        read.await
    } else {
        match tokio::time::timeout(timeout, read).await {
            Ok(answer) => answer,
            Err(_) => Err(Failure::Core(super::policy::timed_out())),
        }
    }
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
        capped_body(response, max).await
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
