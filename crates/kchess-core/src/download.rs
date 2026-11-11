//! The verified downloads (the managed engine, the voice model, the puzzle database) and their
//! metadata go through one pipeline: [`open`] sends the request, [`Body::next`] reads the answer.
//! Together they bound the size and the time, stop the moment the caller cancels, and count every
//! request and byte as network use, retries and resumes included.
//!
//! The first request is sent again after a brief server failure (502, 503, 504, a 429 with a
//! short `Retry-After`, or a connection that failed before any answer), so a single hiccup at the
//! host does not fail a download.
//!
//! A body survives a dropped connection (`Resumable`): it asks again from the byte it reached
//! (`Range`), so the bytes already received are not fetched twice. It resumes only when that is
//! safe: the first answer was a whole `200` that named its length, offered byte ranges and gave a
//! validator (`ETag`, or `Last-Modified`), and the new answer is a `206` for exactly the rest of
//! that same file (`If-Range`, `Content-Range`). Anything else ends the read with the failure the
//! connection had.

use std::time::{Duration, Instant};

use kchess_contracts::types::UsageKind;
use reqwest::StatusCode;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap, IF_RANGE, LAST_MODIFIED, RANGE,
    RETRY_AFTER,
};
use tokio_util::sync::CancellationToken;

use crate::usage::Meter;
use kchess_contracts::error::{CoreError, ErrorKind, Result};

pub use crate::tasks::ABORTED;

/// The HTTP client of a download. It asks for the body as published (no gzip on top), so the
/// length is the progress total, digests are of the published file and a body can resume.
/// `silence`, when given, ends a download whose server stops sending for that long.
pub fn client(silence: Option<Duration>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .no_gzip();
    if let Some(silence) = silence {
        builder = builder.read_timeout(silence);
    }
    builder
        .build()
        .map_err(|cause| CoreError::wrap("Could not start the download", cause))
}

/// How long a download may take to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// What one download may bring and how long it may take, in all.
#[derive(Clone, Copy)]
pub struct Limits {
    pub max_bytes: u64,
    pub deadline: Option<Instant>,
}

/// How one download words its failures for the person who asked for it.
pub struct Wording {
    /// A request that failed or timed out.
    pub network: fn(&reqwest::Error) -> String,
    /// An answer other than success, by its status code.
    pub status: fn(u16) -> String,
    /// A body larger than `Limits::max_bytes`.
    pub too_large: &'static str,
}

/// A request that failed or timed out, in `wording`'s words: an `Offline` (or `TimedOut`) error,
/// so a caller can tell the person to check the connection only when the connection failed.
fn network_error(wording: &Wording, cause: &reqwest::Error) -> CoreError {
    let kind = if cause.is_timeout() {
        ErrorKind::TimedOut
    } else {
        ErrorKind::Offline
    };
    CoreError::new((wording.network)(cause)).with_kind(kind)
}

/// Where a download's traffic is counted.
#[derive(Clone, Copy)]
pub struct Usage<'a> {
    pub meter: &'a dyn Meter,
    pub kind: UsageKind,
}

/// An answer being read. Its traffic is counted when it is dropped, however the read ended.
pub struct Body<'a> {
    body: Resumable,
    total: Option<u64>,
    received: u64,
    requests: u64,
    limits: Limits,
    cancel: &'a CancellationToken,
    usage: Usage<'a>,
    wording: &'a Wording,
}

