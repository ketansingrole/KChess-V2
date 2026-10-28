//! Reached through `api::call` (see RUST_MIGRATION.md).
//!
//! Trainer: the puzzle training session's transitions (`puzzle_session`), what an attempt counts
//! as and the day streak (`puzzle_progress`), and the analysis tree's
//! edits and helpers (`analysis_tree`).

mod analysis_tree;
pub mod puzzle_progress;
mod puzzle_session;

use serde_json::Value;

type Out = Result<Value, String>;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    type Module = fn(&str, &[Value]) -> Option<Out>;
    let modules: [Module; 3] = [
        puzzle_session::call,
        puzzle_progress::call,
        analysis_tree::call,
    ];
    modules.iter().find_map(|module| module(method, args))
}
