//! The position-lookup cache table (`core/src/services/setupPositionLookup.ts`). The cache keeps
//! the newest 256 entries. Decoding a cached entry and the explorer requests stay in TypeScript.

use rusqlite::{Connection, OptionalExtension, params, types::Value as Sql};
use serde_json::Value;

use super::StoreContext;
use super::library::{db_err, js_json, str_arg, utf16_len};
use crate::error::Result;

/// Stored entries beyond this size are not read back (`row.data.length > 512_000`).
const MAX_READ_LENGTH: usize = 512_000;
const MAX_ENTRIES: i64 = 256;

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let result = match method {
        "store.setupPositionLookup.read" => str_arg(args, 0, "key").and_then(|key| read(db, key)),
        "store.setupPositionLookup.write" => {
            let value = args.get(1).cloned().unwrap_or(Value::Null);
            str_arg(args, 0, "key").and_then(|key| write(db, key, &value))
        }
        "store.setupPositionLookup.explorerAccount" => explorer_account(db),
        _ => return None,
    };
    Some(result)
}

/// The stored JSON text of an entry, or null when it is missing or too large to read.
fn read(db: &Connection, key: &str) -> Result<Value> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM position_lookups WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(match data {
        Some(data) if utf16_len(&data) <= MAX_READ_LENGTH => Value::String(data),
        _ => Value::Null,
    })
}

/// The connected account the explorer request is signed with: the first by name, or null.
fn explorer_account(db: &Connection) -> Result<Value> {
    let username: Option<String> = db
        .query_row(
            "SELECT username FROM accounts WHERE connected = 1 ORDER BY username LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(username.map_or(Value::Null, Value::String))
}

/// Store an entry as its JSON text, keyed by `key`, and keep only the newest entries.
fn write(db: &Connection, key: &str, value: &Value) -> Result<Value> {
    let fetched_at = match value.get("fetchedAt").and_then(Value::as_f64) {
        Some(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Sql::Integer(n as i64),
        Some(n) => Sql::Real(n),
        None => Sql::Null,
    };
    db.execute(
        "INSERT OR REPLACE INTO position_lookups (key, data, fetchedAt) VALUES (?1, ?2, ?3)",
        params![key, js_json(value), fetched_at],
    )
    .map_err(db_err)?;
    db.execute(
        "DELETE FROM position_lookups WHERE key NOT IN (SELECT key FROM position_lookups ORDER BY fetchedAt DESC LIMIT ?1)",
        params![MAX_ENTRIES],
    )
    .map_err(db_err)?;
    Ok(Value::Null)
}
