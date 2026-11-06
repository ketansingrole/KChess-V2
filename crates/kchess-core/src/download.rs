//! Large downloads that survive a dropped connection: `Resumable` reads a body as
//! `reqwest::Response::chunk` does, and when the connection fails partway it asks again from the
//! byte it reached (`Range`), so the bytes already received are not fetched twice.
//!
//! It resumes only when that is safe: the first answer was a whole `200` that named its length,
//! offered byte ranges and gave a validator (`ETag`, or `Last-Modified`), and the new answer is a
//! `206` for exactly the rest of that same file (`If-Range`, `Content-Range`). Anything else
//! ends the read with the failure the connection had, as it did before resuming existed.

use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap, IF_RANGE, LAST_MODIFIED, RANGE,
};

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
}
