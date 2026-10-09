//! See `store/mod.rs`.

use rusqlite::Connection;
use serde_json::Value;

use crate::error::Result;

/// This module's storage methods; None when the method is not one of them.
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Option<Result<Value>> {
    // Each method: `"store.<module>.<name>" => Some(…)`; no methods yet.
    let _ = (db, method, args);
    None
}
