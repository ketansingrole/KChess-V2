//! Chess.com players whose games are synced, as `store.chesscom.<name>` methods. Their games live
//! in `games` like Lichess games, under the account `name@chess.com`
//! (`kchess_domain::chesscom::account_of`), so History, reviews, insights and drills read them
//! unchanged. Each monthly archive's validators (`ETag`, `Last-Modified`) are kept so an unchanged
//! month is not downloaded again, and a month that ended and was read in full is not asked again.

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Serialize;
use serde_json::Value;

use super::StoreContext;
use super::games::{
    GAME_KEYS, MAX_GAMES, StorageResult, arg, as_json, current, now_ms, optional_arg,
    purge_account_data, to_sql, transaction, upsert_sql,
};
use crate::error::{CoreError, Result};
use crate::host::Host;
use kchess_domain::chesscom::account_of;

/// A synced Chess.com player.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChesscomAccount {
    pub username: String,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

/// What was last read of one month's archive.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Archive {
    month: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    complete: bool,
}

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let name = method.strip_prefix("store.chesscom.")?;
    Some(match name {
        "accounts" => accounts(db).and_then(as_json),
        "addAccount" => arg(args, 0, "username").and_then(|username: String| {
            if !current(args, 1) {
                return Err(cancelled());
            }
            add_account(db, &username)?;
            accounts(db).and_then(as_json)
        }),
        "removeAccount" => arg(args, 0, "username").and_then(|username: String| {
            remove_account(ctx.host, db, &username)?;
            accounts(db).and_then(as_json)
        }),
        "archives" => arg(args, 0, "username").and_then(|username: String| archives(db, &username)),
        "saveArchive" => arg(args, 0, "archive").and_then(|archive: Value| {
            if !current(args, 1) {
                return Err(cancelled());
            }
            save_archive(ctx.host, db, &archive).map(|()| Value::Null)
        }),
        "finishSync" => arg(args, 0, "username").and_then(|username: String| {
            let synced_at = optional_arg::<i64>(args, 1, "syncedAt")?.unwrap_or_else(now_ms);
            if !current(args, 2) {
                return Err(cancelled());
            }
            finish_sync(ctx.host, db, &username, synced_at).map(|()| Value::Null)
        }),
        _ => return None,
    })
}

fn cancelled() -> CoreError {
    CoreError::new("Sync was cancelled.")
}

fn accounts(db: &Connection) -> Result<Vec<ChesscomAccount>> {
    let mut statement = db
        .prepare("SELECT username, addedAt, lastSyncedAt FROM chesscom_accounts ORDER BY addedAt, username")
        .sql()?;
    statement
        .query_map([], |row| {
            Ok(ChesscomAccount {
                username: row.get(0)?,
                added_at: row.get(1)?,
                last_synced_at: row.get(2)?,
            })
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()
}

/// The player as they were added (their spelling), or why they are not there.
fn stored(db: &Connection, username: &str) -> Result<String> {
    db.query_row(
        "SELECT username FROM chesscom_accounts WHERE username = ? COLLATE NOCASE",
        [username],
        |row| row.get(0),
    )
    .optional()
    .sql()?
    .ok_or_else(|| CoreError::new("This account was removed during sync."))
}

/// Adds a player; one already added keeps its games, sync state and spelling.
fn add_account(db: &Connection, username: &str) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO chesscom_accounts (username, addedAt) VALUES (?, ?)",
        params![username, now_ms()],
    )
    .sql()?;
    Ok(())
}

