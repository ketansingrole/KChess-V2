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

/// The contracts both sides are built from: every method and event, with their types.
const CONTRACTS: [&str; 4] = [
    include_str!("../../kchess-contracts/src/core_api.rs"),
    include_str!("../../kchess-contracts/src/types.rs"),
    include_str!("../../kchess-contracts/src/library.rs"),
    include_str!("../../kchess-contracts/src/lichess.rs"),
];

/// What a client can do for the core when it asks (`host:request`), by name.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    Browser,
    Focus,
    LegacySecrets,
}

impl From<Capability> for kchess_core::connection::Capability {
    fn from(capability: Capability) -> Self {
        match capability {
            Capability::Browser => kchess_core::connection::Capability::Browser,
            Capability::Focus => kchess_core::connection::Capability::Focus,
            Capability::LegacySecrets => kchess_core::connection::Capability::LegacySecrets,
        }
    }
}

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
    },
    /// Stop a call in flight; its result is an error.
    Cancel { id: u64 },
    /// A core event (`CoreEvents` names, and `host:request`).
    Event { name: String, payload: Value },
    /// The client's window gained or lost focus.
    Focus { focused: bool },
}

/// The KChess release this build belongs to: the workspace version, or `KCHESS_PACKAGE_VERSION`
/// when a test installer is built above every release (`build.rs`).
pub fn release() -> String {
    env!("KCHESS_RELEASE").to_string()
}

/// The protocol fingerprint: the framing and the contracts this build speaks. Two builds share a
/// core only when theirs match.
pub fn protocol() -> String {
    let mut hash = Sha256::new();
    hash.update(FRAMING.as_bytes());
    for contract in CONTRACTS {
        hash.update(contract.as_bytes());
    }
    hex(&hash.finalize()[..8])
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Reads one frame; `None` when the other side closed between frames.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Frame>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME {
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
    let body = serde_json::to_vec(frame)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("a frame of {} bytes is over the limit", body.len()),
        ));
    }
    let length = u32::try_from(body.len())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&body).await?;
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
            capabilities: vec![Capability::Browser, Capability::LegacySecrets],
        };
        let text = serde_json::to_value(&hello).expect("json");
        assert_eq!(text["type"], json!("hello"));
        assert_eq!(text["capabilities"], json!(["browser", "legacySecrets"]));
        let result = Frame::Result {
            id: 1,
            value: None,
            error: Some("No.".into()),
        };
        assert_eq!(
            serde_json::to_value(&result).expect("json"),
            json!({ "type": "result", "id": 1, "error": "No." })
        );
    }

    #[test]
    fn the_release_and_protocol_are_stable() {
        assert!(!release().is_empty());
        assert_eq!(protocol(), protocol());
        assert_eq!(protocol().len(), 16);
    }
}
