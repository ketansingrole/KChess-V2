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
