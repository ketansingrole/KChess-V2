//! Errors as they cross to the host: a message, and whether it was a cancellation
//! (`AbortError` on the JavaScript side).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub message: String,
    pub aborted: bool,
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> CoreError {
        CoreError {
            message: message.into(),
            aborted: false,
        }
    }

    pub fn aborted(message: impl Into<String>) -> CoreError {
        CoreError {
            message: message.into(),
            aborted: true,
        }
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
