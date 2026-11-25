//! Where a line leaves the games of the game databases: its first move that no game played from
//! its position, as ChessBase marks a novelty. Each position of the line is looked up in every
//! database at once (`explore::moves_from`, at any depth, transpositions included), so the line
//! is known as far as any of them knows it.
//!
//! The line's own game is not counted against it: the game it was opened from, and a copy of it
//! (the same players and moves) in any database. Each played its own moves where its line and
//! the board's agree, so it is taken off there, one game a ply.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, OptionalExtension as _, params};
use shakmaty::Move;
use shakmaty::variant::VariantPosition;

use super::{explore, search};
use crate::store::sql::Sql as _;
use kchess_contracts::error::Result;
use kchess_contracts::types::{GameDbNovelty, NoveltyMove};
use kchess_domain::gamedb;
use kchess_domain::rules;

/// The plies of a line looked at: a game known past its hundredth move is known.
pub const PLIES: usize = 200;
/// The other moves an answer names.
const PLAYED: usize = 3;

/// A line from the board, with what tells its own game.
pub struct Line {
    pub fen: String,
    pub moves: Vec<String>,
    pub white: Option<String>,
    pub black: Option<String>,
    /// The database and game it was opened from.
    pub from: Option<(String, u64)>,
}

/// The line's start and moves, played out; None for a line this looks nothing up for (not
/// standard chess, or a move that is not legal).
fn played(line: &Line) -> Option<(VariantPosition, Vec<VariantPosition>, Vec<Move>)> {
    let start = rules::standard_from_fen(&line.fen)?;
    let mut pos = start.clone();
    let (mut positions, mut moves) = (Vec::new(), Vec::new());
    for uci in line.moves.iter().take(PLIES) {
        let m = rules::legal_move(&pos, rules::parse_uci(uci)?)?;
        positions.push(pos.clone());
        moves.push(m);
        shakmaty::Position::play_unchecked(&mut pos, m);
    }
    Some((start, positions, moves))
}

/// The moves of the games in `db` that are the line's own: game `game` when given, and every
/// game of its players with its very moves.
fn own_games(
    db: &Connection,
    start: &VariantPosition,
    moves: &[Move],
    line: &Line,
    game: Option<u64>,
) -> Result<Vec<Vec<Move>>> {
    let visible = search::visible(db)?;
    let mut rows: Vec<(Option<String>, Vec<u8>)> = Vec::new();
    if let Some(game) = game {
        rows.extend(
            db.prepare_cached("SELECT fen, moves FROM games WHERE id = ?1 AND import_id <= ?2")
                .sql()?
                .query_row(params![game as i64, visible], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()
                .sql()?,
        );
    }
    if let (Some(white), Some(black), Some(code)) =
        (&line.white, &line.black, gamedb::encode_line(start, moves))
    {
        let mut copies = db
            .prepare_cached(
                "SELECT g.fen, g.moves FROM games g
                 JOIN players w ON w.id = g.white JOIN players b ON b.id = g.black
                 WHERE w.name = ?1 AND b.name = ?2 AND g.moves = ?3 AND g.import_id <= ?4
                   AND (?5 IS NULL OR g.id != ?5)",
            )
            .sql()?;
        let found = copies
            .query_map(
                params![white, black, code, visible, game.map(|game| game as i64)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .sql()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .sql()?;
        rows.extend(found);
    }
    Ok(rows
        .into_iter()
        .filter_map(|(fen, code)| {
            let from = gamedb::start_of(fen.as_deref())?;
            // A game from another start shares none of the line's positions by its moves.
            (rules::make_fen(&from) == rules::make_fen(start))
                .then(|| gamedb::decode_line(&from, &code))
                .flatten()
        })
        .collect())
}

/// The line's first move no game of `databases` (each its id and connection) played.
pub fn find(
    databases: &[(String, Connection)],
    line: &Line,
    stop: &AtomicBool,
) -> Result<GameDbNovelty> {
    let Some((start, positions, moves)) = played(line) else {
        return Ok(GameDbNovelty::default());
    };
    let mut own = Vec::new();
    for (id, db) in databases {
        let game = line
            .from
            .as_ref()
            .filter(|(from, _)| from == id)
            .map(|(_, game)| *game);
        own.extend(own_games(db, &start, &moves, line, game)?);
    }
    for (ply, (pos, played)) in positions.iter().zip(&moves).enumerate() {
        let mut counts: HashMap<u16, u64> = HashMap::new();
        for (_, db) in databases {
            for found in explore::moves_from(db, pos, stop)? {
                *counts.entry(found.mv).or_default() += found.stats.games();
            }
        }
        // The line's own game played here what its own line played.
        for other in &own {
            if other.len() > ply && other[..ply] == moves[..ply] {
                let mv = gamedb::pack_move(&other[ply]);
                if let Some(count) = counts.get_mut(&mv) {
                    *count = count.saturating_sub(1);
                }
            }
        }
        let new = gamedb::pack_move(played);
        if counts.get(&new).copied().unwrap_or(0) > 0 {
            continue;
        }
        let mut others: Vec<(u16, u64)> = counts
            .into_iter()
            .filter(|(mv, games)| *mv != new && *games > 0)
            .collect();
        others.sort_by_key(|(mv, games)| (std::cmp::Reverse(*games), *mv));
        let games = others.iter().map(|(_, games)| games).sum();
        // No game started from the line's start: it is no novelty, the databases lack it.
        if ply == 0 && games == 0 {
            break;
        }
        let played = others
            .into_iter()
            .take(PLAYED)
            .filter_map(|(mv, games)| {
                let m = gamedb::unpack_move(pos, mv)?;
                Some(NoveltyMove {
                    uci: rules::make_uci(&m),
                    san: rules::san_and_play(&mut pos.clone(), m),
                    games,
                })
            })
            .collect();
        return Ok(GameDbNovelty {
            ply: Some(ply as u32),
            games,
            played,
        });
    }
    Ok(GameDbNovelty::default())
}
