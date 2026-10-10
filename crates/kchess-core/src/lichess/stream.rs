//! Bounded NDJSON line decoding (`crates/kchess-node/js/ndjson.ts`'s `readLines`): lines split across
//! chunks are reassembled, heartbeats (blank lines) are skipped, a line longer than the limit is
//! an error, a stream that stalls past its idle deadline is an error, and cancellation stops the
//! reader and drops the body.

use std::fmt::Display;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;

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

/// Read `body` to its end, calling `on_line` for every non-empty trimmed line as it arrives.
///
/// `on_line` may fail: the read stops at that line, so a malformed record surfaces when it
/// arrives rather than at the end of the stream (`readLines` rejected from `onLine` the same way).
/// `cancel` (when given) stops the read with an abort error; the body is dropped on every exit,
/// so a cancelled or failed read releases the connection.
pub async fn read_lines<S, B, E, F, Err>(
    body: S,
    cancel: Option<&CancellationToken>,
    options: &LineOptions,
    mut on_line: F,
) -> std::result::Result<(), Err>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: Display,
    F: FnMut(&str) -> std::result::Result<(), Err>,
    Err: From<CoreError>,
{
    let mut body = body;
    let mut tail: Vec<u8> = Vec::new();
    let max = options.max_line_bytes;
    loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(CoreError::aborted("This operation was aborted").into());
        }
        let next = match cancel {
            Some(token) => tokio::select! {
                biased;
                _ = token.cancelled() => {
                    return Err(CoreError::aborted("This operation was aborted").into());
                }
                answer = tokio::time::timeout(options.idle, body.next()) => answer,
            },
            None => tokio::time::timeout(options.idle, body.next()).await,
        };
        let chunk = match next {
            Err(_) => return Err(CoreError::new(STALLED).into()),
            Ok(None) => break,
            Ok(Some(Err(cause))) => return Err(CoreError::new(cause.to_string()).into()),
            Ok(Some(Ok(chunk))) => chunk,
        };
        tail.extend_from_slice(chunk.as_ref());
        let mut start = 0;
        while let Some(offset) = tail[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + offset;
            if end - start > max {
                return Err(CoreError::new(OVERSIZED).into());
            }
            emit(&tail[start..end], &mut on_line)?;
            start = end + 1;
        }
        if start > 0 {
            tail.drain(..start);
        }
        if tail.len() > max {
            return Err(CoreError::new(OVERSIZED).into());
        }
    }
    if !tail.is_empty() {
        emit(&tail, &mut on_line)?;
    }
    Ok(())
}

/// Read `body` as it arrives in chunks, handing each decoded piece to `on_chunk` (`readPgnStream`).
/// A multi-byte character split across chunks is held back until its rest arrives. There is no
/// idle deadline or line limit here: the caller bounds what it keeps. A cancelled read ends
/// quietly, as `readPgnStream` returned when its signal aborted; the caller checks its token.
pub async fn read_chunks<S, B, E, F, Err>(
    body: S,
    cancel: &CancellationToken,
    mut on_chunk: F,
) -> std::result::Result<(), Err>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: Display,
    F: FnMut(&str) -> std::result::Result<(), Err>,
    Err: From<CoreError>,
{
    let mut body = body;
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            answer = body.next() => answer,
        };
        let Some(item) = next else { break };
        let chunk = item.map_err(|cause| CoreError::new(cause.to_string()))?;
        pending.extend_from_slice(chunk.as_ref());
        let complete = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(cause) if cause.error_len().is_none() => cause.valid_up_to(),
            Err(_) => pending.len(),
        };
        if complete > 0 {
            let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
            pending.drain(..complete);
            on_chunk(&text)?;
        }
    }
    if !pending.is_empty() {
        let text = String::from_utf8_lossy(&pending).into_owned();
        on_chunk(&text)?;
    }
    Ok(())
}

/// One line: decoded as UTF-8 (invalid bytes become U+FFFD, as `TextDecoder` does), trimmed,
/// and passed on when it is not empty.
fn emit<F, Err>(line: &[u8], on_line: &mut F) -> std::result::Result<(), Err>
where
    F: FnMut(&str) -> std::result::Result<(), Err>,
{
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    on_line(trimmed)
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
            |line| {
                seen.push(line.to_string());
                Ok::<(), CoreError>(())
            },
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
        let error: CoreError = read_lines(body, None, &options, |_| Ok::<(), CoreError>(()))
            .await
            .unwrap_err();
        assert!(error.message.contains("oversized"));
    }

    #[tokio::test]
    async fn an_idle_stream_is_a_stall() {
        let body = futures_util::stream::pending::<std::result::Result<Vec<u8>, String>>();
        let options = LineOptions {
            max_line_bytes: MAX_LINE_BYTES,
            idle: Duration::from_millis(20),
        };
        let error: CoreError =
            read_lines(Box::pin(body), None, &options, |_| Ok::<(), CoreError>(()))
                .await
                .unwrap_err();
        assert_eq!(error.message, STALLED);
    }

    /// A malformed record fails the read when it arrives: the second chunk never comes, yet the
    /// first line's error still ends the read (`readLines` rejected from `onLine` the same way).
    #[tokio::test]
    async fn a_failed_line_ends_the_read_before_the_stream_does() {
        let first: std::result::Result<Vec<u8>, String> = Ok(b"bad\n".to_vec());
        let body = futures_util::stream::iter(vec![first]).chain(futures_util::stream::pending());
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            read_lines(Box::pin(body), None, &LineOptions::default(), |line| {
                Err::<(), CoreError>(CoreError::new(format!("unreadable: {line}")))
            }),
        )
        .await
        .expect("the failed line must end the read without waiting for the stream");
        assert_eq!(outcome.unwrap_err().message, "unreadable: bad");
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
        let error: CoreError = read_lines(
            Box::pin(body),
            Some(&token),
            &LineOptions::default(),
            |_| Ok::<(), CoreError>(()),
        )
        .await
        .unwrap_err();
        assert!(error.aborted);
    }
}
