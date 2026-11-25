//! A database's players' names: the spellings likely one player's, found by
//! `kchess_domain::gamedb::names`, and merging some into one, which only renames: the games, their
//! moves and the indexes stay as they are.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _, params};

use super::{schema, search};
use crate::store::sql::{Sql as _, transaction};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{GameDbName, GameDbNameGroup};
use kchess_domain::gamedb::names::same_players;

/// The most groups a database lists, the most played first.
pub const GROUPS: usize = 500;

/// Every name the games of `db` hold, with their games.
fn played(db: &Connection) -> Result<Vec<(String, u64)>> {
    let visible = search::visible(db)?;
    let mut names = db
        .prepare(
            "SELECT p.name, COUNT(*) FROM players p JOIN (
               SELECT white AS id FROM games WHERE import_id <= ?1
               UNION ALL SELECT black FROM games WHERE import_id <= ?1 AND black != white
             ) g ON g.id = p.id GROUP BY p.id",
        )
        .sql()?;
    let rows = names
        .query_map([visible], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
            ))
        })
        .sql()?;
    rows.map(|row| row.sql()).collect()
}

/// The names of `db` that are likely one player's.
pub fn groups(db: &Connection) -> Result<Vec<GameDbNameGroup>> {
    Ok(same_players(&played(db)?)
        .into_iter()
        .take(GROUPS)
        .map(|names| GameDbNameGroup {
            names: names
                .into_iter()
                .map(|(name, games)| GameDbName { name, games })
                .collect(),
        })
        .collect())
}

/// Gives the games of each of `names` in the database at `file` the name `into`; answers how
/// many games changed. The names merged go, with their words.
pub fn merge(file: &Path, into: &str, names: &[String]) -> Result<u64> {
    let db = schema::open_write(file)?;
    let id_of = |name: &str| -> Result<Option<i64>> {
        db.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .optional()
        .sql()
    };
    let Some(into_id) = id_of(into)? else {
        return Err(CoreError::new("That name is not in this database."));
    };
    let mut changed = 0u64;
    transaction(&db, || {
        for name in names.iter().filter(|name| name.as_str() != into) {
            let Some(gone) = id_of(name)? else { continue };
            changed += db
                .prepare_cached(
                    "UPDATE games SET
                       white = CASE white WHEN ?1 THEN ?2 ELSE white END,
                       black = CASE black WHEN ?1 THEN ?2 ELSE black END
                     WHERE white = ?1 OR black = ?1",
                )
                .sql()?
                .execute(params![gone, into_id])
                .sql()? as u64;
            db.execute("DELETE FROM players WHERE id = ?1", [gone])
                .sql()?;
            db.execute("DELETE FROM player_words WHERE id = ?1", [gone])
                .sql()?;
        }
        db.execute(
            "UPDATE meta SET updated_at = ?1 WHERE id = 1",
            [crate::time::now_ms()],
        )
        .sql()?;
        Ok(())
    })?;
    Ok(changed)
}
