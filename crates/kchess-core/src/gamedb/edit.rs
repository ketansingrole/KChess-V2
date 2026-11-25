//! Changing a database's games once they are in: removing some, and writing some out as PGN, to
//! copy them into another database or save them as a file. Adding games is an import into the
//! database (`import::Target::Existing`).
//!
//! Removing games builds the opening tree and the position index again from the games left, in
//! the same transaction as the removal. The tree's rows keep each move's best and latest game,
//! which counts alone cannot replace when that game goes; built again, every row is exactly what
//! an import of the games left would have made. A failure or a cancel rolls it all back.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, params};

use super::deep::{self, DeepBuilder};
use super::import::{self, Count, Indexed, Watch};
use super::tree::TreeBuilder;
use super::{explore, schema, search};
use crate::store::sql::{Sql as _, transaction};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::GameDbQuery;
use kchess_domain::gamedb::{self, Outcome};

/// Where games from the board go: a database on this computer, or a new one of that name.
pub enum Into {
    Existing(String),
    New(String),
}

/// Which games an edit acts on.
pub enum Selection {
    /// The games a search listed, by id.
    Games(Vec<u64>),
    /// Every game a search of the database finds, whatever its page.
    Search(GameDbQuery),
    /// Every game that copies an earlier one (`duplicates`).
    Duplicates,
}

/// The ids of the games `selection` names that the database holds, in id order.
pub fn selected(db: &Connection, selection: &Selection, stop: &AtomicBool) -> Result<Vec<i64>> {
    match selection {
        Selection::Games(ids) => {
            let visible = search::visible(db)?;
            let mut exists = db
                .prepare_cached("SELECT 1 FROM games WHERE id = ?1 AND import_id <= ?2")
                .sql()?;
            let mut held = Vec::with_capacity(ids.len());
            for &id in ids {
                let id = i64::try_from(id).unwrap_or(i64::MAX);
                if exists.exists(params![id, visible]).sql()? {
                    held.push(id);
                }
            }
            Ok(held)
        }
        Selection::Search(query) if query.position.is_some() => explore::ids_at(db, query, stop),
        Selection::Search(query) => search::ids(db, query),
        Selection::Duplicates => duplicates(db, stop),
    }
}

/// The games that copy an earlier game of the database, in id order: the same players, date,
/// result, start and moves. The first of each stays.
pub fn duplicates(db: &Connection, stop: &AtomicBool) -> Result<Vec<i64>> {
    let visible = search::visible(db)?;
    // Each game is compared with the earlier games of its White on its date, which the White
    // index (`schema::INDEXES`) finds.
    let mut games = db
        .prepare(
            "SELECT g.id FROM games g
             WHERE g.import_id <= ?1 AND EXISTS (
               SELECT 1 FROM games o
               WHERE o.white = g.white AND o.date_key = g.date_key AND o.id < g.id
                 AND o.import_id <= ?1 AND o.black = g.black AND o.result IS g.result
                 AND o.fen IS g.fen AND o.plies = g.plies AND o.moves = g.moves)
             ORDER BY g.id",
        )
        .sql()?;
    let mut rows = games.query([visible]).sql()?;
    let mut ids = Vec::new();
    let mut read = 0u64;
    while let Some(row) = rows.next().sql()? {
        ids.push(row.get(0).sql()?);
        read += 1;
        if read.is_multiple_of(10_000) && stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CoreError::aborted("Cancelled."));
        }
    }
    Ok(ids)
}

/// Writes the games `ids` of `db` to `out` as PGN, one after another; answers how many. A game
/// that cannot be written again is left out.
pub fn write_pgn(
    db: &Connection,
    ids: &[i64],
    out: &mut impl Write,
    cancelled: &dyn Fn() -> bool,
) -> Result<u64> {
    let mut written = 0u64;
    for (index, &id) in ids.iter().enumerate() {
        if index % 1_000 == 0 && cancelled() {
            return Err(CoreError::aborted("Cancelled."));
        }
        let Some(stored) = search::stored(db, id as u64)? else {
            continue;
        };
        let Some(pgn) = gamedb::pgn_of(&stored) else {
            continue;
        };
        out.write_all(pgn.trim_end().as_bytes())
            .and_then(|()| out.write_all(b"\n\n"))
            .map_err(|cause| CoreError::wrap("Could not write the games", cause))?;
        written += 1;
    }
    out.flush()
        .map_err(|cause| CoreError::wrap("Could not write the games", cause))?;
    Ok(written)
}

