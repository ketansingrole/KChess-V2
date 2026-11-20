//! The profile's core as a frontend holds it (`docs/shared-core.md`), without the core itself:
//! the crates a frontend is built from depend on this one, so a change to the core rebuilds only
//! the programs that host it (the app, the command line, the service).
//!
//! - `protocol`: the frames, the release and the protocol fingerprint;
//! - `discovery`: the socket, `core.json`, `core.token` and the runtime folder;
//! - `transport`: Unix sockets and Windows named pipes;
//! - `client`: connecting, and starting the service when none runs;
//! - `handle`: a frontend's core, shared or embedded (by its host: `kchess_service::embed`), with
//!   its events and host requests;
//! - `layout`: the profile and the programs a frontend configures the core with;
//! - `runtime`: the per-release copy the service runs from.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod client;
pub mod discovery;
pub mod handle;
pub mod layout;
pub mod protocol;
pub mod runtime;
pub mod transport;

pub use client::{
    Client, ClientEvent, ConnectError, Hello, Start, connect_or_start, connect_or_start_within,
};
pub use handle::{
    Answer, BoxFuture, CORE_STACK, CoreHandle, EMBEDDED_CONNECTION, Embed, EmbeddedCore, Embedding,
    Log, Mode, Options, core_runtime,
};
pub use layout::Layout;
pub use protocol::Capability;
