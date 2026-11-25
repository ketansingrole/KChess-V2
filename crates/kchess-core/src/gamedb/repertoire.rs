//! A repertoire checked against the games of the databases (`gameDbRepertoire`): each position
//! where it is the repertoire's side to move is looked up in every database at once, as the
//! explorer and the novelty mark look a position up (`explore::moves_from`: at any depth,
//! transpositions included), and each of the repertoire's moves there is judged by the games
//! (`kchess_domain::training::repertoire_check`).
//!
//! Through your own games (`gameDbRepertoireGames`) it is the other way round: each of Your
//! games where you had the repertoire's side is replayed through the repertoire's positions,
//! both sides', found by position (so a transposition stays in it), until a move it does not
//! have: one of yours, where it plays another, or your opponent's, which it has no answer to.
//! Your games are a few thousand, so replaying each once costs less than a lookup a position.
//!
//! A repertoire is built from the games too (`gameDbRepertoireBuild`): the opening moves of
//! one player's games with one side counted move by move, their most played move kept where it
//! is their turn and the other side's common replies where it is not.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use shakmaty::Color;

use super::tree::Stats;
use super::{explore, search};
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    GameDbRepertoireCheck, GameDbRepertoireGames, GameDbRepertoireGap, GameDbRepertoireMove,
    GameDbVerdict, RepertoireStep,
};
use kchess_domain::gamedb::{self, Outcome};
use kchess_domain::rules;
use kchess_domain::training::repertoire_check::{self, Counts, Verdict};

/// The other moves an answer names beside the repertoire's.
pub const OTHERS: usize = 3;
/// The most places your games left a repertoire that an answer lists.
pub const MOST_GAPS: usize = 100;
/// How deep a built repertoire goes, in half-moves.
pub const BUILT_PLIES: usize = 20;
/// The fewest games a move of a built repertoire was played in.
pub const BUILT_GAMES: u64 = 2;
/// The most replies of the other side a built repertoire keeps at a position.
pub const BUILT_REPLIES: usize = 4;
/// The smallest share of a position's games, in percent, a reply it keeps was played in.
pub const BUILT_SHARE: u64 = 5;
/// The most moves a built repertoire holds.
pub const BUILT_MOVES: usize = 2_000;

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

/// Where your games left a repertoire: a step and a move, with its games so far.
struct Gap {
    step: usize,
    found: GameDbRepertoireGap,
    /// The last game's `date_key`.
    last: i64,
}

/// The games of Your games (`db`) where one of `names` (in any case) had `color`, each replayed
/// through `steps` until it left them; a game that does not start in them goes in at the first
/// of their positions it reaches. At most `MOST_GAPS` places, the most played first.
pub fn your_games(
    db: &Connection,
    steps: &[RepertoireStep],
    color: Color,
    names: &[String],
    stop: &AtomicBool,
) -> Result<GameDbRepertoireGames> {
    let (games, reached, gaps) = follow(db, steps, color, names, color, stop)?;
    Ok(GameDbRepertoireGames {
        games,
        reached,
        gaps,
    })
}

/// The games in `db` of `name` with the side the repertoire (`color`) meets, each replayed
/// through `steps` as `your_games` does: each move they chose where the repertoire waits for
/// theirs, the ones it has no answer to first, the most played first.
pub fn player_games(
    db: &Connection,
    steps: &[RepertoireStep],
    color: Color,
    name: &str,
    stop: &AtomicBool,
) -> Result<GameDbRepertoireGames> {
    let (games, reached, gaps) = follow(db, steps, color, &[name.to_string()], !color, stop)?;
    Ok(GameDbRepertoireGames {
        games,
        reached,
        gaps,
    })
}

