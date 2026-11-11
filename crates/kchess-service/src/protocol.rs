//! The frames clients and the service exchange (`docs/shared-core.md`, "Frames"): a 4-byte
//! big-endian length, then that many bytes of UTF-8 JSON. A client sends `hello` first; the
//! service answers `welcome` or `refuse`. After that, calls, results, cancellations, events and
//! focus reports flow until either side closes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// No frame is larger: library documents are bounded far below it.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Raised when the framing itself changes; part of the protocol fingerprint.
const FRAMING: &str = "kchess-frames-1";

pub use kchess_contracts::Capability;

/// An app connected to a core, as `refuse` describes it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientSummary {
    pub kind: String,
    pub release: String,
}

/// Why a client was not let in.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RefuseReason {
    /// Another KChess release runs this profile's core.
    Version,
    /// The token or the user did not match.
    Unauthorized,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Frame {
    /// The client's first frame.
    #[serde(rename_all = "camelCase")]
    Hello {
        token: String,
        kind: String,
        release: String,
        protocol: String,
        capabilities: Vec<Capability>,
    },
    /// The service let the client in, as `connection`.
    #[serde(rename_all = "camelCase")]
    Welcome {
        connection: u64,
        release: String,
        protocol: String,
    },
    /// The service did not let the client in. `sole`: no other client was connected, so the
    /// service is closing and the client may start its own release's core.
    #[serde(rename_all = "camelCase")]
    Refuse {
        reason: RefuseReason,
        message: String,
        sole: bool,
        clients: Vec<ClientSummary>,
    },
    /// Run a core method (`Core::call_as`).
    Call {
        id: u64,
        method: String,
        args: Vec<Value>,
    },
    /// A call's answer: `value`, or the core's error message.
    Result {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// The error's `ErrorKind`, so the client picks its action without reading the message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
    /// Stop a call in flight; its result is an error.
    Cancel { id: u64 },
    /// A core event (a `CoreEvent` by its name).
    Event { name: String, payload: Value },
    /// The client's window gained or lost focus.
    Focus { focused: bool },
    /// The client has no window open: what it started stops, and the core serves on.
    Suspend,
}

/// The KChess release this build belongs to: the workspace version, or `KCHESS_PACKAGE_VERSION`
/// when a test installer is built above every release (`build.rs`).
pub fn release() -> String {
    env!("KCHESS_RELEASE").to_string()
}

/// The protocol fingerprint: the framing, and the methods, events and error kinds this build
/// speaks. Two builds share a core only when theirs match.
pub fn protocol() -> String {
    let mut hash = Sha256::new();
    let mut add = |part: &str| {
        hash.update(part.as_bytes());
        hash.update([0]);
    };
    add(FRAMING);
    for method in kchess_contracts::core_api::METHODS {
        add(method.name);
        for param in method.params {
            add(param.name);
            add(if param.optional { "?" } else { "" });
        }
        add(")");
    }
    for name in kchess_contracts::CoreEvent::NAMES {
        add(name);
    }
    for kind in kchess_contracts::ErrorKind::ALL {
        add(kind.as_str());
    }
    kchess_core::ids::hex(&hash.finalize()[..8])
}

/// Reads one frame; `None` when the other side closed between frames.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Frame>> {
    read_frame_within(reader, MAX_FRAME).await
}

/// The largest `hello`: read before the token is checked, so a client that has not proven itself
/// can make the service allocate no more than this.
pub const MAX_HELLO: usize = 64 * 1024;

