//! A position looked up in a game database, as the explorer shows Lichess's: the moves played
//! from it with their results, and its best-rated games.
//!
//! A position is found at any depth, in two parts summed. A game's first `index_depth` plies: with
//! no filter the opening tree answers, one indexed read. Past them: the games the position index
//! (`deep`) says passed through the position's structure are replayed, a handful of games.
//!
//! With filters (a player, the side they had, years, a rating) the first part is replayed too:
//! the tree counts every game, so it cannot tell a player's. However many games match, every one
//! is replayed up to `index_depth` (and only while it can still reach the position by then), on
//! every core, from the replay index (`games_replay`: a game's moves and what a filter checks,
//! a tenth of its row):
//!
//! - A filter that picks out a few games (a player, a high rating) finds their ids by its own
//!   index; this thread reads them while the other cores replay them a batch at a time.
//! - A broad one (years, a rating most games have) reads the whole index in order instead, which
//!   is faster than reaching most games one by one: each core reads and replays its own stretch.
//!
//! A batch's games are replayed in the order of their moves, so the plies a game shares with the
//! one before it are not played again: an opening is replayed once, not once a game. A game stops
//! once it can no longer reach the position (it has fewer pieces, has moved a pawn the position
//! still has at home, or has lost a castling right the position still has).
//!
//! Counts are summed alike in any order, and of games rated alike the first is the best
//! (`Stats::add`), so either way the answer is the one replaying every game in turn would give.
//! A lookup its caller dropped stops between batches.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension as _, params_from_iter};
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Board, Color, Move, Position as _, Rank, Role};

use super::tree::{self, Stats, TreeMove};
use super::{deep, schema, search};
use crate::store::sql::Sql as _;
use crate::tasks::lock;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    ExplorerGame, GameDbPage, GameDbQuery, GameDbSide, GameDbSort, LookupMove, LookupOptions,
    PositionLookupKind, SavedLookup,
};
use kchess_domain::gamedb::{self, Outcome};
use kchess_domain::rules;

/// The best-rated games a lookup lists.
const TOP_GAMES: usize = 8;
/// Games replayed as one piece of work: enough that handing it over costs little beside it.
const BATCH: usize = 1024;
/// A rating filter that more than this share of the games pass is broad: reading the whole
/// replay index in order then costs less than reaching each game by the rating index (on 1.3
/// million games, 0.03 µs a game read in order across the cores, 0.57 µs a game by the index).
const BROAD_SHARE: f64 = 0.05;
/// Stretches of the file per core in a broad lookup, so that one with more games to replay
/// does not keep the others waiting.
const STRETCHES_PER_CORE: i64 = 4;
/// The columns a replay reads of each game.
const REPLAYED: &str = "g.id, g.fen, g.moves, g.result, g.rating, g.date_key";
/// What a lookup its caller dropped answers.
const STOPPED: &str = "The lookup stopped.";

/// The search a lookup's filters make, or None when it has none and the tree answers. The side
/// is the player's, so it filters nothing without one.
fn query(database: &str, options: &LookupOptions) -> Option<GameDbQuery> {
    let year = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|text| text.get(..4))
            .and_then(|year| year.parse::<i32>().ok())
    };
    let player = options
        .player
        .as_deref()
        .map(str::trim)
        .filter(|player| !player.is_empty())
        .map(str::to_string);
    let side = player.as_ref().and(options.color).map(|color| match color {
        Color::White => GameDbSide::White,
        Color::Black => GameDbSide::Black,
    });
    let query = GameDbQuery {
        database: database.to_string(),
        player,
        side,
        year_from: year(&options.since),
        year_to: year(&options.until),
        min_elo: options.min_elo.filter(|&elo| elo > 0),
        ..GameDbQuery::default()
    };
    let filtered = query.player.is_some()
        || query.year_from.is_some()
        || query.year_to.is_some()
        || query.min_elo.is_some();
    filtered.then_some(query)
}

/// What a position has that no position after it in a game can get back once lost: its number
/// of pieces, each side's pawns still on their starting squares, and its castling rights.
struct Reachable {
    pieces: usize,
    white_home: Bitboard,
    black_home: Bitboard,
    castling: Bitboard,
}