/// Removes a player with their games, the reviews only those games used, and their archives.
fn remove_account(host: &dyn Host, db: &Connection, username: &str) -> Result<()> {
    transaction(host, db, || {
        let name = stored(db, username).unwrap_or_else(|_| username.to_owned());
        purge_account_data(db, &account_of(&name), false).sql()?;
        db.execute(
            "DELETE FROM chesscom_accounts WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        Ok(())
    })
}

fn archives(db: &Connection, username: &str) -> Result<Value> {
    let mut statement = db
        .prepare(
            "SELECT month, etag, lastModified, complete FROM chesscom_archives WHERE username = ? COLLATE NOCASE ORDER BY month",
        )
        .sql()?;
    let rows = statement
        .query_map([username], |row| {
            Ok(Archive {
                month: row.get(0)?,
                etag: row.get(1)?,
                last_modified: row.get(2)?,
                complete: row.get::<_, i64>(3)? != 0,
            })
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    as_json(rows)
}

/// One month's games and its validators, in one transaction: a month is never marked read
/// without its games. `archive` is `{username, month, etag?, lastModified?, complete, games}`,
/// each game a `games` row (`kchess_domain::chesscom::game_row`).
fn save_archive(host: &dyn Host, db: &Connection, archive: &Value) -> Result<()> {
    let text = |key: &str| archive.get(key).and_then(Value::as_str);
    let username = text("username").ok_or_else(|| CoreError::new("Invalid archive."))?;
    let month = text("month").ok_or_else(|| CoreError::new("Invalid archive."))?;
    let games = archive
        .get("games")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    transaction(host, db, || {
        let account = account_of(&stored(db, username)?);
        let mut upsert = db.prepare(&upsert_sql()).sql()?;
        for game in games {
            // The row's account is the stored player's, whatever case the archive spelled it in.
            let params = GAME_KEYS
                .iter()
                .map(|key| match *key {
                    "account" => Ok(SqlValue::Text(account.clone())),
                    key => to_sql(game.get(key)),
                })
                .collect::<Result<Vec<_>>>()?;
            upsert.execute(params_from_iter(params)).sql()?;
        }
        db.execute(
            "INSERT INTO chesscom_archives (username, month, etag, lastModified, complete) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(username, month) DO UPDATE SET etag = excluded.etag,
               lastModified = excluded.lastModified, complete = excluded.complete",
            params![
                username,
                month,
                text("etag"),
                text("lastModified"),
                archive.get("complete").and_then(Value::as_bool).unwrap_or(false),
            ],
        )
        .sql()?;
        Ok(())
    })
}

/// The end of a sync: the player keeps their newest `MAX_GAMES` games, and the sync time.
fn finish_sync(host: &dyn Host, db: &Connection, username: &str, synced_at: i64) -> Result<()> {
    transaction(host, db, || {
        let account = account_of(&stored(db, username)?);
        db.execute(
            &format!(
                "DELETE FROM games WHERE account = ? COLLATE NOCASE AND rowid NOT IN (SELECT rowid FROM games WHERE account = ? COLLATE NOCASE ORDER BY createdAt DESC LIMIT {MAX_GAMES})"
            ),
            [&account, &account],
        )
        .sql()?;
        db.execute(
            "UPDATE chesscom_accounts SET lastSyncedAt = ? WHERE username = ? COLLATE NOCASE",
            params![synced_at, username],
        )
        .sql()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        super::super::migrations::migrate(&db).unwrap();
        db
    }

    struct Quiet;
    impl Host for Quiet {
        fn log(&self, _: crate::host::Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    fn game(id: &str, created_at: i64) -> Value {
        json!({
            "id": id, "account": "ignored", "createdAt": created_at, "lastMoveAt": created_at,
            "rated": true, "speed": "blitz", "perf": "blitz", "status": "resign",
            "winner": "white", "color": "white", "opponent": "bob", "opponentRating": 1400,
            "playerRating": 1500, "ratingDiff": null, "opening": null, "moves": "e4 e5",
            "pgn": "1. e4 e5 1-0",
        })
    }

    fn count(db: &Connection, account: &str) -> i64 {
        db.query_row(
            "SELECT COUNT(*) FROM games WHERE account = ?",
            [account],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn a_month_is_saved_with_its_games_and_removing_the_player_clears_them() {
        let db = database();
        add_account(&db, "Alice").unwrap();
        add_account(&db, "alice").unwrap();
        assert_eq!(accounts(&db).unwrap().len(), 1);
        // The Lichess player of the same name keeps their games.
        db.execute(
            "INSERT INTO accounts (username, connected) VALUES ('alice', 0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color, opponent, moves) VALUES ('alice', 'abcdefgh', 1, 1, 1, 'blitz', 'blitz', 'mate', 'white', 'x', 'e4')",
            [],
        )
        .unwrap();
        let archive = json!({
            "username": "ALICE", "month": "2024/03", "etag": "\"v1\"", "complete": true,
            "games": [game("cc-l1", 10), game("cc-l2", 20)],
        });
        save_archive(&Quiet, &db, &archive).unwrap();
        assert_eq!(count(&db, "Alice@chess.com"), 2);
        let saved = archives(&db, "alice").unwrap();
        assert_eq!(saved[0]["month"], "2024/03");
        assert_eq!(saved[0]["etag"], "\"v1\"");
        assert_eq!(saved[0]["complete"], true);
        remove_account(&Quiet, &db, "alice").unwrap();
        assert_eq!(count(&db, "Alice@chess.com"), 0);
        assert_eq!(count(&db, "alice"), 1);
        assert_eq!(archives(&db, "alice").unwrap(), json!([]));
    }

    #[test]
    fn a_removed_player_takes_no_more_games() {
        let db = database();
        let archive = json!({ "username": "carol", "month": "2024/03", "complete": true,
            "games": [game("cc-l1", 10)] });
        let error = save_archive(&Quiet, &db, &archive).unwrap_err();
        assert!(error.message.contains("removed"), "{}", error.message);
        assert_eq!(count(&db, "carol@chess.com"), 0);
        assert!(finish_sync(&Quiet, &db, "carol", 5).is_err());
    }

    #[test]
    fn a_sync_keeps_the_newest_games() {
        let db = database();
        add_account(&db, "dave").unwrap();
        let games: Vec<Value> = (0..(MAX_GAMES + 3))
            .map(|n| game(&format!("cc-l{n}"), n))
            .collect();
        let archive = json!({ "username": "dave", "month": "2024/03", "complete": false,
            "games": games });
        save_archive(&Quiet, &db, &archive).unwrap();
        finish_sync(&Quiet, &db, "dave", 99).unwrap();
        assert_eq!(count(&db, "dave@chess.com"), MAX_GAMES);
        let oldest: i64 = db
            .query_row(
                "SELECT MIN(createdAt) FROM games WHERE account = 'dave@chess.com'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(oldest, 3);
        assert_eq!(accounts(&db).unwrap()[0].last_synced_at, Some(99));
    }
}