/// `read_frame`, refusing a frame over `limit` bytes before reading it.
pub async fn read_frame_within<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<Frame>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length) as usize;
    if length > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("a frame of {length} bytes is over the limit"),
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// Writes one frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &Frame,
) -> std::io::Result<()> {
    // The length prefix and the body go out in one write.
    let mut bytes = vec![0u8; 4];
    serde_json::to_writer(&mut bytes, frame)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let body = bytes.len() - 4;
    if body > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("a frame of {body} bytes is over the limit"),
        ));
    }
    let length = u32::try_from(body)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    bytes[..4].copy_from_slice(&length.to_be_bytes());
    writer.write_all(&bytes).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn frames_round_trip_and_oversized_ones_are_refused() {
        let (mut client, mut service) = tokio::io::duplex(1024);
        let call = Frame::Call {
            id: 7,
            method: "library".into(),
            args: vec![json!(1)],
        };
        write_frame(&mut client, &call).await.expect("written");
        assert_eq!(read_frame(&mut service).await.expect("read"), Some(call));

        let mut oversized = (MAX_FRAME as u32 + 1).to_be_bytes().to_vec();
        oversized.extend_from_slice(b"{}");
        tokio::io::AsyncWriteExt::write_all(&mut client, &oversized)
            .await
            .expect("written");
        assert!(read_frame(&mut service).await.is_err());
    }

    #[tokio::test]
    async fn a_closed_connection_reads_as_none() {
        let (client, mut service) = tokio::io::duplex(64);
        drop(client);
        assert_eq!(read_frame(&mut service).await.expect("read"), None);
    }

    #[test]
    fn frames_are_tagged_json() {
        let hello = Frame::Hello {
            token: "t".into(),
            kind: "gpui".into(),
            release: "2026.10.12".into(),
            protocol: protocol(),
            capabilities: vec![Capability::Browser, Capability::Focus],
        };
        let text = serde_json::to_value(&hello).expect("json");
        assert_eq!(text["type"], json!("hello"));
        assert_eq!(text["capabilities"], json!(["browser", "focus"]));
        let result = Frame::Result {
            id: 1,
            value: None,
            error: Some("No.".into()),
            kind: None,
        };
        assert_eq!(
            serde_json::to_value(&result).expect("json"),
            json!({ "type": "result", "id": 1, "error": "No." })
        );
    }

    /// A page chooses its action from the failure's kind, so the kind crosses the service.
    #[test]
    fn a_failure_carries_its_kind() {
        let refused = Frame::Result {
            id: 2,
            value: None,
            error: Some("This game cannot be played with the Board API.".into()),
            kind: Some("refused".into()),
        };
        let text = serde_json::to_string(&refused).expect("json");
        let Frame::Result { kind, .. } = serde_json::from_str(&text).expect("frame") else {
            panic!("a result");
        };
        assert_eq!(kind.as_deref(), Some("refused"));
        // A frame from a service without kinds still reads.
        let old: Frame =
            serde_json::from_value(json!({ "type": "result", "id": 3, "error": "No." }))
                .expect("frame");
        assert!(matches!(old, Frame::Result { kind: None, .. }));
    }

    /// A frame whose body is exactly `size` bytes.
    fn event_of(size: usize) -> Frame {
        let empty = Frame::Event {
            name: "e".into(),
            payload: json!(""),
        };
        let overhead = serde_json::to_vec(&empty).expect("json").len();
        Frame::Event {
            name: "e".into(),
            payload: json!("a".repeat(size - overhead)),
        }
    }

    /// Frames up to the limit go out, with a length the reader follows to the next frame.
    #[tokio::test]
    async fn frames_up_to_the_limit_are_written_and_read_back_in_turn() {
        let largest = event_of(MAX_FRAME);
        let small = Frame::Focus { focused: true };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &largest)
            .await
            .expect("the largest frame");
        write_frame(&mut bytes, &small)
            .await
            .expect("a small frame");
        let mut reader = bytes.as_slice();
        let first = read_frame(&mut reader).await.expect("read");
        assert!(first == Some(largest), "the largest frame reads back");
        assert_eq!(read_frame(&mut reader).await.expect("read"), Some(small));
        assert_eq!(read_frame(&mut reader).await.expect("read"), None);

        let over = write_frame(&mut tokio::io::sink(), &event_of(MAX_FRAME + 1)).await;
        assert!(over.is_err_and(|error| error.to_string().contains("over the limit")));
    }

    /// Library documents of several megabytes fit in a frame.
    #[test]
    fn the_limit_is_sixteen_megabytes() {
        assert_eq!(MAX_FRAME, 16 << 20);
    }

    /// A read that fails is an error, not a closed connection.
    #[tokio::test]
    async fn a_broken_connection_is_an_error() {
        struct Broken;
        impl AsyncRead for Broken {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()))
            }
        }
        assert!(read_frame(&mut Broken).await.is_err());
    }

    #[test]
    fn the_release_and_protocol_are_stable() {
        let packaged = std::env::var("KCHESS_PACKAGE_VERSION").unwrap_or_default();
        if packaged.is_empty() {
            assert_eq!(release(), env!("CARGO_PKG_VERSION"));
        }
        assert!(!release().is_empty());
        assert_eq!(protocol(), protocol());
        assert_eq!(protocol().len(), 16);
    }
}
