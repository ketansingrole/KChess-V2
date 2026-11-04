//! Errors as they cross to the host: a message, and whether it was a cancellation
//! (`AbortError` on the JavaScript side).

use std::fmt;

/// What kind of failure an error is, so a frontend offers the action that can help (a retry, a
/// reconnect, nothing) without reading its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Lichess could not be reached.
    Offline,
    /// Lichess did not answer in time.
    TimedOut,
    /// Lichess asked for fewer requests for a while.
    RateLimited,
    /// The account's login no longer works or lacks a permission: reconnect it.
    Login,
    /// Lichess refused the request for a reason it gave; trying again changes nothing.
    Refused,
    /// What was asked for does not exist (any more).
    NotFound,
    /// Lichess failed or answered something unexpected.
    Unavailable,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Offline => "offline",
            ErrorKind::TimedOut => "timedOut",
            ErrorKind::RateLimited => "rateLimited",
            ErrorKind::Login => "login",
            ErrorKind::Refused => "refused",
            ErrorKind::NotFound => "notFound",
            ErrorKind::Unavailable => "unavailable",
        }
    }

    pub fn parse(text: &str) -> Option<ErrorKind> {
        [
            ErrorKind::Offline,
            ErrorKind::TimedOut,
            ErrorKind::RateLimited,
            ErrorKind::Login,
            ErrorKind::Refused,
            ErrorKind::NotFound,
            ErrorKind::Unavailable,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == text)
    }

    /// Whether the same request may work if made again later.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::Offline
                | ErrorKind::TimedOut
                | ErrorKind::RateLimited
                | ErrorKind::Unavailable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    /// What a person is told: plain words, never a status, an endpoint or a response body.
    pub message: String,
    pub aborted: bool,
    pub kind: Option<ErrorKind>,
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> CoreError {
        CoreError {
            message: message.into(),
            aborted: false,
            kind: None,
        }
    }

    pub fn aborted(message: impl Into<String>) -> CoreError {
        CoreError {
            message: message.into(),
            aborted: true,
            kind: None,
        }
    }

    #[must_use]
    pub fn with_kind(mut self, kind: ErrorKind) -> CoreError {
        self.kind = Some(kind);
        self
    }

    /// A low-level failure with what was being attempted: `"{message}: {err}"`.
    /// Use it at `map_err` sites so host logs name the operation, not just the cause.
    pub fn wrap(message: impl Into<String>, err: impl std::fmt::Display) -> CoreError {
        CoreError::new(format!("{}: {err}", message.into()))
    }

    /// This error with a failed `ROLLBACK` appended, so the host log that reports the error
    /// also shows the connection may still be inside the transaction.
    #[must_use]
    pub fn with_rollback_failure(mut self, rollback: impl std::fmt::Display) -> CoreError {
        self.message = format!("{}; rollback failed: {rollback}", self.message);
        self
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CoreError {}

impl From<String> for CoreError {
    fn from(message: String) -> CoreError {
        CoreError::new(message)
    }
}

impl From<&str> for CoreError {
    fn from(message: &str) -> CoreError {
        CoreError::new(message)
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