impl Reachable {
    fn of(pos: &VariantPosition) -> Reachable {
        let board = pos.board();
        let pawns = board.by_role(Role::Pawn);
        Reachable {
            pieces: board.occupied().count(),
            white_home: pawns & board.by_color(Color::White) & Bitboard::from_rank(Rank::Second),
            black_home: pawns & board.by_color(Color::Black) & Bitboard::from_rank(Rank::Seventh),
            castling: pos.castles().castling_rights(),
        }
    }

    /// Whether the position can still follow `pos`: captures, pawn moves and lost castling
    /// rights are for good.
    fn from(&self, pos: &VariantPosition) -> bool {
        let board = pos.board();
        let pawns = board.by_role(Role::Pawn);
        board.occupied().count() >= self.pieces
            && pawns & board.by_color(Color::White) & self.white_home == self.white_home
            && pawns & board.by_color(Color::Black) & self.black_home == self.black_home
            && pos.castles().castling_rights() & self.castling == self.castling
    }
}

/// The position a replay looks for, as each replayed move is checked against it: the side to
/// move and the board first, which most positions fail, and its key (castling, en passant) only
/// when they match. Only a game's plies `from..until` count: the tree counts the first
/// `index_depth`, the position index the rest.
struct Target {
    key: u64,
    board: Board,
    turn: Color,
    reachable: Reachable,
    from: usize,
    until: usize,
}

impl Target {
    fn of(pos: &VariantPosition, from: usize, until: usize) -> Target {
        Target {
            key: gamedb::position_key(pos),
            board: pos.board().clone(),
            turn: pos.turn(),
            reachable: Reachable::of(pos),
            from,
            until,
        }
    }

    /// Whether a game at `pos` on ply `ply` can still reach the position on a ply that counts:
    /// one capture a ply at best.
    fn in_time(&self, pos: &VariantPosition, ply: usize) -> bool {
        let captures = pos
            .board()
            .occupied()
            .count()
            .saturating_sub(self.reachable.pieces);
        ply.saturating_add(captures) < self.until
    }

    fn is(&self, pos: &VariantPosition) -> bool {
        pos.turn() == self.turn
            && *pos.board() == self.board
            && gamedb::position_key(pos) == self.key
    }
}

/// A game as the replay reads it; its moves are `moves[start..end]` of its batch.
struct Game {
    id: i64,
    fen: Option<String>,
    start: usize,
    end: usize,
    result: Option<i64>,
    rating: Option<i64>,
    date: i64,
}

/// Games read together, replayed together, and counted in the order they were read (`index`).
#[derive(Default)]
struct Batch {
    index: usize,
    games: Vec<Game>,
    moves: Vec<u8>,
}

/// One ply of the line replayed last: the position before it, what the target makes of that
/// position, and the move the line played from it (its byte and move, once known).
struct Step {
    pos: VariantPosition,
    reachable: bool,
    hit: bool,
    played: Option<(u8, Move)>,
}

impl Step {
    fn at(pos: VariantPosition, ply: usize, target: &Target) -> Step {
        let reachable = target.reachable.from(&pos) && target.in_time(&pos, ply);
        Step {
            hit: reachable && target.is(&pos),
            reachable,
            pos,
            played: None,
        }
    }
}

/// Replays `code` along `path`, the plies of the line replayed before it from the same start,
/// adding to `played` each move it made from the target. The plies the two lines share are
/// read from `path` rather than played again; from the first that differs, `path` becomes this
/// line's. It stops where the position can no longer come, past the plies that count, or at a
/// damaged move.
fn follow(path: &mut Vec<Step>, code: &[u8], played: &mut Vec<u16>, target: &Target) {
    for (ply, &byte) in code.iter().enumerate() {
        if ply >= target.until {
            return;
        }
        let step = &mut path[ply];
        if !step.reachable {
            return;
        }
        let m = match step.played {
            Some((known, m)) if known == byte => m,
            _ => {
                let Some(m) = gamedb::coded_move(&step.pos, byte) else {
                    return;
                };
                step.played = Some((byte, m));
                path.truncate(ply + 1);
                m
            }
        };
        if path[ply].hit && ply >= target.from {
            played.push(gamedb::pack_move(&m));
        }
        if path.len() == ply + 1 {
            let mut next = path[ply].pos.clone();
            next.play_unchecked(m);
            path.push(Step::at(next, ply + 1, target));
        }
    }
}

