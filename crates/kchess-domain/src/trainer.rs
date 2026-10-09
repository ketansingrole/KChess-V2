//! Rules moved from `core/src/domain` (see RUST_MIGRATION.md); reached through `api::call`.
//!
//! Trainer: the puzzle training session's transitions (`puzzle_session`) and the analysis tree's
//! edits and helpers (`analysis_tree`).

mod analysis_tree;
mod puzzle_session;

use serde_json::Value;

type Out = Result<Value, String>;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    type Module = fn(&str, &[Value]) -> Option<Out>;
    let modules: [Module; 2] = [puzzle_session::call, analysis_tree::call];
    modules.iter().find_map(|module| module(method, args))
}
