use serde_json::{Value, json};

use super::state::CHALLENGE_TYPES;

/// `String.prototype.slice(0, max)` over UTF-16 units.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let width = ch.len_utf16();
        if units + width > max {
            break;
        }
        units += width;
        out.push(ch);
    }
    out
}

pub(crate) fn is_challenge_event(kind: &str) -> bool {
    CHALLENGE_TYPES.contains(&kind)
}

/// `validateOnlineEvent(raw)`: the event, or None for an unknown type or a blank line.
pub(crate) fn validate_online_event(raw: &Value) -> std::result::Result<Option<Value>, String> {
    match kchess_domain::records::call("validateOnlineEvent", std::slice::from_ref(raw)) {
        Some(Ok(Value::Null)) | None => Ok(None),
        Some(Ok(event)) => Ok(Some(event)),
        Some(Err(message)) => Err(message),
    }
}

/// `isGameInProgress(status)`.
pub(crate) fn game_in_progress(status: &str) -> bool {
    matches!(
        kchess_domain::records::call("isGameInProgress", &[json!(status)]),
        Some(Ok(Value::Bool(true)))
    )
}

/// `perfFor(minutes, increment)`.
pub(crate) fn perf_for(minutes: f64, increment: f64) -> String {
    match kchess_domain::records::call("perfFor", &[json!(minutes), json!(increment)]) {
        Some(Ok(Value::String(perf))) => perf,
        _ => String::new(),
    }
}

pub(crate) fn rule_bool(method: &str, minutes: f64, increment: f64) -> bool {
    matches!(
        kchess_domain::records::call(method, &[json!(minutes), json!(increment)]),
        Some(Ok(Value::Bool(true)))
    )
}