/// Sends the request `request` builds (again after a transient failure) and checks the answer:
/// a success, not announced larger than the limit. Cancelling `cancel` ends the wait at once.
pub async fn open<'a>(
    client: &reqwest::Client,
    request: impl Fn() -> reqwest::RequestBuilder,
    limits: Limits,
    cancel: &'a CancellationToken,
    usage: Usage<'a>,
    wording: &'a Wording,
) -> Result<Body<'a>> {
    let mut requests = 0;
    let sent = tokio::select! {
        biased;
        () = cancel.cancelled() => Err(crate::tasks::aborted()),
        sent = send_retrying(request, limits.deadline, &mut requests) => {
            sent.map_err(|cause| network_error(wording, &cause))
        }
    };
    let counted = |requests| usage.meter.count("", usage.kind, requests, 0);
    let response = match sent {
        Ok(response) => response,
        Err(error) => {
            counted(requests);
            return Err(error);
        }
    };
    if !response.status().is_success() {
        counted(requests);
        return Err(CoreError::new((wording.status)(response.status().as_u16())));
    }
    let total = response.content_length().filter(|&total| total > 0);
    if total.is_some_and(|total| total > limits.max_bytes) {
        counted(requests);
        return Err(CoreError::new(wording.too_large));
    }
    Ok(Body {
        body: Resumable::new(client, response, limits.deadline),
        total,
        received: 0,
        requests,
        limits,
        cancel,
        usage,
        wording,
    })
}

impl Body<'_> {
    /// The length the server announced, if it did.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// The bytes received so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// The next chunk; `None` once the body has all arrived. Fails past the size limit, when
    /// the connection is lost for good, or as an abort once the caller cancels.
    pub async fn next(&mut self) -> Result<Option<bytes::Bytes>> {
        let chunk = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(crate::tasks::aborted()),
            chunk = self.body.chunk() => chunk,
        };
        let chunk = chunk.map_err(|cause| network_error(self.wording, &cause))?;
        if let Some(chunk) = &chunk {
            self.received += chunk.len() as u64;
            if self.received > self.limits.max_bytes {
                return Err(CoreError::new(self.wording.too_large));
            }
        }
        Ok(chunk)
    }

    /// The whole body, for metadata.
    pub async fn bytes(mut self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

impl Drop for Body<'_> {
    fn drop(&mut self) {
        self.usage.meter.count(
            "",
            self.usage.kind,
            self.requests + self.body.resumed() as u64,
            self.received,
        );
    }
}

/// How long `send_retrying` waits before each further attempt (three attempts in all).
#[cfg(not(test))]
const SEND_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
#[cfg(test)]
const SEND_WAITS: [Duration; 2] = [Duration::from_millis(1); 2];

/// The longest `Retry-After` a 429 may ask for and still be waited out.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Sends the request `request` builds, again after a transient failure: a 502, 503 or 504, a 429
/// whose `Retry-After` is at most ten seconds, or a connection that failed before any answer
/// (not a timeout). Every other answer, a 404 included, comes back at once. After the last
/// attempt the caller gets that attempt's answer or error, as if it had sent only once.
///
/// `deadline`, when given, bounds every attempt together, as one request timeout bounded the
/// single attempt: each one gets the time that is left, and no wait runs past it. Dropping the
/// future (a cancelled install) stops it between or during attempts. `attempts` counts the
/// requests sent.
async fn send_retrying(
    request: impl Fn() -> reqwest::RequestBuilder,
    deadline: Option<Instant>,
    attempts: &mut u64,
) -> reqwest::Result<reqwest::Response> {
    let mut waits = SEND_WAITS.iter();
    loop {
        let mut builder = request();
        if let Some(deadline) = deadline {
            builder = builder.timeout(deadline.saturating_duration_since(Instant::now()));
        }
        *attempts += 1;
        let outcome = builder.send().await;
        let Some(wait) = retry_wait(&outcome) else {
            return outcome;
        };
        let Some(&base) = waits.next() else {
            return outcome;
        };
        let wait = wait.max(base);
        if deadline.is_some_and(|deadline| Instant::now() + wait >= deadline) {
            return outcome;
        }
        tokio::time::sleep(wait).await;
    }
}

/// How long a transient failure asks to be waited out at least; `None` when it is final.
fn retry_wait(outcome: &reqwest::Result<reqwest::Response>) -> Option<Duration> {
    match outcome {
        Err(error) => (error.is_request() && !error.is_timeout()).then_some(Duration::ZERO),
        Ok(response) => match response.status() {
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => Some(Duration::ZERO),
            StatusCode::TOO_MANY_REQUESTS => retry_after(response.headers()),
            _ => None,
        },
    }
}

