//! Typed errors for the shared rules. The string boundary (`api::call`) surfaces them as
//! strings; internals use [`DomainError`] so callers can match instead of comparing message text.

/// Failures the shared rules can report.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DomainError {
    /// Back-compat message preserved from the previous `Result<T, String>` API.
    #[error("{0}")]
    Message(String),
    /// JSON (de)serialization failed.
    #[error("serialization: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for DomainError {
    fn from(e: serde_json::Error) -> Self {
        DomainError::Serialization(e.to_string())
    }
}

impl From<DomainError> for String {
    fn from(e: DomainError) -> String {
        e.to_string()
    }
}

/// The result type for fallible rules in this crate.
pub type Result<T> = std::result::Result<T, DomainError>;
