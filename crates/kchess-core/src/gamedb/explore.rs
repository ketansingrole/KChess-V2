//! A position looked up in a game database, as the explorer shows Lichess's: the moves played
//! from it with their results, and its best-rated games.
//!
//! With no filter the opening tree answers, one indexed read. With filters (a player, the side
//! they had, years, a rating) the matching games are found by their header indexes and replayed
//! to the position: the tree counts every game, so it cannot tell a player's. A game stops being
//! replayed once it can no longer reach the position (it has fewer pieces, or has moved a pawn
//! the position still has at home), so most stop within a few moves.

use std::cmp::Reverse;
use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension as _, params_from_iter};
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Color, Move, Position as _, Rank, Role};

use super::search;
use super::tree::{self, Stats, TreeMove};
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    ExplorerGame, GameDbQuery, GameDbSide, LookupMove, LookupOptions, PositionLookupKind,
    SavedLookup,
};
use kchess_domain::gamedb::{self, Outcome};
use kchess_domain::rules;

/// The most games a filtered lookup replays; past it, the filters must narrow the search.
pub const REPLAY_MAX: i64 = 100_000;
/// The best-rated games a lookup lists.
const TOP_GAMES: usize = 8;

/// A count as people read it: 119,350.
fn grouped(count: i64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) && digit.is_ascii_digit() {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

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
/// of pieces, and each side's pawns still on their starting squares.
struct Reachable {
    pieces: usize,
    white_home: Bitboard,
    black_home: Bitboard,
}

impl Reachable {
    fn of(pos: &VariantPosition) -> Reachable {
        let board = pos.board();
        let pawns = board.by_role(Role::Pawn);
        Reachable {
            pieces: board.occupied().count(),
            white_home: pawns & board.by_color(Color::White) & Bitboard::from_rank(Rank::Second),
            black_home: pawns & board.by_color(Color::Black) & Bitboard::from_rank(Rank::Seventh),
        }
    }

    /// Whether the position can still follow `pos`: captures and pawn moves are for good.
    fn from(&self, pos: &VariantPosition) -> bool {
        let board = pos.board();
        let pawns = board.by_role(Role::Pawn);
        board.occupied().count() >= self.pieces
            && pawns & board.by_color(Color::White) & self.white_home == self.white_home
            && pawns & board.by_color(Color::Black) & self.black_home == self.black_home
    }
}

/// The moves played from the position `key` (`target`) in the games `query` matches, counted
/// as the tree counts them: each time a game reached it, the game's result once.
fn replayed(
    db: &Connection,
    query: &GameDbQuery,
    target: &VariantPosition,
) -> Result<Vec<TreeMove>> {
    let (clauses, args) = search::filters(query, search::visible(db)?);
    // An unfinished game counts in no result: the tree leaves it out, and so does this.
    let clauses = format!("{clauses} AND g.result IS NOT NULL");
    let matching: i64 = db
        .query_row(
            &format!("SELECT COUNT(*) FROM games g WHERE {clauses}"),
            params_from_iter(args.iter()),
            |row| row.get(0),
        )
        .sql()?;
    if matching > REPLAY_MAX {
        return Err(CoreError::new(format!(
            "{} games match these filters. Narrow them to {} or fewer.",
            grouped(matching),
            grouped(REPLAY_MAX)
        )));
    }
    let key = gamedb::position_key(target);
    let reachable = Reachable::of(target);
    let mut select = db
        .prepare(&format!(
            "SELECT g.id, g.fen, g.moves, g.result, g.rating, g.date_key FROM games g
             WHERE {clauses}"
        ))
        .sql()?;
    let mut rows = select.query(params_from_iter(args.iter())).sql()?;
    let mut moves: HashMap<u16, Stats> = HashMap::new();
    while let Some(row) = rows.next().sql()? {
        let fen: Option<String> = row.get(1).sql()?;
        let Some(start) = gamedb::start_of(fen.as_deref()) else {
            continue;
        };
        let code: Vec<u8> = row.get(2).sql()?;
        let mut played = Vec::new();
        gamedb::walk_line(&start, &code, |pos, m| {
            if !reachable.from(pos) {
                return false;
            }
            if gamedb::position_key(pos) == key {
                played.push(gamedb::pack_move(m));
            }
            true
        });
        if played.is_empty() {
            continue;
        }
        let rating: Option<i64> = row.get(4).sql()?;
        let stats = Stats::of(
            row.get(0).sql()?,
            Outcome::from_score(row.get(3).sql()?),
            rating.and_then(|rating| u16::try_from(rating).ok()),
            u32::try_from(row.get::<_, i64>(5).sql()?).unwrap_or(0),
        );
        for mv in played {
            moves
                .entry(mv)
                .and_modify(|sum| sum.add(&stats))
                .or_insert(stats);
        }
    }
    let mut moves: Vec<TreeMove> = moves
        .into_iter()
        .map(|(mv, stats)| TreeMove { mv, stats })
        .collect();
    moves.sort_by_key(|m| (Reverse(m.stats.games()), m.mv));
    Ok(moves)
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
/// `now` is the lookup's time.
pub fn lookup(
    db: &Connection,
    database: &str,
    fen: &str,
    options: &LookupOptions,
    now: i64,
) -> Result<SavedLookup> {
    let Some(pos) = rules::standard_from_fen(fen) else {
        return Err(CoreError::new("That position is not legal."));
    };
    let found = match query(database, options) {
        Some(query) => replayed(db, &query, &pos)?,
        None => tree::moves(db, gamedb::position_key(&pos))?,
    };
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
    fn counts_are_grouped_in_thousands() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(119_350), "119,350");
        assert_eq!(grouped(1_294_471), "1,294,471");
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
