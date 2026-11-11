//! The shared KChess core (`docs/shared-core.md`): one core per profile, which every KChess
//! window, and later a CLI or MCP server, uses over a private local socket.
//!
//! - `protocol`: the frames, the release and the protocol fingerprint;
//! - `discovery`: the socket, `core.json`, `core.token` and the runtime folder;
//! - `transport`: Unix sockets and Windows named pipes;
//! - `server`: the service (`kchess-service serve`);
//! - `client`: connecting, and starting the service when none runs;
//! - `handle`: a frontend's core, embedded or shared, with its events and host requests;
//! - `layout`: the profile and the programs a frontend configures the core with;
//! - `runtime`: the per-release copy the service runs from.

pub mod client;
pub mod discovery;
pub mod handle;
pub mod layout;
pub mod protocol;
pub mod runtime;
pub mod server;
pub mod transport;

pub use client::{
    Client, ClientEvent, ConnectError, Hello, Start, connect_or_start, connect_or_start_within,
};
pub use handle::{Answer, CORE_STACK, CoreHandle, Log, Mode, Options, core_runtime};
pub use layout::Layout;
pub use protocol::Capability;
