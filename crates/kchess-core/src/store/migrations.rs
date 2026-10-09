//! See `store/mod.rs`.

use rusqlite::Connection;
use serde_json::Value;

use crate::error::Result;

/// Bring the schema up to date, exactly as `migrations.ts` `migrate` does.
pub fn migrate(db: &Connection) -> Result<()> {
    let _ = db;
    Ok(())
}

/// This module's storage methods; None when the method is not one of them.
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let _ = (db, method, args);
    None
}