/// What a replay gathers of the games that reached its target: the moves they played from it,
/// counted (the explorer), or the games themselves (a list of them).
trait Tally: Default + Send {
    /// `game` played `played` from the target, on the plies the target counts.
    fn reached(&mut self, game: &Game, played: &[u16]);
    /// Adds `later`, gathered from the games read after these.
    fn join(&mut self, later: Self);
}

impl Tally for HashMap<u16, Stats> {
    fn reached(&mut self, game: &Game, played: &[u16]) {
        let stats = Stats::of(
            game.id,
            Outcome::from_score(game.result),
            game.rating.and_then(|rating| u16::try_from(rating).ok()),
            u32::try_from(game.date).unwrap_or(0),
        );
        for &mv in played {
            self.entry(mv)
                .and_modify(|sum| sum.add(&stats))
                .or_insert(stats);
        }
    }

    fn join(&mut self, later: Self) {
        for (mv, stats) in later {
            self.entry(mv)
                .and_modify(|sum| sum.add(&stats))
                .or_insert(stats);
        }
    }
}

/// A game that reached a position, with what a list of them is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reached {
    pub id: i64,
    pub rating: Option<i64>,
    pub date: i64,
}

impl Tally for Vec<Reached> {
    fn reached(&mut self, game: &Game, _: &[u16]) {
        self.push(Reached {
            id: game.id,
            rating: game.rating,
            date: game.date,
        });
    }

    fn join(&mut self, later: Self) {
        self.extend(later);
    }
}

/// What each game of `batch` played from `target`, gathered. The games are replayed in the
/// order of their moves, so each shares the plies its line has in common with the one before
/// it: an opening a thousand games played is replayed once.
fn replay<T: Tally>(batch: &Batch, target: &Target) -> T {
    let line = |game: &Game| &batch.moves[game.start..game.end];
    let mut order: Vec<&Game> = batch.games.iter().collect();
    order.sort_unstable_by(|a, b| (&a.fen, line(a)).cmp(&(&b.fen, line(b))));
    let mut tally = T::default();
    let mut played = Vec::new();
    let mut path: Vec<Step> = Vec::new();
    let mut from: Option<&Option<String>> = None;
    for game in order {
        if from != Some(&game.fen) {
            path.clear();
            from = Some(&game.fen);
        }
        if path.is_empty() {
            let Some(start) = gamedb::start_of(game.fen.as_deref()) else {
                continue;
            };
            path.push(Step::at(start, 0, target));
        }
        played.clear();
        follow(&mut path, line(game), &mut played, target);
        if !played.is_empty() {
            tally.reached(game, &played);
        }
    }
    tally
}

/// Adds the game in `row` (read as `REPLAYED`) to `batch`.
fn read_game(row: &rusqlite::Row<'_>, batch: &mut Batch) -> Result<()> {
    let start = batch.moves.len();
    let moves = row.get_ref(2).sql()?.as_blob();
    batch.moves.extend_from_slice(
        moves.map_err(|e| CoreError::wrap("A game's moves could not be read", e))?,
    );
    batch.games.push(Game {
        id: row.get(0).sql()?,
        fen: row.get(1).sql()?,
        start,
        end: batch.moves.len(),
        result: row.get(3).sql()?,
        rating: row.get(4).sql()?,
        date: row.get(5).sql()?,
    });
    Ok(())
}

/// Whether the games `query` matches are better read in file order than by their index: no
/// player or event narrows them, and a rating, if any, is one most games have.
fn broad(db: &Connection, query: Option<&GameDbQuery>) -> Result<bool> {
    let Some(query) = query else {
        return Ok(true);
    };
    if query.player.is_some()
        || query.opponent.is_some()
        || query.event.is_some()
        || query.eco.is_some()
    {
        return Ok(false);
    }
    let Some(elo) = query.min_elo else {
        // Years have no index: the file is read whole anyway.
        return Ok(true);
    };
    // Both from indexes alone, in a few milliseconds.
    let passing: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM games WHERE low_elo >= ?1",
            [i64::from(elo)],
            |row| row.get(0),
        )
        .sql()?;
    let games: i64 = db
        .query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0))
        .sql()?;
    Ok(passing as f64 > games as f64 * BROAD_SHARE)
}