/// The games of `db` where one of `names` (in any case) had `side`, each replayed through
/// `steps`, the repertoire of `color`, until it left them. With `side` the repertoire's, every
/// move that left it is counted; with the other side, every move that side chose where the
/// repertoire waits for its reply, in it or not, and only its own moves that left it. Results are
/// for `color`. Answers the games, those that reached the repertoire, and at most `MOST_GAPS`
/// moves: those it lacks first, the most played first.
fn follow(
    db: &Connection,
    steps: &[RepertoireStep],
    color: Color,
    names: &[String],
    side: Color,
    stop: &AtomicBool,
) -> Result<(u64, u64, Vec<GameDbRepertoireGap>)> {
    let names: HashSet<String> = names
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    let choices = side != color;
    // Each position's first step and the repertoire's moves there, packed, from every step
    // that reaches it.
    let mut at: HashMap<u64, (usize, Vec<u16>)> = HashMap::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(pos) = rules::standard_from_fen(&step.fen) else {
            continue;
        };
        let (_, book) = at
            .entry(gamedb::position_key(&pos))
            .or_insert_with(|| (index, Vec::new()));
        for uci in &step.moves {
            if let Some(m) = rules::parse_uci(uci).and_then(|m| rules::legal_move(&pos, m)) {
                let mv = gamedb::pack_move(&m);
                if !book.contains(&mv) {
                    book.push(mv);
                }
            }
        }
    }
    let mut statement = db
        .prepare(
            "SELECT w.name, b.name, g.fen, g.moves, g.result, g.date, g.date_key FROM games g
             JOIN players w ON w.id = g.white JOIN players b ON b.id = g.black
             WHERE g.import_id <= ?1",
        )
        .sql()?;
    let mut rows = statement.query([search::visible(db)?]).sql()?;
    let (mut games, mut reached) = (0, 0);
    let mut gaps: HashMap<(usize, u16), Gap> = HashMap::new();
    while let Some(row) = rows.next().sql()? {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("The repertoire check stopped."));
        }
        let player: String = row.get(if side == Color::White { 0 } else { 1 }).sql()?;
        if !names.contains(&player.trim().to_lowercase()) {
            continue;
        }
        games += 1;
        let fen: Option<String> = row.get(2).sql()?;
        let code: Vec<u8> = row.get(3).sql()?;
        let Some((mut pos, moves)) = gamedb::start_of(fen.as_deref())
            .and_then(|start| Some((start.clone(), gamedb::decode_line(&start, &code)?)))
        else {
            continue;
        };
        // The moves counted: each its step, the position it was played from, the move and
        // whether the repertoire has it.
        let mut counted = Vec::new();
        let mut inside = false;
        for m in moves {
            match at.get(&gamedb::position_key(&pos)) {
                Some((step, book)) => {
                    inside = true;
                    let prepared = book.contains(&gamedb::pack_move(&m));
                    let theirs = shakmaty::Position::turn(&pos) == side;
                    if (choices && theirs) || (!choices && !prepared) {
                        counted.push((*step, pos.clone(), m, prepared));
                    }
                    if !prepared {
                        break;
                    }
                }
                // Out of the repertoire's positions after being in them: past its end.
                None if inside => break,
                None => {}
            }
            shakmaty::Position::play_unchecked(&mut pos, m);
        }
        reached += u64::from(inside);
        if counted.is_empty() {
            continue;
        }
        let outcome = Outcome::from_score(row.get(4).sql()?);
        let (date, key): (Option<String>, i64) = (row.get(5).sql()?, row.get(6).sql()?);
        for (step, from, m, prepared) in counted {
            let gap = gaps
                .entry((step, gamedb::pack_move(&m)))
                .or_insert_with(|| Gap {
                    step,
                    found: GameDbRepertoireGap {
                        step: u32::try_from(step).unwrap_or(u32::MAX),
                        uci: rules::make_uci(&m),
                        san: rules::san_and_play(&mut from.clone(), m),
                        yours: shakmaty::Position::turn(&from) == color,
                        prepared,
                        ..GameDbRepertoireGap::default()
                    },
                    last: i64::MIN,
                });
            gap.found.games += 1;
            match (outcome, color) {
                (Outcome::White, Color::White) | (Outcome::Black, Color::Black) => {
                    gap.found.won += 1
                }
                (Outcome::White, Color::Black) | (Outcome::Black, Color::White) => {
                    gap.found.lost += 1
                }
                (Outcome::Draw, _) => gap.found.drawn += 1,
                (Outcome::Unknown, _) => {}
            }
            if key > gap.last {
                gap.last = key;
                gap.found.last = date.clone();
            }
        }
    }
    let mut gaps: Vec<Gap> = gaps.into_values().collect();
    gaps.sort_by_key(|gap| {
        (
            gap.found.prepared,
            std::cmp::Reverse(gap.found.games),
            std::cmp::Reverse(gap.last),
            gap.step,
            gap.found.uci.clone(),
        )
    });
    let gaps = gaps
        .into_iter()
        .take(MOST_GAPS)
        .map(|gap| gap.found)
        .collect();
    Ok((games, reached, gaps))
}

