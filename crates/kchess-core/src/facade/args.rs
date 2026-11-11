use kchess_domain::misc::value::J;
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use crate::lichess::accounts::Reply;

/// `Date.now()`.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// A normalized string value; anything else is empty.
pub(crate) fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// `value` as `check` (`kchess_domain::misc::checks`) normalizes it.
pub(crate) fn normal(
    check: fn(J) -> std::result::Result<Value, String>,
    value: &Value,
) -> Result<Value> {
    check(J::from(value)).map_err(CoreError::new)
}

/// An argument the check passed, read as the service takes it.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(value: Value, name: &str) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|cause| CoreError::new(format!("Invalid {name}: {cause}")))
}

/// A whole number the check passed, as a service takes it.
pub(crate) fn count(value: &Value, at: usize) -> Result<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| CoreError::new(format!("Invalid argument {at}.")))
}

/// A service's answer as JSON.
pub(crate) fn to_json<T: serde::Serialize>(
    result: std::result::Result<T, impl Into<CoreError>>,
) -> Result<Value> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string())),
        Err(cause) => Err(cause.into()),
    }
}

/// A service's answer as a caller reads it: the value, or `{ needsReconnect: true }` when the
/// account's login lacks the permission.
pub(crate) fn reply_json<T: serde::Serialize, E: Into<CoreError>>(
    result: std::result::Result<Reply<T>, E>,
) -> Result<Value> {
    match result.map_err(Into::into)? {
        Reply::Done(value) => {
            serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string()))
        }
        Reply::NeedsReconnect => Ok(json!({ "needsReconnect": true })),
    }
}

/// An action's answer: `true` when it went through, or `{ needsReconnect: true }`.
pub(crate) fn reply_true<E: Into<CoreError>>(
    result: std::result::Result<Reply<()>, E>,
) -> Result<Value> {
    match result.map_err(Into::into)? {
        Reply::Done(()) => Ok(json!(true)),
        Reply::NeedsReconnect => Ok(json!({ "needsReconnect": true })),
    }
}
