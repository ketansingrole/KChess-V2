//! A repertoire checked against the games of the databases (`gameDbRepertoire`): each position
//! where it is the repertoire's side to move is looked up in every database at once, as the
//! explorer and the novelty mark look a position up (`explore::moves_from`: at any depth,
//! transpositions included), and each of the repertoire's moves there is judged by the games
//! (`kchess_domain::training::repertoire_check`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use shakmaty::Color;

use super::explore;
use super::tree::Stats;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    GameDbRepertoireCheck, GameDbRepertoireMove, GameDbVerdict, RepertoireStep,
};
use kchess_domain::gamedb;
use kchess_domain::rules;
use kchess_domain::training::repertoire_check::{self, Counts, Verdict};

/// The other moves an answer names beside the repertoire's.
pub const OTHERS: usize = 3;

fn counts(stats: &Stats) -> Counts {
    Counts {
        games: stats.games(),
        white: u64::from(stats.white),
        draws: u64::from(stats.draws),
        black: u64::from(stats.black),
    }
}

fn verdict(verdict: Verdict) -> GameDbVerdict {
    match verdict {
        Verdict::Played => GameDbVerdict::Played,
        Verdict::Unplayed => GameDbVerdict::Unplayed,
        Verdict::Rare => GameDbVerdict::Rare,
        Verdict::Weaker => GameDbVerdict::Weaker,
    }
}

/// Each of `steps` through `databases` (each its id and connection), in order. A position that
/// is not legal standard chess answers no games.
pub fn check(
    databases: &[(String, Connection)],
    steps: &[RepertoireStep],
    stop: &AtomicBool,
) -> Result<Vec<GameDbRepertoireCheck>> {
    let mut checks = Vec::with_capacity(steps.len());
    for step in steps {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("The repertoire check stopped."));
        }
        let Some(pos) = rules::standard_from_fen(&step.fen) else {
            checks.push(GameDbRepertoireCheck {
                fen: step.fen.clone(),
                ..GameDbRepertoireCheck::default()
            });
            continue;
        };
        let mut played: HashMap<u16, Stats> = HashMap::new();
        for (_, db) in databases {
            for found in explore::moves_from(db, &pos, stop)? {
                played.entry(found.mv).or_default().add(&found.stats);
            }
        }
        let total: u64 = played.values().map(Stats::games).sum();
        let white = shakmaty::Position::turn(&pos) == Color::White;
        let describe = |mv: u16, stats: &Stats| -> Option<GameDbRepertoireMove> {
            let m = gamedb::unpack_move(&pos, mv)?;
            Some(GameDbRepertoireMove {
                uci: rules::make_uci(&m),
                san: rules::san_and_play(&mut pos.clone(), m),
                games: stats.games(),
                white: u64::from(stats.white),
                draws: u64::from(stats.draws),
                black: u64::from(stats.black),
                rating: (stats.elo_n > 0)
                    .then(|| u32::try_from(stats.elo_sum / u64::from(stats.elo_n)).ok())
                    .flatten(),
                verdict: None,
            })
        };
        // The repertoire's moves as packed, each once; a move that is not legal is left out.
        let mut yours: Vec<u16> = Vec::new();
        for uci in &step.moves {
            let Some(m) = rules::parse_uci(uci).and_then(|m| rules::legal_move(&pos, m)) else {
                continue;
            };
            let mv = gamedb::pack_move(&m);
            if !yours.contains(&mv) {
                yours.push(mv);
            }
        }
        let mut others: Vec<(u16, &Stats)> = played
            .iter()
            .filter(|(mv, _)| !yours.contains(mv))
            .map(|(mv, stats)| (*mv, stats))
            .collect();
        others.sort_by_key(|(mv, stats)| (std::cmp::Reverse(stats.games()), *mv));
        let none = Stats::default();
        let mut moves = Vec::new();
        for &mv in &yours {
            let stats = played.get(&mv).unwrap_or(&none);
            // The main alternative: the most played move the repertoire does not play.
            let main = others.first().map(|(_, stats)| counts(stats));
            let judged = repertoire_check::verdict(counts(stats), main, total, white);
            moves.extend(describe(mv, stats).map(|described| GameDbRepertoireMove {
                verdict: Some(verdict(judged)),
                ..described
            }));
        }
        moves.extend(
            others
                .iter()
                .take(OTHERS)
                .filter_map(|(mv, stats)| describe(*mv, stats)),
        );
        checks.push(GameDbRepertoireCheck {
            fen: step.fen.clone(),
            games: total,
            moves,
        });
    }
    Ok(checks)
}
