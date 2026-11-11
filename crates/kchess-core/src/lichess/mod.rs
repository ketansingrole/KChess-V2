//! Lichess: accounts, online play, challenges, tournaments, studies, spectating and lookups.

pub mod accounts;
pub mod adapters;
pub mod challenges;
pub mod client;
pub mod decode;
pub mod lookups;
pub mod oauth;
pub mod online;
pub mod policy;
pub mod puzzles;
pub mod reviews;
pub mod services;
pub mod social;
pub mod stream;
pub mod studies;
pub mod tournaments;
pub mod watch;

/// Whether `text` is a Lichess id of a game, study, chapter or puzzle: eight ASCII letters or
/// digits.
pub fn is_lichess_id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The error of a request or stream that was cancelled.
pub(crate) fn aborted() -> crate::error::CoreError {
    crate::error::CoreError::aborted("This operation was aborted")
}
