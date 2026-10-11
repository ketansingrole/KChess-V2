//! Typed errors for the shared rules. Public boundaries that cross FFI
//! (`api::call`, the Node/WASM wrappers) still surface strings; internals use
//! [`DomainError`] so callers can match instead of comparing message text.

/// Failures the shared rules can report.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DomainError {
    /// Back-compat message preserved from the previous `Result<T, String>` API.
    #[error("{0}")]
    Message(String),
    /// A malformed argument at a named position.
    #[error("invalid argument {name}: {reason}")]
    InvalidArgument { name: String, reason: String },
    /// A `{ variant, fen }` setup that is not a legal position.
    #[error("invalid setup: {0}")]
    InvalidSetup(String),
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
