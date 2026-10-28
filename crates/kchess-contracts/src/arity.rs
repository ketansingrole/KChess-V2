//! The argument arities of the core's methods and of the desktop's IPC invocations, and the
//! methods refused during a live human game. The checks themselves run in
//! `kchess_domain::misc::contracts`; this re-exports what callers of the contracts need.

pub use kchess_domain::misc::{ASSISTANCE_METHODS, Scope, arities};