/// The games through a position of a built repertoire, and those through each move from it.
#[derive(Default)]
struct Counted {
    games: u64,
    next: HashMap<u16, Counted>,
}

/// A repertoire of `color` from the games in `db` where one of `names` (in any case) had it,
/// from the standard start: at most `BUILT_PLIES` deep, each move played in `BUILT_GAMES` games
/// at least. At their turn their most played move; at the other side's, its replies played in
/// `BUILT_SHARE` percent of the games there, at most `BUILT_REPLIES`, the most played first (the
/// main line). Answers the games and the tree, at most `BUILT_MOVES` moves.
pub fn build(
    db: &Connection,
    color: Color,
    names: &[String],
    stop: &AtomicBool,
) -> Result<(u64, kchess_domain::tree::TreeNode)> {
    let names: HashSet<String> = names
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    let start = rules::default_position(shakmaty::variant::Variant::Chess);
    let start_key = gamedb::position_key(&start);
    let mut statement = db
        .prepare(
            "SELECT w.name, b.name, g.fen, g.moves FROM games g
             JOIN players w ON w.id = g.white JOIN players b ON b.id = g.black
             WHERE g.import_id <= ?1",
        )
        .sql()?;
    let mut rows = statement.query([search::visible(db)?]).sql()?;
    let mut games = 0;
    let mut root = Counted::default();
    while let Some(row) = rows.next().sql()? {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("Making the repertoire stopped."));
        }
        let player: String = row.get(if color == Color::White { 0 } else { 1 }).sql()?;
        if !names.contains(&player.trim().to_lowercase()) {
            continue;
        }
        let fen: Option<String> = row.get(2).sql()?;
        let code: Vec<u8> = row.get(3).sql()?;
        // Only games from the standard start: a repertoire starts there.
        let Some((_, moves)) = gamedb::start_of(fen.as_deref())
            .filter(|from| gamedb::position_key(from) == start_key)
            .and_then(|from| Some(((), gamedb::decode_line(&from, &code)?)))
        else {
            continue;
        };
        games += 1;
        let mut at = &mut root;
        at.games += 1;
        for m in moves.iter().take(BUILT_PLIES) {
            at = at.next.entry(gamedb::pack_move(m)).or_default();
            at.games += 1;
        }
    }
    let mut tree = kchess_domain::tree::TreeNode::new(None);
    let mut moves = 0;
    grow(&mut tree, "", &start, &root, color, &mut moves);
    Ok((games, tree))
}

/// Adds the moves `counted` keeps from `pos` (at `path` of `tree`) and the lines after them.
fn grow(
    tree: &mut kchess_domain::tree::TreeNode,
    path: &str,
    pos: &shakmaty::variant::VariantPosition,
    counted: &Counted,
    color: Color,
    moves: &mut usize,
) {
    let mut next: Vec<(u16, &Counted)> = counted
        .next
        .iter()
        .filter(|(_, after)| after.games >= BUILT_GAMES)
        .map(|(mv, after)| (*mv, after))
        .collect();
    next.sort_by_key(|(mv, after)| (std::cmp::Reverse(after.games), *mv));
    if shakmaty::Position::turn(pos) == color {
        next.truncate(1);
    } else {
        next.retain(|(_, after)| after.games * 100 >= counted.games * BUILT_SHARE);
        next.truncate(BUILT_REPLIES);
    }
    for (mv, after) in next {
        if *moves >= BUILT_MOVES {
            return;
        }
        let Some(m) = gamedb::unpack_move(pos, mv) else {
            continue;
        };
        let Some(child) = tree.add_move(path, &rules::make_uci(&m)) else {
            continue;
        };
        *moves += 1;
        let mut played = pos.clone();
        shakmaty::Position::play_unchecked(&mut played, m);
        grow(tree, &child, &played, after, color, moves);
    }
}