/// Removes the games `ids` from the database at `file` and builds its indexes again from the
/// games left; answers how many were removed. `dir` holds the builders' run files.
pub fn remove(
    file: &Path,
    dir: &Path,
    ids: &[i64],
    capacity: usize,
    watch: &mut dyn Watch,
) -> Result<u64> {
    let db = schema::open_write(file)?;
    let depth: u32 = db
        .query_row("SELECT index_depth FROM meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .sql()?;
    let mut removed = 0u64;
    transaction(&db, || {
        // An import that never finished (KChess was stopped under it) left games no search saw.
        db.execute_batch(
            "DELETE FROM games WHERE import_id IN
               (SELECT id FROM imports WHERE finished_at IS NULL);
             DELETE FROM imports WHERE finished_at IS NULL;",
        )
        .sql()?;
        {
            // A database's count is what each import kept (`schema::info`), so the import a
            // game came in by keeps one fewer.
            let mut uncount = db
                .prepare_cached(
                    "UPDATE imports SET games = games - 1
                     WHERE games > 0 AND id = (SELECT import_id FROM games WHERE id = ?1)",
                )
                .sql()?;
            let mut delete = db.prepare_cached("DELETE FROM games WHERE id = ?1").sql()?;
            for &id in ids {
                uncount.execute([id]).sql()?;
                removed += delete.execute([id]).sql()? as u64;
            }
        }
        // The names only those games had go with them.
        db.execute_batch(
            "DELETE FROM players WHERE id NOT IN
               (SELECT white FROM games UNION SELECT black FROM games);
             DELETE FROM player_words WHERE id NOT IN (SELECT id FROM players);
             DELETE FROM events WHERE id NOT IN
               (SELECT event FROM games WHERE event IS NOT NULL);
             DELETE FROM event_words WHERE id NOT IN (SELECT id FROM events);
             DELETE FROM tree;
             DELETE FROM lone;
             DELETE FROM deep;",
        )
        .sql()?;
        reindex(&db, dir, depth, capacity, watch)?;
        db.execute(
            "UPDATE meta SET updated_at = ?1, deep_built = 1 WHERE id = 1",
            [crate::time::now_ms()],
        )
        .sql()?;
        Ok(())
    })?;
    Ok(removed)
}

/// The tree and the position index of every game in `db`, written into their emptied tables.
fn reindex(
    db: &Connection,
    dir: &Path,
    depth: u32,
    capacity: usize,
    watch: &mut dyn Watch,
) -> Result<()> {
    let mut tree = TreeBuilder::new(dir, capacity)?;
    let mut deep = DeepBuilder::new(dir, deep::CAPACITY)?;
    let mut count = Count::default();
    {
        let mut games = db
            .prepare("SELECT id, result, rating, date_key, fen, moves FROM games ORDER BY id")
            .sql()?;
        let mut rows = games.query([]).sql()?;
        while let Some(row) = rows.next().sql()? {
            let fen: Option<String> = row.get(4).sql()?;
            let moves: Vec<u8> = row.get(5).sql()?;
            let rating: Option<i64> = row.get(2).sql()?;
            let date: i64 = row.get(3).sql()?;
            let game = Indexed {
                id: row.get(0).sql()?,
                outcome: Outcome::from_score(row.get(1).sql()?),
                rating: rating.and_then(|rating| u16::try_from(rating).ok()),
                date: u32::try_from(date).unwrap_or(0),
                fen: fen.as_deref(),
                moves: &moves,
            };
            import::index_moves(&mut tree, &mut deep, &game, depth)?;
            count.games += 1;
            if count.games.is_multiple_of(5_000) {
                if watch.cancelled() {
                    return Err(CoreError::aborted("Cancelled."));
                }
                watch.indexing(count);
            }
        }
    }
    watch.indexing(count);
    tree.write(db, false, &|| watch.cancelled())?;
    deep.write(db, &|| watch.cancelled())?;
    Ok(())
}