/// A 429's `Retry-After` in seconds, when it is short enough to wait out.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let wait = Duration::from_secs(seconds);
    (wait <= MAX_RETRY_AFTER).then_some(wait)
}

/// How often one download may resume, and how long it waits before each attempt.
#[cfg(not(test))]
const RESUME_WAITS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(8),
];
#[cfg(test)]
const RESUME_WAITS: [Duration; 3] = [Duration::from_millis(1); 3];

/// A download body read chunk by chunk, resumed after a dropped connection.
pub struct Resumable {
    client: reqwest::Client,
    url: reqwest::Url,
    response: reqwest::Response,
    /// The file's length and validator, when resuming is possible.
    resume: Option<(u64, String)>,
    received: u64,
    resumed: usize,
    deadline: Option<Instant>,
}

impl Resumable {
    /// Read `response`, the answer to a plain GET of `response.url()` by `client`. `deadline`
    /// bounds the resumed requests as a request timeout bounds the first.
    pub fn new(
        client: &reqwest::Client,
        response: reqwest::Response,
        deadline: Option<Instant>,
    ) -> Resumable {
        let resume = (response.status() == StatusCode::OK)
            .then(|| resume_terms(response.headers()))
            .flatten();
        Resumable {
            client: client.clone(),
            url: response.url().clone(),
            response,
            resume,
            received: 0,
            resumed: 0,
            deadline,
        }
    }

    /// How many times the download has resumed (each one a request).
    pub fn resumed(&self) -> usize {
        self.resumed
    }

    /// The next chunk of the body; `None` once it has all arrived.
    pub async fn chunk(&mut self) -> reqwest::Result<Option<bytes::Bytes>> {
        loop {
            match self.response.chunk().await {
                Ok(chunk) => {
                    if let Some(chunk) = &chunk {
                        self.received += chunk.len() as u64;
                    }
                    return Ok(chunk);
                }
                Err(error) => {
                    if !self.resume_after(&error).await {
                        return Err(error);
                    }
                }
            }
        }
    }

    /// Asks for the rest of the file; whether the read can go on.
    async fn resume_after(&mut self, error: &reqwest::Error) -> bool {
        // A body read past its deadline has run out of time, not lost its connection.
        if error.is_timeout() && self.deadline.is_some_and(|at| Instant::now() >= at) {
            return false;
        }
        let Some((length, validator)) = self.resume.clone() else {
            return false;
        };
        while let Some(wait) = RESUME_WAITS.get(self.resumed) {
            self.resumed += 1;
            tokio::time::sleep(*wait).await;
            let mut request = self
                .client
                .get(self.url.clone())
                .header(RANGE, format!("bytes={}-", self.received))
                .header(IF_RANGE, &validator);
            if let Some(deadline) = self.deadline {
                let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                    return false;
                };
                request = request.timeout(left);
            }
            let Ok(response) = request.send().await else {
                continue;
            };
            if response.status() != StatusCode::PARTIAL_CONTENT {
                // A changed file comes back whole (`If-Range`): the bytes so far are not its.
                return false;
            }
            let expected = format!("bytes {}-{}/{length}", self.received, length - 1);
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok());
            if range != Some(expected.as_str()) {
                return false;
            }
            self.response = response;
            return true;
        }
        false
    }
}

