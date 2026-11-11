//! Network accounting in the database: the counters `flushUsage`
//! writes, `resetUsage` clears and `usageReport` reads. The in-memory batching, the flush timer
//! and the request contexts are `crate::usage`'s.

use kchess_contracts::types::{AccountStorage, UsageCell, UsageReport};
use rusqlite::{Connection, params};

use super::StoreContext;
use super::sql::{Sql as _, transaction};
use crate::error::Result;
use crate::time::now_ms;

/// The traffic of one account and kind, not yet written.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageRow {
    pub account: String,
    pub kind: String,
    pub requests: i64,
    pub bytes_in: i64,
}

/// One batch of counters, added to the stored totals in one transaction.
pub fn flush_usage(db: &Connection, rows: &[UsageRow]) -> Result<()> {
    transaction(db, || {
        let now = now_ms();
        for row in rows {
            db.execute(
                "INSERT INTO usage (account, kind, requests, bytesIn, since) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account, kind) DO UPDATE SET requests = requests + excluded.requests, bytesIn = bytesIn + excluded.bytesIn",
                params![row.account, row.kind, row.requests, row.bytes_in, now],
            )
            .sql()?;
        }
        Ok(())
    })
}

pub fn reset_usage(db: &Connection) -> Result<()> {
    db.execute("DELETE FROM usage", []).sql()?;
    Ok(())
}

/// The database part of `usageReport`: per-account totals and kinds, the storage each account
/// occupies in games and cached responses, and the earliest counting time.
pub fn usage_report(ctx: &StoreContext) -> Result<UsageReport> {
    let db = ctx.db;
    let mut report = UsageReport::default();

    let mut statement = db
        .prepare("SELECT account, kind, requests, bytesIn, since FROM usage")
        .sql()?;
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
        .sql()?;
    for row in rows {
        let (account, kind, requests, bytes_in, started) = row.sql()?;
        let entry = report.accounts.entry(&account);
        *entry.by_kind.entry(&kind) = UsageCell { requests, bytes_in };
        entry.total.requests += requests;
        entry.total.bytes_in += bytes_in;
        let started = started as f64;
        report.since = Some(report.since.map_or(started, |since| since.min(started)));
    }

    for (account, games, bytes) in game_storage(ctx)? {
        *report.storage.entry(&account.to_lowercase()) = AccountStorage {
            games,
            bytes,
            cache_bytes: 0,
        };
    }

    let mut statement = db
        .prepare("SELECT key, LENGTH(value) AS bytes FROM api_cache")
        .sql()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .sql()?;
    for row in rows {
        let (key, bytes) = row.sql()?;
        let account = key.split(':').next().unwrap_or_default();
        report.storage.entry(account).cache_bytes += bytes;
    }
    Ok(report)
}

/// Each account's games and the bytes they take. Summing them reads every game's row, moves and
/// PGN included, so the sums are kept until the games change; `octet_length` reads each value's
/// size from its row header instead of decoding the text.
fn game_storage(ctx: &StoreContext) -> Result<Vec<(String, i64, i64)>> {
    if let Some(rows) = ctx.cache.storage() {
        return Ok(rows);
    }
    let mut statement = ctx
        .db
        .prepare(
            "SELECT account, COUNT(*) AS games,
               SUM(octet_length(id) + octet_length(status) + octet_length(speed)
                 + octet_length(perf) + octet_length(opponent) + COALESCE(octet_length(opening), 0)
                 + octet_length(moves) + COALESCE(octet_length(pgn), 0) + 64) AS bytes
             FROM games GROUP BY account",
        )
        .sql()?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .sql()?
        .collect::<std::result::Result<Vec<(String, i64, i64)>, _>>()
        .sql()?;
    ctx.cache.remember_storage(&rows);
    Ok(rows)
}
