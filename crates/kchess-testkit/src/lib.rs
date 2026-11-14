//! A loopback HTTP/1.1 server for the tests' stand-ins (Lichess, Chess.com) and xtask's
//! benchmark Lichess: it reads each request, hands it to the stand-in's handler on a thread of
//! its own, and gives the handler the pieces of a response. Every response closes its
//! connection (`Connection: close`).
//!
//! The core's async fixture (`crates/kchess-core/tests/support/lichess_http.rs`) reads with tokio
//! and shares the parsing and the response heads (`Request::from_head`, `sized_head`,
//! `chunked_head`, `chunk`).

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a connection whose response is written waits for its client to hang up, as the
/// core's own servers do (`kchess_core::loopback::close_after_client`): a server that closes
/// first loses its response on some Windows PCs, whose 127.0.0.1 filter turns the close into a
/// reset.
pub const HANG_UP_WAIT: Duration = Duration::from_secs(10);

/// One request, as the client sent it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path and its query, still percent-encoded.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    /// The request whose head (request line and headers, without the blank line) is `head`;
    /// its body is empty.
    pub fn from_head(head: &str) -> Option<Request> {
        let mut lines = head.lines();
        let mut parts = lines.next()?.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .collect();
        Some(Request {
            method,
            target,
            headers,
            body: String::new(),
        })
    }

    /// Reads one request: its head, then the `Content-Length` bytes of its body. `None` when the
    /// client hung up before a whole request.
    pub fn read(reader: &mut impl BufRead) -> Option<Request> {
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).ok()? == 0 {
                return None;
            }
            if line.trim_end().is_empty() {
                break;
            }
            head.push_str(&line);
        }
        let mut request = Request::from_head(&head)?;
        let mut body = vec![0; request.content_length()];
        reader.read_exact(&mut body).ok()?;
        request.body = String::from_utf8_lossy(&body).into_owned();
        Some(request)
    }

    /// The target without its query.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    /// The query, still percent-encoded; empty when there is none.
    pub fn query_string(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, query)| query)
    }

    /// The query parameter `name`, still percent-encoded.
    pub fn query(&self, name: &str) -> Option<String> {
        self.query_string().split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| value.to_string())
        })
    }

    /// The header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body's length as the head gives it; 0 without one.
    pub fn content_length(&self) -> usize {
        self.header("content-length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }
}

/// The reason phrase of `status`.
pub fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
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

/// The head of a response whose body is `length` bytes long.
pub fn sized_head(status: u16, headers: &[(&str, &str)], length: usize) -> String {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", status_text(status));
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {length}\r\nConnection: close\r\n\r\n"
    ));
    head
}

/// The head of a response sent in chunks (`chunk`) until an empty one.
pub fn chunked_head(status: u16, content_type: &str) -> String {
    format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        status_text(status)
    )
}

/// One chunk of a chunked body; an empty `text` ends the body.
pub fn chunk(text: &str) -> String {
    format!("{:x}\r\n{text}\r\n", text.len())
}

/// Writes a whole response, then waits for the client to hang up (`answered`).
pub fn respond(
    out: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    out.write_all(sized_head(status, &[("Content-Type", content_type)], body.len()).as_bytes())?;
    out.write_all(body)?;
    answered(out)
}

/// A JSON response.
pub fn json(out: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    respond(out, status, "application/json", body.to_string().as_bytes())
}

/// An NDJSON response of `lines`, one per line.
pub fn ndjson(out: &mut TcpStream, lines: &[Value]) -> std::io::Result<()> {
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    respond(out, 200, "application/x-ndjson", body.as_bytes())
}

/// A response is written in full: the client hangs up first (see `HANG_UP_WAIT`).
pub fn answered(out: &mut TcpStream) -> std::io::Result<()> {
    out.flush()?;
    out.set_read_timeout(Some(HANG_UP_WAIT))?;
    std::io::copy(out, &mut std::io::sink()).map(|_| ())
}

/// An NDJSON stream: `first`'s lines at once, then `next(sent)`'s whenever it has one (`sent`
/// counts the lines so far), a blank keep-alive line after `keep_alive` without one, until the
/// client hangs up or `open` says the stream is over.
pub fn ndjson_stream(
    out: &mut TcpStream,
    first: Vec<Value>,
    keep_alive: Duration,
    open: impl Fn() -> bool,
    mut next: impl FnMut(usize) -> Option<Value>,
) -> std::io::Result<()> {
    out.write_all(chunked_head(200, "application/x-ndjson").as_bytes())?;
    let mut sent = 0;
    for line in first {
        out.write_all(chunk(&format!("{line}\n")).as_bytes())?;
        sent += 1;
    }
    out.flush()?;
    let mut quiet = Instant::now();
    // A hang-up is seen at once, not at the next keep-alive: the client does not send on a
    // stream, so a read that ends means it has gone.
    out.set_read_timeout(Some(Duration::from_millis(1)))?;
    while open() {
        if matches!(out.peek(&mut [0u8; 1]), Ok(0)) {
            return Ok(());
        }
        if let Some(line) = next(sent) {
            out.write_all(chunk(&format!("{line}\n")).as_bytes())?;
            out.flush()?;
            sent += 1;
            quiet = Instant::now();
        } else if quiet.elapsed() >= keep_alive {
            out.write_all(chunk("\n").as_bytes())?;
            out.flush()?;
            quiet = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    out.write_all(chunk("").as_bytes())?;
    answered(out)
}

/// What a server does with each request: answers it on `out`.
pub type Handler = dyn Fn(Request, &mut TcpStream) -> std::io::Result<()> + Send + Sync;

/// A server on a free 127.0.0.1 port, each connection on a thread of its own, until it is
/// dropped.
pub struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(
        handler: impl Fn(Request, &mut TcpStream) -> std::io::Result<()> + Send + Sync + 'static,
    ) -> std::io::Result<Server> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Arc<Handler> = Arc::new(handler);
        let accept = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for socket in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(socket) = socket else { continue };
                    let handler = Arc::clone(&handler);
                    std::thread::spawn(move || {
                        let _ = serve(socket, &*handler);
                    });
                }
            })
        };
        Ok(Server {
            base,
            stop,
            accept: Some(accept),
        })
    }

    /// The origin clients are pointed at: `http://127.0.0.1:<port>`.
    pub fn base(&self) -> &str {
        &self.base
    }
}

fn serve(socket: TcpStream, handler: &Handler) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let Some(request) = Request::read(&mut reader) else {
        return Ok(());
    };
    let mut out = socket;
    let answer = handler(request, &mut out);
    let _ = out.shutdown(Shutdown::Write);
    answer
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees the stop.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

#[cfg(test)]
mod tests;