/// A replay's SQL, on the games as `g`. `narrow` picks the games out by an index (the position
/// index's games, a player, a rating few games have), as ids alone; `kept` is checked on the
/// columns the replay index has; `from` reads the games through that index when the file has it.
struct Filters {
    narrow: Option<String>,
    kept: String,
    args: Vec<SqlValue>,
    from: &'static str,
}

impl Filters {
    /// The games `query` matches (every finished game without one), among `ids` if given.
    fn of(
        db: &Connection,
        query: Option<&GameDbQuery>,
        ids: Option<&[i64]>,
        broad: bool,
    ) -> Result<Filters> {
        let empty = GameDbQuery::default();
        let query = query.unwrap_or(&empty);
        let mut args = vec![SqlValue::Integer(search::visible(db)?)];
        // An unfinished game counts in no result: the tree leaves it out, and so does this.
        let mut kept = vec![
            "g.import_id <= ?1".to_string(),
            "g.result IS NOT NULL".to_string(),
        ];
        let mut narrow = Vec::new();
        if let Some(ids) = ids {
            // Integers alone: nothing in them can reach the SQL as anything but a number.
            let listed: Vec<String> = ids.iter().map(i64::to_string).collect();
            narrow.push(format!("id IN ({})", listed.join(",")));
        }
        narrow.extend(search::named(query, search::worded(db)?, &mut args, ""));
        if let Some(eco) = &query.eco {
            args.push(SqlValue::Text(format!("{eco}%")));
            narrow.push(format!("eco LIKE ?{}", args.len()));
        }
        kept.extend(search::result(query).map(str::to_string));
        kept.extend(search::length(query, &mut args));
        for (year, test) in [(query.year_from, ">="), (query.year_to, "<=")] {
            if let Some(year) = year {
                args.push(SqlValue::Integer(i64::from(year)));
                kept.push(format!("g.year {test} ?{}", args.len()));
            }
        }
        if let Some(elo) = query.min_elo {
            args.push(SqlValue::Integer(i64::from(elo)));
            let n = args.len();
            kept.push(format!("g.low_elo >= ?{n}"));
            if !broad && narrow.is_empty() {
                narrow.push(format!("low_elo >= ?{n}"));
            }
        }
        let indexed: bool = db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master
                 WHERE type = 'index' AND name = 'games_replay')",
                [],
                |row| row.get(0),
            )
            .sql()?;
        Ok(Filters {
            narrow: (!narrow.is_empty()).then(|| narrow.join(" AND ")),
            kept: kept.join(" AND "),
            args,
            from: match (indexed, broad) {
                (true, _) => "INDEXED BY games_replay",
                // In id order, a stretch's ends found by the table's own key.
                (false, true) => "NOT INDEXED",
                (false, false) => "",
            },
        })
    }
}

