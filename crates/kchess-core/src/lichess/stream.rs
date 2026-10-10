//! Bounded NDJSON line decoding (`core/src/services/ndjson.ts`'s `readLines`): lines split across
//! chunks are reassembled, heartbeats (blank lines) are skipped, a line longer than the limit is
//! an error, a stream that stalls past its idle deadline is an error, and cancellation stops the
//! reader and drops the body.

use std::fmt::Display;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, Result};

/// The default limit for one line (`maxLineBytes`), in bytes.
pub const MAX_LINE_BYTES: usize = 2_000_000;
/// The default idle deadline between chunks (`idleMs`).
pub const IDLE: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
pub struct LineOptions {
    /// Longest line accepted, measured in bytes from its first byte.
    pub max_line_bytes: usize,
    /// How long the stream may stay silent before it counts as stalled.
    pub idle: Duration,
}

impl Default for LineOptions {
    fn default() -> LineOptions {
        LineOptions {
            max_line_bytes: MAX_LINE_BYTES,
            idle: IDLE,
        }
    }
}

const OVERSIZED: &str = "Lichess sent an oversized stream record.";
const STALLED: &str = "Lichess stream stalled. Reconnecting…";

/// Read `body` to its end, calling `on_line` for every non-empty trimmed line.
///
/// `cancel` (when given) stops the read with an abort error; the body is dropped on every exit,
/// so a cancelled or failed read releases the connection.
pub async fn read_lines<S, B, E, F>(
    body: S,
    cancel: Option<&CancellationToken>,
    options: &LineOptions,
    mut on_line: F,
) -> Result<()>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: Display,
    F: FnMut(&str),
{
    let mut body = body;
    let mut tail: Vec<u8> = Vec::new();
    let max = options.max_line_bytes;
    loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(CoreError::aborted("This operation was aborted"));
        }
        let next = match cancel {
            Some(token) => tokio::select! {
                biased;
                _ = token.cancelled() => {
                    return Err(CoreError::aborted("This operation was aborted"));
                }
                answer = tokio::time::timeout(options.idle, body.next()) => answer,
            },
            None => tokio::time::timeout(options.idle, body.next()).await,
        };
        let chunk = match next {
            Err(_) => return Err(CoreError::new(STALLED)),
            Ok(None) => break,
            Ok(Some(Err(cause))) => return Err(CoreError::new(cause.to_string())),
            Ok(Some(Ok(chunk))) => chunk,
        };
        tail.extend_from_slice(chunk.as_ref());
        let mut start = 0;
        while let Some(offset) = tail[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + offset;
            if end - start > max {
                return Err(CoreError::new(OVERSIZED));
            }
            emit(&tail[start..end], &mut on_line);
            start = end + 1;
        }
        if start > 0 {
            tail.drain(..start);
        }
        if tail.len() > max {
            return Err(CoreError::new(OVERSIZED));
        }
    }
    if !tail.is_empty() {
        emit(&tail, &mut on_line);
    }
    Ok(())
}

/// One line: decoded as UTF-8 (invalid bytes become U+FFFD, as `TextDecoder` does), trimmed,
/// and passed on when it is not empty.
fn emit<F: FnMut(&str)>(line: &[u8], on_line: &mut F) {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        on_line(trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_of(
        chunks: Vec<&'static [u8]>,
    ) -> impl Stream<Item = std::result::Result<Vec<u8>, String>> + Unpin {
        futures_util::stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok(chunk.to_vec()))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn reassembles_lines_split_across_chunks() {
        let mut seen = Vec::new();
        read_lines(
            stream_of(vec![b"{\"a\"", b":1}\n{\"b\":2}\npart", b"ial\n"]),
            None,
            &LineOptions::default(),
            |line| seen.push(line.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(seen, vec![r#"{"a":1}"#, r#"{"b":2}"#, "partial"]);
    }

    #[tokio::test]
    async fn rejects_an_oversized_line_measured_from_its_start() {
        let body = stream_of(vec![b"xxxxxxxxxxxxxxxxxxxx\n"]);
        let options = LineOptions {
            max_line_bytes: 10,
            idle: IDLE,
        };
        let error = read_lines(body, None, &options, |_| {}).await.unwrap_err();
        assert!(error.message.contains("oversized"));
    }

    #[tokio::test]
    async fn an_idle_stream_is_a_stall() {
        let body = futures_util::stream::pending::<std::result::Result<Vec<u8>, String>>();
        let options = LineOptions {
            max_line_bytes: MAX_LINE_BYTES,
            idle: Duration::from_millis(20),
        };
        let error = read_lines(Box::pin(body), None, &options, |_| {})
            .await
            .unwrap_err();
        assert_eq!(error.message, STALLED);
    }

    #[tokio::test]
    async fn cancellation_ends_a_waiting_read_with_an_abort() {
        let token = CancellationToken::new();
        let canceller = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            canceller.cancel();
        });
        let body = futures_util::stream::pending::<std::result::Result<Vec<u8>, String>>();
        let error = read_lines(
            Box::pin(body),
            Some(&token),
            &LineOptions::default(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.aborted);
    }
}
