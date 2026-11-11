//! The position-lookup cache table. The cache keeps
//! the newest 256 entries. Decoding a cached entry and the explorer requests are
//! `crate::lichess::lookups`'s.

use rusqlite::{Connection, OptionalExtension, params, types::Value as Sql};
use serde_json::Value;

use super::library::{js_json, utf16_len};
use super::sql::Sql as _;
use crate::error::Result;

/// Stored entries beyond this size are not read back (`row.data.length > 512_000`).
const MAX_READ_LENGTH: usize = 512_000;
const MAX_ENTRIES: i64 = 256;

/// The stored JSON text of an entry, or null when it is missing or too large to read.
pub fn read(db: &Connection, key: &str) -> Result<Value> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM position_lookups WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .sql()?;
    Ok(match data {
        Some(data) if utf16_len(&data) <= MAX_READ_LENGTH => Value::String(data),
        _ => Value::Null,
    })
}

/// The connected account the explorer request is signed with: the first by name, or null.
pub fn explorer_account(db: &Connection) -> Result<Value> {
    let username: Option<String> = db
        .query_row(
            "SELECT username FROM accounts WHERE connected = 1 ORDER BY username LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .sql()?;
    Ok(username.map_or(Value::Null, Value::String))
}

/// Store an entry as its JSON text, keyed by `key`, and keep only the newest entries.
pub fn write(db: &Connection, key: &str, value: &Value) -> Result<Value> {
    let fetched_at = match value.get("fetchedAt").and_then(Value::as_f64) {
        Some(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Sql::Integer(n as i64),
        Some(n) => Sql::Real(n),
        // Missing or malformed timestamps sort first under `ORDER BY fetchedAt DESC`
        // (SQLite NULLs first on DESC) and would evict good rows first; coerce to 0.
        None => Sql::Integer(0),
    };
    db.execute(
        "INSERT OR REPLACE INTO position_lookups (key, data, fetchedAt) VALUES (?1, ?2, ?3)",
        params![key, js_json(value), fetched_at],
    )
    .sql()?;
    db.execute(
        "DELETE FROM position_lookups WHERE key NOT IN (SELECT key FROM position_lookups ORDER BY fetchedAt DESC LIMIT ?1)",
        params![MAX_ENTRIES],
    )
    .sql()?;
    Ok(Value::Null)
}
