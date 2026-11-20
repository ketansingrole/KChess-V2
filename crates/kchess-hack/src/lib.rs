//! Empty: the package only depends on what every workspace package should build with one
//! feature set (`.config/hakari.toml`).

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
