//! A deep search: the games a search's other filters keep, replayed to keep those played at a
//! speed (their time control), and those that reach a material balance, a pawn structure or
//! pieces on given squares and keep it (`kchess_domain::gamedb::patterns`). It runs on every core at once, each over a
//! stretch of the games in id order, and stops as soon as its caller does.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;

use super::explore::Reached;
use super::{schema, search};
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{ExplorerSpeed, GameDbQuery, GameDbStructure};
use kchess_domain::gamedb::patterns::{self, Material, Placement, Speed, Structure};
use kchess_domain::gamedb::{self};

const STOPPED: &str = "The search stopped.";
/// The games a worker reads between looks at whether to stop.
const BATCH: usize = 256;

/// Whether `query` needs its games replayed.
pub fn deep(query: &GameDbQuery) -> bool {
    query.speed.is_some()
        || query.material.is_some()
        || query.structure.is_some()
        || query.pieces.is_some()
}

/// What a deep search looks for.
struct Wanted {
    speed: Option<ExplorerSpeed>,
    material: Option<Material>,
    structure: Option<Structure>,
    pieces: Option<Placement>,
}

impl Wanted {
    fn of(query: &GameDbQuery) -> Result<Wanted> {
        let material = match query.material.as_deref() {
            Some(text) => Some(
                Material::parse(text)
                    .ok_or_else(|| CoreError::new("That material is not one a game can have."))?,
            ),
            None => None,
        };
        let pieces = match query.pieces.as_deref() {
            Some(text) => Some(Placement::parse(text).ok_or_else(|| {
                CoreError::new("Write the pieces as each side's, like Nd5 vs e6.")
            })?),
            None => None,
        };
        Ok(Wanted {
            speed: query.speed,
            material,
            pieces,
            structure: query.structure.map(|structure| match structure {
                GameDbStructure::IsolatedQueenPawn => Structure::IsolatedQueenPawn,
                GameDbStructure::HangingPawns => Structure::HangingPawns,
                GameDbStructure::Carlsbad => Structure::Carlsbad,
                GameDbStructure::MaroczyBind => Structure::MaroczyBind,
                GameDbStructure::Stonewall => Structure::Stonewall,
            }),
        })
    }

    /// Whether the game starting at `fen` with the encoded `moves` and `extra` is one.
    fn keeps(&self, fen: Option<&str>, moves: &[u8], extra: Option<&[u8]>) -> bool {
        if let Some(speed) = self.speed {
            let played = gamedb::header_of(extra, "TimeControl")
                .as_deref()
                .and_then(patterns::speed);
            if played != Some(speed_of(speed)) {
                return false;
            }
        }
        if self.material.is_none() && self.structure.is_none() && self.pieces.is_none() {
            return true;
        }
        let Some(start) = gamedb::start_of(fen) else {
            return false;
        };
        if let Some(material) = &self.material
            && patterns::reaches(&start, moves, patterns::changes_material, |board| {
                material.on(board)
            }) != Some(true)
        {
            return false;
        }
        if let Some(structure) = self.structure
            && patterns::reaches(&start, moves, patterns::changes_pawns, |board| {
                structure.on(board)
            }) != Some(true)
        {
            return false;
        }
        // Any move can bring a piece to its square or take it away.
        if let Some(pieces) = &self.pieces
            && patterns::reaches(&start, moves, |_| true, |board| pieces.on(board)) != Some(true)
        {
            return false;
        }
        true
    }
}

fn speed_of(speed: ExplorerSpeed) -> Speed {
    match speed {
        ExplorerSpeed::UltraBullet => Speed::UltraBullet,
        ExplorerSpeed::Bullet => Speed::Bullet,
        ExplorerSpeed::Blitz => Speed::Blitz,
        ExplorerSpeed::Rapid => Speed::Rapid,
        ExplorerSpeed::Classical => Speed::Classical,
        ExplorerSpeed::Correspondence => Speed::Correspondence,
    }
}

/// Every game the search's columns keep (all but its position and its deep filters), in id order.
pub fn candidates(db: &Connection, query: &GameDbQuery) -> Result<Vec<Reached>> {
    let (clauses, args) = search::filters(query, search::visible(db)?, search::worded(db)?);
    let mut select = db
        .prepare(&format!(
            "SELECT g.id, g.rating, g.date_key FROM games g WHERE {clauses} ORDER BY g.id"
        ))
        .sql()?;
    select
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(Reached {
                id: row.get(0)?,
                rating: row.get(1)?,
                date: row.get(2)?,
            })
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()
}

/// Those of `games` (in id order) the deep filters of `query` keep, in the same order.
pub fn kept(
    db: &Connection,
    query: &GameDbQuery,
    games: Vec<Reached>,
    stop: &AtomicBool,
) -> Result<Vec<Reached>> {
    if !deep(query) || games.is_empty() {
        return Ok(games);
    }
    let wanted = Wanted::of(query)?;
    let file = db.path().filter(|path| !path.is_empty()).map(Path::new);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = games.len().div_ceil(workers).max(1);
    let parts: Vec<&[Reached]> = games.chunks(chunk).collect();
    let wanted = &wanted;
    let answers: Vec<Result<Vec<bool>>> = match file {
        // A file on disk is read by each worker on its own connection; one in memory (tests) by
        // the caller's alone.
        Some(file) if parts.len() > 1 => std::thread::scope(|scope| {
            let running: Vec<_> = parts
                .iter()
                .map(|part| {
                    scope.spawn(move || keep(&schema::open_read(file)?, part, wanted, stop))
                })
                .collect();
            running
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|_| Err(CoreError::new("The search failed.")))
                })
                .collect()
        }),
        _ => parts
            .iter()
            .map(|part| keep(db, part, wanted, stop))
            .collect(),
    };
    if stop.load(Ordering::Relaxed) {
        return Err(CoreError::aborted(STOPPED));
    }
    let mut kept = Vec::new();
    for (part, answer) in parts.iter().zip(answers) {
        kept.extend(
            part.iter()
                .zip(answer?)
                .filter(|(_, keep)| *keep)
                .map(|(game, _)| *game),
        );
    }
    Ok(kept)
}

/// Which of `part` (in id order) `wanted` keeps, read from `db`.
fn keep(
    db: &Connection,
    part: &[Reached],
    wanted: &Wanted,
    stop: &AtomicBool,
) -> Result<Vec<bool>> {
    let (Some(first), Some(last)) = (part.first(), part.last()) else {
        return Ok(Vec::new());
    };
    let mut select = db
        .prepare_cached(
            "SELECT id, fen, moves, extra FROM games WHERE id BETWEEN ?1 AND ?2 ORDER BY id",
        )
        .sql()?;
    let mut rows = select
        .query([SqlValue::Integer(first.id), SqlValue::Integer(last.id)])
        .sql()?;
    let mut kept = vec![false; part.len()];
    let (mut at, mut read) = (0, 0);
    while let Some(row) = rows.next().sql()? {
        let id: i64 = row.get(0).sql()?;
        while at < part.len() && part[at].id < id {
            at += 1;
        }
        if at == part.len() {
            break;
        }
        if part[at].id != id {
            continue;
        }
        read += 1;
        if read % BATCH == 0 && stop.load(Ordering::Relaxed) {
            return Err(CoreError::aborted(STOPPED));
        }
        let fen: Option<String> = row.get(1).sql()?;
        let moves = row.get_ref(2).sql()?.as_blob().unwrap_or_default();
        let extra = row.get_ref(3).sql()?.as_blob_or_null().ok().flatten();
        kept[at] = wanted.keeps(fen.as_deref(), moves, extra);
    }
    Ok(kept)
}
