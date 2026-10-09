//! Rules moved from `core/src/domain` (see RUST_MIGRATION.md); reached through `api::call`.

use serde_json::Value;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // Each method: `"name" => Some(…)`; no methods yet.
    let _ = (method, args);
    None
}
