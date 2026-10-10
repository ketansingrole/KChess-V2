//! Network accounting in the database (`core/src/services/usage.ts`): the counters `flushUsage`
//! writes, `resetUsage` clears and `usageReport` reads. The in-memory batching, the flush timer,
//! the request contexts and the database file sizes stay in TypeScript.

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::StoreContext;
use super::library::{db_err, now_ms, transaction};
use crate::error::Result;

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let result = match method {
        "store.usage.flushUsage" => flush_usage(db, args.first().unwrap_or(&Value::Null)),
        "store.usage.resetUsage" => db
            .execute("DELETE FROM usage", [])
            .map(|_| Value::Null)
            .map_err(db_err),
        "store.usage.usageReport" => usage_report(db),
        _ => return None,
    };
    Some(result)
}

/// `flushUsage`'s write: one batch of counters, added to the stored totals in one transaction.
fn flush_usage(db: &Connection, rows: &Value) -> Result<Value> {
    let rows = rows.as_array().cloned().unwrap_or_default();
    transaction(db, || {
        let now = now_ms()? as i64;
        for row in &rows {
            db.execute(
                "INSERT INTO usage (account, kind, requests, bytesIn, since) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account, kind) DO UPDATE SET requests = requests + excluded.requests, bytesIn = bytesIn + excluded.bytesIn",
                params![
                    row.get("account").and_then(Value::as_str).unwrap_or_default(),
                    row.get("kind").and_then(Value::as_str).unwrap_or_default(),
                    row.get("requests").and_then(Value::as_i64).unwrap_or_default(),
                    row.get("bytesIn").and_then(Value::as_i64).unwrap_or_default(),
                    now,
                ],
            )
            .map_err(db_err)?;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The database part of `usageReport`: per-account totals and kinds, the storage each account
/// occupies in games and cached responses, and the earliest counting time.
fn usage_report(db: &Connection) -> Result<Value> {
    let mut accounts = Map::new();
    let mut storage = Map::new();
    let mut since: Option<f64> = None;

    let mut statement = db
        .prepare("SELECT account, kind, requests, bytesIn, since FROM usage")
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(db_err)?;
    for row in rows {
        let (account, kind, requests, bytes_in, started) = row.map_err(db_err)?;
        let entry = accounts
            .entry(account)
            .or_insert_with(|| json!({ "total": { "requests": 0, "bytesIn": 0 }, "byKind": {} }));
        if let Some(by_kind) = entry.get_mut("byKind").and_then(Value::as_object_mut) {
            by_kind.insert(kind, json!({ "requests": requests, "bytesIn": bytes_in }));
        }
        if let Some(total) = entry.get_mut("total").and_then(Value::as_object_mut) {
            add_to(total, "requests", requests);
            add_to(total, "bytesIn", bytes_in);
        }
        since = Some(since.map_or(started as f64, |s| s.min(started as f64)));
    }

    let mut statement = db
        .prepare(
            "SELECT account, COUNT(*) AS games,
               SUM(LENGTH(id) + LENGTH(status) + LENGTH(speed) + LENGTH(perf) + LENGTH(opponent)
                 + COALESCE(LENGTH(opening), 0) + LENGTH(moves) + COALESCE(LENGTH(pgn), 0) + 64) AS bytes
             FROM games GROUP BY account",
        )
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(db_err)?;
    for row in rows {
        let (account, games, bytes) = row.map_err(db_err)?;
        storage.insert(
            account.to_lowercase(),
            json!({ "games": games, "bytes": bytes, "cacheBytes": 0 }),
        );
    }

    let mut statement = db
        .prepare("SELECT key, LENGTH(value) AS bytes FROM api_cache")
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(db_err)?;
    for row in rows {
        let (key, bytes) = row.map_err(db_err)?;
        let account = key.split(':').next().unwrap_or_default().to_string();
        let entry = storage
            .entry(account)
            .or_insert_with(|| json!({ "games": 0, "bytes": 0, "cacheBytes": 0 }));
        if let Some(cache) = entry.get_mut("cacheBytes") {
            let total = cache.as_i64().unwrap_or_default() + bytes;
            *cache = json!(total);
        }
    }

    let mut report = json!({ "accounts": accounts, "storage": storage });
    if let (Some(started), Some(object)) = (since, report.as_object_mut()) {
        object.insert("since".into(), json!(started));
    }
    Ok(report)
}

/// `map[key] += delta` for an integer counter.
fn add_to(map: &mut Map<String, Value>, key: &str, delta: i64) {
    let current = map.get(key).and_then(Value::as_i64).unwrap_or_default();
    map.insert(key.to_string(), json!(current + delta));
}