/// Each batch's counts, by index: this thread reads the games `filters` pick out, the others
/// replay them.
fn by_index<T: Tally>(
    db: &Connection,
    filters: &Filters,
    target: &Target,
    stop: &AtomicBool,
    workers: usize,
) -> Result<Vec<(usize, T)>> {
    let (send, batches) = mpsc::sync_channel::<Batch>(workers * 2);
    let batches = Mutex::new(batches);
    let (answer, answers) = mpsc::channel();
    std::thread::scope(|scope| {
        // Owned here, so that leaving, however it leaves, closes them and the replays end.
        let (send, answer) = (send, answer);
        for _ in 0..workers {
            let (batches, answer) = (&batches, answer.clone());
            scope.spawn(move || {
                loop {
                    let next = lock(batches).recv();
                    let Ok(batch) = next else {
                        break;
                    };
                    if stop.load(Ordering::Relaxed) {
                        continue;
                    }
                    if answer
                        .send((batch.index, replay::<T>(&batch, target)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(answer);
        let Filters {
            narrow, kept, from, ..
        } = filters;
        // The ids first, from the header's index alone; then each game, in id order, from the
        // replay index.
        let sql = match narrow {
            Some(narrow) => format!(
                "SELECT {REPLAYED} FROM games g {from}
                 WHERE g.id IN (SELECT id FROM games WHERE {narrow}) AND {kept}"
            ),
            None => format!("SELECT {REPLAYED} FROM games g {from} WHERE {kept}"),
        };
        let mut select = db.prepare(&sql).sql()?;
        let mut rows = select.query(params_from_iter(filters.args.iter())).sql()?;
        let mut batch = Batch::default();
        while let Some(row) = rows.next().sql()? {
            read_game(row, &mut batch)?;
            if batch.games.len() == BATCH {
                if stop.load(Ordering::Relaxed) {
                    return Err(CoreError::aborted(STOPPED));
                }
                let index = batch.index + 1;
                let full = std::mem::replace(
                    &mut batch,
                    Batch {
                        index,
                        ..Batch::default()
                    },
                );
                if send.send(full).is_err() {
                    break;
                }
            }
        }
        if !batch.games.is_empty() {
            send.send(batch).ok();
        }
        drop(send);
        Ok(answers.iter().collect())
    })
}

/// Each stretch's counts, in file order: every core reads its own stretches of the file (`path`)
/// on a connection of its own, and replays the games in them that `filters` keep.
fn in_file_order<T: Tally>(
    db: &Connection,
    path: &std::path::Path,
    filters: &Filters,
    target: &Target,
    stop: &AtomicBool,
    workers: usize,
) -> Result<Vec<(usize, T)>> {
    // Apart: SQLite reads a lone MIN or MAX off the key's end, but scans the table for both.
    let end = |sql: &str| -> Result<Option<i64>> { db.query_row(sql, [], |row| row.get(0)).sql() };
    let (first, last) = (
        end("SELECT MIN(id) FROM games")?,
        end("SELECT MAX(id) FROM games")?,
    );
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(Vec::new());
    };
    let stretches = i64::try_from(workers).unwrap_or(1) * STRETCHES_PER_CORE;
    let span = (last - first) / stretches + 1;
    let next = AtomicUsize::new(0);
    let Filters {
        kept, args, from, ..
    } = filters;
    let (low, high) = (args.len() + 1, args.len() + 2);
    let sql = format!(
        "SELECT {REPLAYED} FROM games g {from} WHERE g.id BETWEEN ?{low} AND ?{high} AND {kept}"
    );
    let read = || -> Result<Vec<(usize, T)>> {
        let db = schema::open_read(path)?;
        let mut select = db.prepare(&sql).sql()?;
        let mut counted = Vec::new();
        loop {
            let stretch = next.fetch_add(1, Ordering::Relaxed);
            let start = first + i64::try_from(stretch).unwrap_or(i64::MAX / 2) * span;
            if start > last || stop.load(Ordering::Relaxed) {
                return Ok(counted);
            }
            let mut values = args.clone();
            values.push(SqlValue::Integer(start));
            values.push(SqlValue::Integer(start.saturating_add(span - 1)));
            let mut rows = select.query(params_from_iter(values.iter())).sql()?;
            let mut batch = Batch {
                index: stretch,
                ..Batch::default()
            };
            while let Some(row) = rows.next().sql()? {
                read_game(row, &mut batch)?;
                if batch.games.len().is_multiple_of(BATCH) && stop.load(Ordering::Relaxed) {
                    return Ok(counted);
                }
            }
            counted.push((stretch, replay::<T>(&batch, target)));
        }
    };
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..workers).map(|_| scope.spawn(read)).collect();
        let mut counted = Vec::new();
        for reader in readers {
            let part = reader
                .join()
                .map_err(|_| CoreError::new("The lookup failed."))??;
            counted.extend(part);
        }
        Ok(counted)
    })
}

/// What the games `query` matches (every finished game without one), among `ids` if given,
/// played from `target` on the plies it counts, gathered in the order the games were read: as
/// moves, counted as the tree counts them, each time a game reached it, the game's result once.
/// `stop` ends it early.
fn replayed<T: Tally>(
    db: &Connection,
    query: Option<&GameDbQuery>,
    ids: Option<&[i64]>,
    target: &Target,
    stop: &AtomicBool,
) -> Result<T> {
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let file = db
        .path()
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from);
    let mut counted = match file {
        Some(file) if ids.is_none() && broad(db, query)? => {
            let filters = Filters::of(db, query, None, true)?;
            in_file_order(db, &file, &filters, target, stop, workers)?
        }
        _ => {
            let filters = Filters::of(db, query, ids, false)?;
            by_index(db, &filters, target, stop, workers)?
        }
    };
    if stop.load(Ordering::Relaxed) {
        return Err(CoreError::aborted(STOPPED));
    }
    counted.sort_unstable_by_key(|(index, _)| *index);
    let mut tally = T::default();
    for (_, part) in counted {
        tally.join(part);
    }
    Ok(tally)
}

/// The plies of each game the opening tree holds (`meta.index_depth`).
fn index_depth(db: &Connection) -> Result<usize> {
    let depth: i64 = db
        .query_row("SELECT index_depth FROM meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .sql()?;
    Ok(usize::try_from(depth).unwrap_or(0))
}

/// The games to replay past the tree for `pos`: those the position index says passed through its
/// structure, or every game (`Some(None)`) while a file's index is not built yet, slower and
/// alike. None when no game did, so nothing needs replaying.
fn passed(db: &Connection, pos: &VariantPosition) -> Result<Option<Option<Vec<i64>>>> {
    if !deep::built(db)? {
        return Ok(Some(None));
    }
    let ids = deep::games(db, gamedb::structure_key(pos))?;
    Ok((!ids.is_empty()).then_some(Some(ids)))
}

/// The moves played from `pos` in the games `query` matches (every finished game without one),
/// at any depth: the first `index_depth` plies from the tree, or replayed when filtered; the
/// rest replayed from the games the position index says passed through its structure.
fn found(
    db: &Connection,
    query: Option<&GameDbQuery>,
    pos: &VariantPosition,
    stop: &AtomicBool,
) -> Result<Vec<TreeMove>> {
    let depth = index_depth(db)?;
    let tree = tree::moves(db, gamedb::position_key(pos))?;
    let mut moves: HashMap<u16, Stats> = match query {
        None => tree.into_iter().map(|m| (m.mv, m.stats)).collect(),
        // No game of the file reached it on the tree's plies (a position past the opening, most
        // often): none of those the filters keep did either, so nothing there needs replaying.
        Some(_) if tree.is_empty() => HashMap::new(),
        Some(_) => replayed(db, query, None, &Target::of(pos, 0, depth), stop)?,
    };
    if let Some(ids) = passed(db, pos)? {
        let later = Target::of(pos, depth, usize::MAX);
        moves.join(replayed(db, query, ids.as_deref(), &later, stop)?);
    }
    let mut moves: Vec<TreeMove> = moves
        .into_iter()
        .map(|(mv, stats)| TreeMove { mv, stats })
        .collect();
    moves.sort_by_key(|m| (Reverse(m.stats.games()), m.mv));
    Ok(moves)
}

/// The moves played from `pos` in every game of the database, at any depth.
pub fn moves_from(
    db: &Connection,
    pos: &VariantPosition,
    stop: &AtomicBool,
) -> Result<Vec<TreeMove>> {
    found(db, None, pos, stop)
}

/// The games `query` matches, in id order: those that reached its position, if it has one, and
/// that its deep filters keep (`scan`), if it has them.
fn reached(db: &Connection, query: &GameDbQuery, stop: &AtomicBool) -> Result<Vec<Reached>> {
    let games = if query.position.is_some() {
        reached_at(db, query, stop)?
    } else {
        super::scan::candidates(db, query)?
    };
    super::scan::kept(db, query, games, stop)
}

/// The games `query` matches that reached its position (`query.position`) at any depth, with a
/// move played from it: those the explorer counts there, each once, in id order.
fn reached_at(db: &Connection, query: &GameDbQuery, stop: &AtomicBool) -> Result<Vec<Reached>> {
    let Some(pos) = query.position.as_deref().and_then(rules::standard_from_fen) else {
        return Err(CoreError::new("That position is not legal."));
    };
    let depth = index_depth(db)?;
    // As for the explorer: no game the tree holds reached it in its plies, so no game the
    // filters keep did either.
    let mut games: Vec<Reached> = if tree::moves(db, gamedb::position_key(&pos))?.is_empty() {
        Vec::new()
    } else {
        replayed(db, Some(query), None, &Target::of(&pos, 0, depth), stop)?
    };
    if let Some(ids) = passed(db, &pos)? {
        let later = Target::of(&pos, depth, usize::MAX);
        games.join(replayed(db, Some(query), ids.as_deref(), &later, stop)?);
    }
    // A game that reached it both before and past the tree's plies is listed once.
    games.sort_unstable_by_key(|game| game.id);
    games.dedup_by_key(|game| game.id);
    Ok(games)
}

/// The games the last position search found, kept for its other pages and orders: they are
/// found again only for another search, or once the database changed.
#[derive(Default)]
pub struct Listed(Mutex<Option<(String, std::sync::Arc<Vec<Reached>>)>>);

impl Listed {
    /// Forgets the games kept: their database was removed or written afresh, and a file written
    /// again counts its imports from the first, so the key alone would not tell.
    pub fn forget(&self) {
        lock(&self.0).take();
    }
}

/// The id of every game `query` matches that reached its position or that its deep filters
/// keep, in id order.
pub fn ids_at(db: &Connection, query: &GameDbQuery, stop: &AtomicBool) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = reached(db, query, stop)?
        .into_iter()
        .map(|game| game.id)
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// One page of the games `query` matches that reached its position or that its deep filters
/// keep, in its order.
pub fn games_at(
    db: &Connection,
    query: &GameDbQuery,
    listed: &Listed,
    stop: &AtomicBool,
) -> Result<GameDbPage> {
    let search = GameDbQuery {
        sort: None,
        offset: 0,
        limit: 0,
        ..query.clone()
    };
    // A finished import adding to the database changes what it holds.
    let key = format!("{} {search:?}", search::visible(db)?);
    let kept = lock(&listed.0)
        .as_ref()
        .filter(|(at, _)| *at == key)
        .map(|(_, games)| std::sync::Arc::clone(games));
    let games = match kept {
        Some(games) => games,
        None => {
            let games = std::sync::Arc::new(reached(db, &search, stop)?);
            *lock(&listed.0) = Some((key, std::sync::Arc::clone(&games)));
            games
        }
    };
    let mut order: Vec<&Reached> = games.iter().collect();
    // As a search without a position orders its games.
    match query.sort.unwrap_or(GameDbSort::Newest) {
        GameDbSort::Newest => order.sort_unstable_by_key(|game| Reverse((game.date, game.id))),
        GameDbSort::Oldest => order.sort_unstable_by_key(|game| (game.date, game.id)),
        GameDbSort::Rating => {
            order.sort_unstable_by_key(|game| (Reverse(game.rating), Reverse(game.id)))
        }
    }
    let start = (query.offset as usize).min(order.len());
    let end = start.saturating_add(query.limit as usize).min(order.len());
    let mut select = db
        .prepare_cached(&format!(
            "SELECT {} FROM games g {} WHERE g.id = ?1",
            search::COLUMNS,
            search::JOINS
        ))
        .sql()?;
    let mut rows = Vec::with_capacity(end - start);
    for game in &order[start..end] {
        rows.extend(select.query_row([game.id], search::row).optional().sql()?);
    }
    Ok(GameDbPage {
        games: rows,
        total: order.len() as u64,
        offset: query.offset,
    })
}

/// One game as the explorer lists it, with the move it played from the position.
fn listed(db: &Connection, game: i64, san: String) -> Result<Option<ExplorerGame>> {
    let row = db
        .prepare_cached(&format!(
            "SELECT {} FROM games g {} WHERE g.id = ?1",
            search::COLUMNS,
            search::JOINS
        ))
        .sql()?
        .query_row([game], search::row)
        .optional()
        .sql()?;
    Ok(row.map(|row| ExplorerGame {
        id: game.to_string(),
        white: row.white,
        black: row.black,
        white_rating: row.white_elo.map(f64::from),
        black_rating: row.black_elo.map(f64::from),
        winner: match row.result.as_str() {
            "1-0" => Some(Color::White),
            "0-1" => Some(Color::Black),
            _ => None,
        },
        year: row
            .date
            .as_deref()
            .and_then(|date| gamedb::date_parts(date).0)
            .map(i64::from),
        month: None,
        san: Some(san),
    }))
}

/// The position `fen` looked up in database `database` (open as `db`) with `options`' filters;
/// `now` is the lookup's time. Set, `stop` ends a lookup that replays games early.
pub fn lookup(
    db: &Connection,
    database: &str,
    fen: &str,
    options: &LookupOptions,
    now: i64,
    stop: &AtomicBool,
) -> Result<SavedLookup> {
    let Some(pos) = rules::standard_from_fen(fen) else {
        return Err(CoreError::new("That position is not legal."));
    };
    let found = found(db, query(database, options).as_ref(), &pos, stop)?;
    let played: Vec<(Stats, Move, String)> = found
        .into_iter()
        .filter_map(|entry| {
            let m = gamedb::unpack_move(&pos, entry.mv)?;
            let san = rules::san_and_play(&mut pos.clone(), m);
            Some((entry.stats, m, san))
        })
        .collect();
    // Each move's best-rated game, best first; a game that is best for two moves is listed once.
    let mut best: Vec<(&Stats, &String)> = Vec::new();
    for (stats, _, san) in &played {
        if !best
            .iter()
            .any(|(other, _)| other.best_game == stats.best_game)
        {
            best.push((stats, san));
        }
    }
    best.sort_by_key(|(stats, _)| (Reverse(stats.best_elo), stats.best_game));
    let mut games = Vec::new();
    for (stats, san) in best.into_iter().take(TOP_GAMES) {
        games.extend(listed(db, stats.best_game, san.clone())?);
    }
    let total = played.iter().map(|(stats, ..)| stats.games()).sum::<u64>();
    Ok(SavedLookup {
        kind: PositionLookupKind::Database,
        fen: fen.to_string(),
        fetched_at: now,
        total: Some(i64::try_from(total).unwrap_or(i64::MAX)),
        opening: None,
        category: None,
        dtz: None,
        games: Some(games),
        moves: played
            .into_iter()
            .map(|(stats, m, san)| LookupMove {
                uci: rules::make_uci(&m),
                san,
                white: Some(i64::from(stats.white)),
                draws: Some(i64::from(stats.draws)),
                black: Some(i64::from(stats.black)),
                ..LookupMove::default()
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_make_a_search_and_none_leave_it_to_the_tree() {
        assert!(query("db", &LookupOptions::default()).is_none());
        let only_side = LookupOptions {
            color: Some(Color::Black),
            min_elo: Some(0),
            ..LookupOptions::default()
        };
        assert!(
            query("db", &only_side).is_none(),
            "a side alone filters nothing"
        );
        let prep = LookupOptions {
            player: Some(" Carlsen ".into()),
            color: Some(Color::Black),
            since: Some("2019-05".into()),
            until: Some("2024".into()),
            min_elo: Some(2600),
            ..LookupOptions::default()
        };
        let made = query("db", &prep).unwrap();
        assert_eq!(made.player.as_deref(), Some("Carlsen"));
        assert_eq!(made.side, Some(GameDbSide::Black));
        assert_eq!((made.year_from, made.year_to), (Some(2019), Some(2024)));
        assert_eq!(made.min_elo, Some(2600));
    }

    #[test]
    fn a_game_that_captured_or_moved_a_home_pawn_cannot_come_back() {
        let open = rules::standard_from_fen(
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
        )
        .unwrap();
        let reachable = Reachable::of(&open);
        assert!(reachable.from(&rules::default_position(shakmaty::variant::Variant::Chess)));
        assert!(reachable.from(&open));
        let d4 =
            rules::standard_from_fen("rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq - 0 1")
                .unwrap();
        assert!(
            !reachable.from(&d4),
            "the d-pawn the position has at home moved"
        );
        let traded = rules::standard_from_fen(
            "rnbqkb1r/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKB1R w KQkq - 0 4",
        )
        .unwrap();
        assert!(
            !reachable.from(&traded),
            "a piece the position has was captured"
        );
    }
}
