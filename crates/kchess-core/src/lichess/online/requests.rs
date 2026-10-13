use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use super::state::{Lane, OnlineOptions};

/// One request of a stream: the method, the path, the query, the `Authorization` header value
/// and a form body (for the seek).
#[derive(Clone)]
pub(crate) struct StreamRequest {
    pub(crate) method: reqwest::Method,
    pub(crate) path: String,
    pub(crate) query: Vec<(&'static str, String)>,
    pub(crate) auth: Option<String>,
    pub(crate) form: Option<Vec<(&'static str, String)>>,
}

pub(crate) type Emitter = Arc<dyn Fn(Value) + Send + Sync>;
pub(crate) type Quiet = Arc<dyn Fn(&str, Option<&str>) + Send + Sync>;

/// How one `listen` runs (`ListenOptions`), with the session facts it captured at the start.
#[derive(Clone)]
pub(crate) struct Listen {
    pub(crate) lane: Lane,
    pub(crate) reconnect: bool,
    /// Set by the emitter when the game is over: a clean end of stream does not reconnect.
    pub(crate) finished: Option<Arc<AtomicBool>>,
    /// Reports the stream's health instead of the game connection (the idle lobby stream).
    pub(crate) quiet: Option<Quiet>,
    pub(crate) session: u64,
    pub(crate) account: String,
    pub(crate) game_id: String,
}

/// The body of a Lichess error response, as `throwLichessErrors` reads it.
pub(crate) fn body_value(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

/// `gameBody(options)`: the form fields of a seek or challenge.
pub(crate) fn game_body(options: &OnlineOptions) -> Vec<(&'static str, String)> {
    let has_fen = options.fen.as_deref().is_some_and(|fen| !fen.is_empty());
    let variant = if has_fen && options.variant.as_deref() != Some("chess960") {
        Some("fromPosition")
    } else {
        options.variant.as_deref()
    };
    let variant = variant.filter(|variant| !variant.is_empty() && *variant != "standard");
    let mut body = vec![
        ("rated", options.rated.to_string()),
        ("color", options.color.clone()),
    ];
    if let Some(variant) = variant {
        body.push(("variant", variant.to_string()));
    }
    if let Some(fen) = &options.fen {
        body.push(("fen", fen.clone()));
    }
    body
}

pub(crate) fn refs<'a>(form: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    form.iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect()
}
