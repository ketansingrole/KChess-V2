//! Typed wrappers over the stringly-typed `kchess_domain::api::call` bridge.
//!
//! The core reaches domain rules through `super::domain(method, args)` with JSON
//! values, so a typo surfaces only at runtime. These wrappers keep the call sites
//! typed and give `domain_dispatch` test a single list to pin: every wrapper's
//! method must exist in `kchess_domain::api::call`.

use serde_json::{Value, json};

use super::{domain, validate};
use crate::error::Result;

/// Every domain method the core calls through the generic bridge. Pinned by test.
pub const DOMAIN_METHODS: &[&str] = &[
    "engineLevelInfo",
    "parseInfo",
    "assertMoves",
    "assertBestMoveOptions",
];

/// `engineLevelInfo(level)`: the search profile for a computer level.
pub fn engine_level_info(level: &str) -> Result<Value> {
    domain("engineLevelInfo", vec![json!(level)])
}

/// `parseInfo(line, whiteToMove)`: one UCI info line parsed, or None.
pub fn parse_info_typed(text: &str, white_to_move: bool) -> Option<Value> {
    domain("parseInfo", vec![json!(text), json!(white_to_move)]).ok()
}

/// `assertMoves(moves)`: validated move list.
pub fn assert_moves(moves: &Value) -> Result<Value> {
    validate("assertMoves", moves)
}

/// `assertBestMoveOptions(options)`: validated search options.
pub fn assert_best_move_options(options: &Value) -> Result<Value> {
    validate("assertBestMoveOptions", options)
}

/// Whether `method` exists in the domain dispatch: any error other than
/// "unknown rules method" counts as known (argument errors are expected here).
pub fn is_known_method(method: &str) -> bool {
    match kchess_domain::api::call(method, "[]") {
        Ok(_) => true,
        Err(message) => !message.contains("unknown rules method"),
    }
}