/// The file's length and validator when the server offers byte ranges for it. A weak ETag
/// cannot be used with `If-Range`; `Last-Modified` stands in for it.
fn resume_terms(headers: &HeaderMap) -> Option<(u64, String)> {
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if !text(ACCEPT_RANGES).is_some_and(|value| value.eq_ignore_ascii_case("bytes")) {
        return None;
    }
    // A coded body's length is not the file's: such a body is not resumed.
    if headers.contains_key(reqwest::header::CONTENT_ENCODING) {
        return None;
    }
    let length: u64 = text(CONTENT_LENGTH)?.parse().ok().filter(|&n| n > 0)?;
    let validator = text(ETAG)
        .filter(|tag| !tag.starts_with("W/"))
        .or_else(|| text(LAST_MODIFIED))?;
    Some((length, validator.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// What the fixture does with a request.
    #[derive(Clone, Copy, PartialEq)]
    enum Serve {
        /// Sends half of what was asked for, then drops the connection.
        Drop,
        /// Answers as asked: the whole file, or the requested range.
        Whole,
        /// Ignores `Range` and sends the whole file (the file changed).
        Changed,
    }

    struct Fixture {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    /// A file server answering requests in turn by `plan` (the last entry repeats); `ranges`
    /// says whether it offers byte ranges.
    async fn fixture(body: Vec<u8>, plan: Vec<Serve>, ranges: bool) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file.zst", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            for turn in 0.. {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut head = Vec::new();
                let mut buffer = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    head.extend_from_slice(&buffer[..read]);
                }
                let head = String::from_utf8_lossy(&head).to_lowercase();
                seen.lock().unwrap().push(head.clone());
                let start: Option<usize> = head
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .and_then(|range| range.trim_end_matches('-').parse().ok());
                let serve = plan[turn.min(plan.len() - 1)];
                let offer = if ranges {
                    "Accept-Ranges: bytes\r\nETag: \"v1\"\r\n"
                } else {
                    ""
                };
                let answer = match (start, serve) {
                    (Some(from), Serve::Whole | Serve::Drop) => {
                        let rest = &body[from..];
                        let sent = if serve == Serve::Drop {
                            rest.len() / 2
                        } else {
                            rest.len()
                        };
                        let mut answer = format!(
                            "HTTP/1.1 206 Partial Content\r\n{offer}Content-Range: bytes {from}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len() - 1,
                            body.len(),
                            rest.len()
                        )
                        .into_bytes();
                        answer.extend_from_slice(&rest[..sent]);
                        answer
                    }
                    _ => {
                        let mut answer = format!(
                            "HTTP/1.1 200 OK\r\n{offer}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        let sent = if serve == Serve::Drop {
                            body.len() / 2
                        } else {
                            body.len()
                        };
                        answer.extend_from_slice(&body[..sent]);
                        answer
                    }
                };
                let _ = socket.write_all(&answer).await;
                let _ = socket.shutdown().await;
            }
        });
        Fixture { url, requests }
    }

    fn file() -> Vec<u8> {
        (0..200_000u32).map(|n| (n % 251) as u8).collect()
    }

    async fn read_all(fixture: &Fixture) -> (reqwest::Result<Vec<u8>>, usize) {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client.get(&fixture.url).send().await.unwrap();
        let mut body = Resumable::new(&client, response, None);
        let mut bytes = Vec::new();
        loop {
            match body.chunk().await {
                Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                Ok(None) => return (Ok(bytes), body.resumed()),
                Err(error) => return (Err(error), body.resumed()),
            }
        }
    }

    #[tokio::test]
    async fn a_dropped_download_resumes_from_the_byte_it_reached() {
        let fixture = fixture(file(), vec![Serve::Drop, Serve::Whole], true).await;
        let (bytes, resumed) = read_all(&fixture).await;
        assert_eq!(bytes.unwrap(), file());
        assert_eq!(resumed, 1);
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains(&format!("range: bytes={}-", file().len() / 2)));
        assert!(requests[1].contains("if-range: \"v1\""));
    }

    #[tokio::test]
    async fn a_server_without_byte_ranges_fails_as_before() {
        let fixture = fixture(file(), vec![Serve::Drop, Serve::Whole], false).await;
        let (bytes, resumed) = read_all(&fixture).await;
        assert!(bytes.is_err());
        assert_eq!((resumed, fixture.requests.lock().unwrap().len()), (0, 1));
    }

    #[tokio::test]
    async fn a_file_that_changed_is_not_stitched_onto_the_old_one() {
        let fixture = fixture(file(), vec![Serve::Drop, Serve::Changed], true).await;
        let (bytes, resumed) = read_all(&fixture).await;
        assert!(bytes.is_err());
        assert_eq!(resumed, 1);
    }

    #[tokio::test]
    async fn gives_up_after_three_resumes() {
        let fixture = fixture(file(), vec![Serve::Drop], true).await;
        let (bytes, resumed) = read_all(&fixture).await;
        assert!(bytes.is_err());
        assert_eq!(resumed, 3);
        assert_eq!(fixture.requests.lock().unwrap().len(), 4);
    }

    /// A server answering connections in turn by `answers` (the last repeats): a status line and
    /// headers, `CLOSE` to hang up before answering, or `HANG` to never answer. It counts the
    /// connections.
    async fn answering(answers: Vec<&'static str>) -> (String, Arc<Mutex<usize>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let count = Arc::new(Mutex::new(0));
        let seen = Arc::clone(&count);
        tokio::spawn(async move {
            for turn in 0.. {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                *seen.lock().unwrap() += 1;
                let answer = answers[turn.min(answers.len() - 1)];
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n")
                        && matches!(socket.read(&mut byte).await, Ok(1))
                    {
                        head.push(byte[0]);
                    }
                    match answer {
                        CLOSE => {}
                        HANG => std::future::pending::<()>().await,
                        head => {
                            let reply = format!(
                                "HTTP/1.1 {head}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                            );
                            let _ = socket.write_all(reply.as_bytes()).await;
                            crate::loopback::close_after_client(&mut socket).await;
                        }
                    }
                });
            }
        });
        (url, count)
    }

    const CLOSE: &str = "close";
    const HANG: &str = "hang";

    /// Sends to `url` through `send_retrying`; the final status (or `None` for an error), and how
    /// many requests it counted.
    async fn send(
        client: &reqwest::Client,
        url: &str,
        deadline: Option<Instant>,
    ) -> (Option<u16>, u64) {
        let mut attempts = 0;
        let outcome = send_retrying(|| client.get(url), deadline, &mut attempts).await;
        (
            outcome.ok().map(|response| response.status().as_u16()),
            attempts,
        )
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn later() -> Option<Instant> {
        Some(Instant::now() + Duration::from_secs(30))
    }

    #[tokio::test]
    async fn a_gateway_failure_is_sent_again_until_it_answers() {
        for failure in ["502 Bad Gateway", "503 Unavailable", "504 Gateway Timeout"] {
            let (url, connections) = answering(vec![failure, "200 OK"]).await;
            assert_eq!(
                send(&client(), &url, later()).await,
                (Some(200), 2),
                "{failure}"
            );
            assert_eq!(*connections.lock().unwrap(), 2);
        }
    }

    #[tokio::test]
    async fn a_lasting_failure_answers_after_three_attempts() {
        let (url, connections) = answering(vec!["504 Gateway Timeout"]).await;
        assert_eq!(send(&client(), &url, None).await, (Some(504), 3));
        assert_eq!(*connections.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn a_refusal_is_not_sent_again() {
        for refusal in ["404 Not Found", "403 Forbidden", "500 Internal", "429 Slow"] {
            let (url, connections) = answering(vec![refusal, "200 OK"]).await;
            let status: u16 = refusal[..3].parse().unwrap();
            assert_eq!(send(&client(), &url, later()).await, (Some(status), 1));
            assert_eq!(*connections.lock().unwrap(), 1);
        }
    }

    #[tokio::test]
    async fn a_rate_limit_is_waited_out_only_when_it_is_short() {
        let (url, _) = answering(vec!["429 Slow\r\nRetry-After: 0", "200 OK"]).await;
        assert_eq!(send(&client(), &url, later()).await, (Some(200), 2));
        let (url, _) = answering(vec!["429 Slow\r\nRetry-After: 10", "200 OK"]).await;
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "10".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(10)));
        headers.insert(RETRY_AFTER, "11".parse().unwrap());
        assert_eq!(retry_after(&headers), None);
        headers.insert(RETRY_AFTER, "soon".parse().unwrap());
        assert_eq!(retry_after(&headers), None);
        // Ten seconds would outlast this deadline, so the 429 is the answer.
        let soon = Some(Instant::now() + Duration::from_secs(2));
        assert_eq!(send(&client(), &url, soon).await, (Some(429), 1));
        let (url, _) = answering(vec!["429 Slow\r\nRetry-After: 3600", "200 OK"]).await;
        assert_eq!(send(&client(), &url, None).await, (Some(429), 1));
    }

    #[tokio::test]
    async fn a_connection_closed_before_any_answer_is_sent_again() {
        let (url, connections) = answering(vec![CLOSE, "200 OK"]).await;
        assert_eq!(send(&client(), &url, later()).await, (Some(200), 2));
        assert_eq!(*connections.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_timeout_or_a_bad_request_is_not_sent_again() {
        let (url, connections) = answering(vec![HANG]).await;
        let impatient = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        assert_eq!(send(&impatient, &url, None).await, (None, 1));
        assert_eq!(*connections.lock().unwrap(), 1);
        assert_eq!(send(&client(), "not a url", None).await, (None, 1));
    }

    #[tokio::test]
    async fn no_attempt_runs_past_the_deadline() {
        let (url, connections) = answering(vec![HANG]).await;
        let started = Instant::now();
        let deadline = Some(started + Duration::from_millis(200));
        assert_eq!(send(&client(), &url, deadline).await, (None, 1));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(*connections.lock().unwrap(), 1);
    }

    /// Sums the requests and bytes reported as network use.
    #[derive(Default)]
    struct Metered(Mutex<(u64, u64)>);

    impl Meter for Metered {
        fn count(&self, account: &str, kind: UsageKind, requests: u64, bytes: u64) {
            assert_eq!((account, kind), ("", UsageKind::Database));
            let mut total = self.0.lock().unwrap();
            total.0 += requests;
            total.1 += bytes;
        }
    }

    const WORDING: Wording = Wording {
        network: |_| "network".into(),
        status: |status| format!("status {status}"),
        too_large: "too large",
    };

    const UNLIMITED: Limits = Limits {
        max_bytes: u64::MAX,
        deadline: None,
    };

    async fn fetch(
        url: &str,
        limits: Limits,
        cancel: &CancellationToken,
        host: &Metered,
    ) -> Result<Vec<u8>> {
        let client = client();
        let usage = Usage {
            meter: host,
            kind: UsageKind::Database,
        };
        open(&client, || client.get(url), limits, cancel, usage, &WORDING)
            .await?
            .bytes()
            .await
    }

    #[tokio::test]
    async fn usage_counts_every_retry_resume_and_byte() {
        let host = Metered::default();
        let (url, _) = answering(vec!["503 Unavailable", "200 OK"]).await;
        let body = fetch(&url, UNLIMITED, &CancellationToken::new(), &host).await;
        assert_eq!(body.unwrap(), b"ok");
        assert_eq!(*host.0.lock().unwrap(), (2, 2));

        let host = Metered::default();
        let fixture = fixture(file(), vec![Serve::Drop, Serve::Whole], true).await;
        let body = fetch(&fixture.url, UNLIMITED, &CancellationToken::new(), &host).await;
        assert_eq!(body.unwrap(), file());
        assert_eq!(*host.0.lock().unwrap(), (2, file().len() as u64));

        let host = Metered::default();
        let (url, _) = answering(vec!["404 Not Found"]).await;
        let error = fetch(&url, UNLIMITED, &CancellationToken::new(), &host).await;
        assert_eq!(error.unwrap_err().message, "status 404");
        assert_eq!(*host.0.lock().unwrap(), (1, 0));
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_refused() {
        let host = Metered::default();
        let fixture = fixture(file(), vec![Serve::Whole], false).await;
        let limits = Limits {
            max_bytes: 1000,
            deadline: None,
        };
        let error = fetch(&fixture.url, limits, &CancellationToken::new(), &host).await;
        assert_eq!(error.unwrap_err().message, "too large");
    }

    #[tokio::test]
    async fn cancelling_ends_a_download_that_waits_for_its_answer() {
        let host = Metered::default();
        let (url, _) = answering(vec![HANG]).await;
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.cancel();
        });
        let error = fetch(&url, UNLIMITED, &cancel, &host).await.unwrap_err();
        assert!(error.aborted);
        assert_eq!(error.message, ABORTED);
        assert_eq!(*host.0.lock().unwrap(), (1, 0));
    }
}
