//! The Lichess HTTP client (the client half of `crates/kchess-node/js/lichess.ts`): the base URL,
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
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::policy::{Admitted, Policy, is_stream};
use super::stream::{LineOptions, read_chunks, read_lines};
use crate::error::{CoreError, Result as CoreResult};

/// `BASE` of `lichess.ts`.
pub const BASE_URL: &str = "https://lichess.org";

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
            Failure::Lichess(error) => CoreError::new(error.message()),
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

/// `LichessError`: a failed Lichess call. The detail is UI-safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LichessError {
    pub status: u16,
    pub endpoint: String,
    pub detail: String,
}

impl LichessError {
    /// `Lichess ${status} from ${endpoint}: ${detail}`, or without the detail when there is none.
    pub fn message(&self) -> String {
        if self.detail.is_empty() {
            format!("Lichess {} from {}", self.status, self.endpoint)
        } else {
            format!(
                "Lichess {} from {}: {}",
                self.status, self.endpoint, self.detail
            )
        }
    }
}

/// `lichessError(response, error, endpoint)`: the domain rule chooses the detail
/// (`kchess_domain::records::lichess`). `body` is the parsed JSON body, the raw text when the
/// body is not JSON, or None when it is empty.
pub fn lichess_error(status: u16, body: Option<Value>, endpoint: &str) -> LichessError {
    let fields = kchess_domain::records::call(
        "lichessError",
        &[
            json!({ "status": status }),
            body.unwrap_or(Value::Null),
            json!(endpoint),
        ],
    );
    match fields {
        Some(Ok(value)) => LichessError {
            status,
            endpoint: value["endpoint"].as_str().unwrap_or(endpoint).to_string(),
            detail: value["detail"].as_str().unwrap_or("").to_string(),
        },
        _ => LichessError {
            status,
            endpoint: endpoint.to_string(),
            detail: String::new(),
        },
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
}

impl LichessClient {
    pub fn new(policy: Arc<Policy>, http: reqwest::Client) -> LichessClient {
        LichessClient::with_base(BASE_URL, policy, http)
    }

    /// A client for another origin (the deterministic fixtures of the tests).
    pub fn with_base(base: &str, policy: Arc<Policy>, http: reqwest::Client) -> LichessClient {
        LichessClient {
            base: base.trim_end_matches('/').to_string(),
            http,
            policy,
        }
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
        let text = self.read_text(admitted, "GET", path, cancel).await?;
        parse_json(&text, path)
    }

    /// `unwrap(client.POST(path))` with a form body (`urlencoded`).
    pub async fn post_form<T: DeserializeOwned>(
        &self,
        path: &str,
        form: &[(&str, &str)],
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<T> {
        let headers = auth_header(auth);
        let body = Some(("application/x-www-form-urlencoded", urlencoded(form)));
        let admitted = self
            .call(reqwest::Method::POST, path, &[], &headers, body, cancel)
            .await?;
        let text = self.read_text(admitted, "POST", path, cancel).await?;
        parse_json(&text, path)
    }

    /// A text response (`parseAs: 'text'`), such as a game exported as PGN. An empty body is a
    /// valid answer here, as it is for `unwrap`.
    pub async fn get_text(
        &self,
        path: &str,
        query: &[(&str, String)],
        accept: &str,
        auth: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let mut headers = vec![("Accept", accept.to_string())];
        headers.extend(auth_header(auth));
        let admitted = self
            .call(reqwest::Method::GET, path, query, &headers, None, cancel)
            .await?;
        self.read_text(admitted, "GET", path, cancel).await
    }

    /// An NDJSON response, read line by line as it arrives (`parseAs: 'stream'`). `body` is the
    /// comma-separated id list of the `_ids` exports, sent as `text/plain`.
    #[allow(clippy::too_many_arguments)]
    pub async fn ndjson<F: FnMut(&str) -> Result<()>>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
        auth: Option<&str>,
        cancel: &CancellationToken,
        on_line: F,
    ) -> Result<()> {
        let mut headers = vec![("Accept", "application/x-ndjson".to_string())];
        headers.extend(auth_header(auth));
        let body = body.map(|text| ("text/plain", text));
        let admitted = self
            .call(method.clone(), path, query, &headers, body, cancel)
            .await?;
        let Admitted { response, hold } = admitted;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.map_err(read_failure)?;
            return Err(Failure::Lichess(lichess_error(
                status,
                body_value(&text),
                &format!("{method} {path}"),
            )));
        }
        let body = self.policy.metered(response.bytes_stream());
        let outcome = read_lines(
            Box::pin(body),
            Some(cancel),
            &LineOptions::default(),
            on_line,
        )
        .await;
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
            let text = response.text().await.map_err(read_failure)?;
            return Err(Failure::Lichess(lichess_error(
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
    ) -> Result<String> {
        let Admitted { response, hold } = admitted;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let stream = is_stream(path, content_type.as_deref());
        let text = read_with_deadline(response, cancel, stream, self.policy.timeout()).await;
        drop(hold);
        let text = text?;
        if !(200..300).contains(&status) {
            return Err(Failure::Lichess(lichess_error(
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
fn parse_json<T: DeserializeOwned>(text: &str, path: &str) -> Result<T> {
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

/// Read the whole body within the request deadline (a stream is exempt; it has its own idle
/// deadline), abortable through `cancel`.
async fn read_with_deadline(
    response: reqwest::Response,
    cancel: &CancellationToken,
    stream: bool,
    timeout: Duration,
) -> Result<String> {
    let read = async move {
        if stream {
            response.text().await.map_err(read_failure)
        } else {
            match tokio::time::timeout(timeout, response.text()).await {
                Ok(answer) => answer.map_err(read_failure),
                Err(_) => Err(Failure::Core(CoreError::new(
                    "Lichess request timed out. Retry or reconnect.",
                ))),
            }
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Failure::Core(CoreError::aborted("This operation was aborted"))),
        answer = read => answer,
    }
}

fn read_failure(cause: reqwest::Error) -> Failure {
    Failure::Core(CoreError::new(cause.to_string()))
}
