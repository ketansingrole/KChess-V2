//! What the core and its frontends share about failures and logging: a core error as it crosses
//! to a frontend (a message, whether it was a cancellation, and its kind), and the level of a log
//! line. Frontends that only talk to the core use these without depending on `kchess-core`.

use std::fmt;

/// How serious a log line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

pub use kchess_domain::misc::constants::ErrorKind;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_has_its_name() {
        assert_eq!(Level::Debug.name(), "debug");
        assert_eq!(Level::Info.name(), "info");
        assert_eq!(Level::Warn.name(), "warn");
        assert_eq!(Level::Error.name(), "error");
    }

    #[test]
    fn each_kind_reads_back_from_its_name() {
        let names: Vec<&str> = ErrorKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(
            names,
            [
                "offline",
                "timedOut",
                "rateLimited",
                "login",
                "refused",
                "notFound",
                "unavailable",
                "noEngine",
                "conflict"
            ]
        );
        for &kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::from_key(kind.as_str()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
        assert_eq!(ErrorKind::from_key("OFFLINE"), None);
        assert_eq!(ErrorKind::from_key(""), None);
    }

    #[test]
    fn an_error_carries_its_message_cancellation_and_kind() {
        let plain = CoreError::new("No game");
        assert_eq!(plain.message, "No game");
        assert!(!plain.aborted);
        assert_eq!(plain.kind, None);

        let aborted = CoreError::aborted("Stopped");
        assert_eq!(aborted.message, "Stopped");
        assert!(aborted.aborted);
        assert_eq!(aborted.kind, None);

        let kinded = CoreError::new("Lichess is busy").with_kind(ErrorKind::RateLimited);
        assert_eq!(kinded.kind, Some(ErrorKind::RateLimited));
        assert_eq!(kinded.message, "Lichess is busy");
    }

    #[test]
    fn wrapping_names_the_operation_then_the_cause() {
        let error = CoreError::wrap("Could not open the database", "disk full");
        assert_eq!(error.message, "Could not open the database: disk full");
        assert!(!error.aborted);
        let rolled = error.with_rollback_failure("busy");
        assert_eq!(
            rolled.message,
            "Could not open the database: disk full; rollback failed: busy"
        );
    }

    #[test]
    fn an_error_displays_as_its_message_and_comes_from_text() {
        let error = CoreError::new("Not found").with_kind(ErrorKind::NotFound);
        assert_eq!(error.to_string(), "Not found");
        assert_eq!(CoreError::from("text"), CoreError::new("text"));
        assert_eq!(
            CoreError::from(String::from("owned")),
            CoreError::new("owned")
        );
        let boxed: Box<dyn std::error::Error> = Box::new(CoreError::new("boxed"));
        assert_eq!(boxed.to_string(), "boxed");
    }
}
