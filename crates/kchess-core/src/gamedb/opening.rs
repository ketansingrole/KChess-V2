//! A position's opening report through every game database at once (`gameDbOpeningReport`): how
//! its games ended, its main lines, each the most played move after move from one of its moves,
//! and its strongest games. Each position is looked up as the explorer and the novelty finder
//! look it up (`explore::moves_from`), at any depth and by any move order.
//!
//! The strongest games are those the opening tree keeps as each move's best rated: through every
//! move from the position and every move the main lines meet. The strongest of all is always
//! among them; listing every game that reached a position near the start would replay a large
//! database's every game.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use rusqlite::Connection;
use shakmaty::Position as _;
use shakmaty::variant::VariantPosition;

use super::{explore, search};
use crate::store::sql::Sql as _;
use kchess_contracts::error::Result;
use kchess_contracts::types::{
    GameDbFoundGame, GameDbOpeningLine, GameDbOpeningReport, NoveltyMove,
};
use kchess_domain::gamedb;
use kchess_domain::rules;

/// The main lines a report gives, one from each of the most played moves.
pub const LINES: usize = 5;
/// A main line's longest, in plies.
pub const PLIES: usize = 12;
/// The strongest games a report lists.
pub const BEST: usize = 5;

/// One move from a position, through every database.
struct Merged {
    mv: u16,
    white: u64,
    draws: u64,
    black: u64,
    /// Each database's highest rated game through it: its rating, the database and the game.
    best: Vec<(u16, usize, i64)>,
}

impl Merged {
    fn games(&self) -> u64 {
        self.white + self.draws + self.black
    }
}

/// The moves played from `pos` in all of `databases`, the most played first.
fn moves_at(
    databases: &[(String, Connection)],
    pos: &VariantPosition,
    stop: &AtomicBool,
) -> Result<Vec<Merged>> {
    let mut merged: HashMap<u16, Merged> = HashMap::new();
    for (index, (_, db)) in databases.iter().enumerate() {
        for found in explore::moves_from(db, pos, stop)? {
            let entry = merged.entry(found.mv).or_insert_with(|| Merged {
                mv: found.mv,
                white: 0,
                draws: 0,
                black: 0,
                best: Vec::new(),
            });
            entry.white += u64::from(found.stats.white);
            entry.draws += u64::from(found.stats.draws);
            entry.black += u64::from(found.stats.black);
            if found.stats.best_elo > 0 {
                entry
                    .best
                    .push((found.stats.best_elo, index, found.stats.best_game));
            }
        }
    }
    let mut moves: Vec<Merged> = merged
        .into_values()
        .filter(|merged| merged.games() > 0)
        .collect();
    moves.sort_by_key(|merged| (std::cmp::Reverse(merged.games()), merged.mv));
    Ok(moves)
}

/// The move `mv` from `pos` in SAN and UCI, and the position after it.
fn played(pos: &VariantPosition, mv: u16, games: u64) -> Option<(NoveltyMove, VariantPosition)> {
    let m = gamedb::unpack_move(pos, mv)?;
    let mut after = pos.clone();
    let san = rules::san_and_play(&mut after, m);
    Some((
        NoveltyMove {
            san,
            uci: rules::make_uci(&m),
            games,
        },
        after,
    ))
}

/// The report on `fen` through `databases`; empty for a position that is not standard chess.
pub fn report(
    databases: &[(String, Connection)],
    fen: &str,
    stop: &AtomicBool,
) -> Result<GameDbOpeningReport> {
    let Some(start) = rules::standard_from_fen(fen) else {
        return Ok(GameDbOpeningReport::default());
    };
    let root = moves_at(databases, &start, stop)?;
    let mut report = GameDbOpeningReport {
        white: root.iter().map(|merged| merged.white).sum(),
        draws: root.iter().map(|merged| merged.draws).sum(),
        black: root.iter().map(|merged| merged.black).sum(),
        ..GameDbOpeningReport::default()
    };
    let mut best: Vec<(u16, usize, i64)> = root
        .iter()
        .flat_map(|merged| merged.best.iter().copied())
        .collect();
    for first in root.iter().take(LINES) {
        let Some((first_move, mut pos)) = played(&start, first.mv, first.games()) else {
            continue;
        };
        let mut moves = vec![first_move];
        // Followed while the most played move stays the line's: played in a twentieth of its
        // games at least, and in two games.
        let enough = (first.games() / 20).max(2);
        while moves.len() < PLIES && !pos.is_game_over() {
            let next = moves_at(databases, &pos, stop)?;
            best.extend(next.iter().flat_map(|merged| merged.best.iter().copied()));
            let Some(top) = next.first().filter(|top| top.games() >= enough) else {
                break;
            };
            let Some((next_move, after)) = played(&pos, top.mv, top.games()) else {
                break;
            };
            moves.push(next_move);
            pos = after;
        }
        report.lines.push(GameDbOpeningLine {
            moves,
            white: first.white,
            draws: first.draws,
            black: first.black,
        });
    }
    best.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| (a.1, a.2).cmp(&(b.1, b.2))));
    best.dedup_by_key(|(_, database, game)| (*database, *game));
    for (_, index, game) in best.into_iter().take(BEST) {
        let (id, db) = &databases[index];
        let row = db
            .prepare_cached(&format!(
                "SELECT {} FROM games g {} WHERE g.id = ?1",
                search::COLUMNS,
                search::JOINS
            ))
            .sql()?
            .query_row([game], search::row)
            .sql()?;
        report.best.push(GameDbFoundGame {
            database: id.clone(),
            game: row,
        });
    }
    Ok(report)
}
