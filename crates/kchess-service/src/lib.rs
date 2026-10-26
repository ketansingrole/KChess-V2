//! The shared KChess core (`docs/shared-core.md`): one core per profile, which every KChess
//! window, and later a CLI or MCP server, uses over a private local socket.
//!
//! - `protocol`: the frames, the release and the protocol fingerprint;
//! - `discovery`: the socket, `core.json`, `core.token` and the runtime folder;
//! - `transport`: Unix sockets and Windows named pipes;
//! - `server`: the service (`kchess-service serve`);
//! - `client`: connecting, and starting the service when none runs;
//! - `runtime`: the per-release copy the service runs from.

pub mod client;
pub mod discovery;
pub mod protocol;
pub mod runtime;
pub mod server;
pub mod transport;

pub use client::{
    Client, ClientEvent, ConnectError, Hello, ServiceConfig, Start, connect_or_start,
};
pub use protocol::Capability;
