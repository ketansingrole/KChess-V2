//! Reading a database: a page of the games a search matches, and one game's PGN.

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension as _, Row, params_from_iter};

use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    GameDbPage, GameDbQuery, GameDbResult, GameDbRow, GameDbSide, GameDbSort,
};
use kchess_domain::gamedb::{self, Outcome, Stored};

/// The games a search may see: those of finished imports. An import adding to a database writes
/// its games as it goes; until it finishes they are not the database's yet.
pub(super) fn visible(db: &Connection) -> Result<i64> {
    db.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM imports WHERE finished_at IS NOT NULL",
        [],
        |row| row.get(0),
    )
    .sql()
}

/// The SQL that keeps the ids of `table` whose name has, for each of the words searched, a word
/// starting with it, in any order ("magnus c" finds "Carlsen, Magnus"); and its arguments.
fn names(table: &str, words: &str, args: &mut Vec<SqlValue>) -> String {
    // Folded names hold only letters, digits and spaces: no `LIKE` wildcard can reach a pattern.
    let folded = gamedb::fold_name(words);
    let mut clauses = Vec::new();
    for word in folded.split(' ').filter(|word| !word.is_empty()) {
        args.push(SqlValue::Text(format!("{word}%")));
        args.push(SqlValue::Text(format!("% {word}%")));
        let (a, b) = (args.len() - 1, args.len());
        clauses.push(format!("(norm LIKE ?{a} OR norm LIKE ?{b})"));
    }
    if clauses.is_empty() {
        // Nothing a name can hold (only punctuation): no name matches.
        clauses.push("0".to_string());
    }
    format!("SELECT id FROM {table} WHERE {}", clauses.join(" AND "))
}

/// The `WHERE` clause of a search, and its arguments.
pub(super) fn filters(query: &GameDbQuery, visible: i64) -> (String, Vec<SqlValue>) {
    let mut args = vec![SqlValue::Integer(visible)];
    let mut clauses = vec!["g.import_id <= ?1".to_string()];
    if let Some(player) = &query.player {
        let ids = names("players", player, &mut args);
        clauses.push(match query.side.unwrap_or(GameDbSide::Either) {
            GameDbSide::White => format!("g.white IN ({ids})"),
            GameDbSide::Black => format!("g.black IN ({ids})"),
            GameDbSide::Either => format!("(g.white IN ({ids}) OR g.black IN ({ids}))"),
        });
        if let Some(opponent) = &query.opponent {
            let others = names("players", opponent, &mut args);
            clauses.push(match query.side.unwrap_or(GameDbSide::Either) {
                GameDbSide::White => format!("g.black IN ({others})"),
                GameDbSide::Black => format!("g.white IN ({others})"),
                GameDbSide::Either => format!(
                    "((g.white IN ({ids}) AND g.black IN ({others}))
                      OR (g.black IN ({ids}) AND g.white IN ({others})))"
                ),
            });
        }
    } else if let Some(opponent) = &query.opponent {
        let others = names("players", opponent, &mut args);
        clauses.push(format!("(g.white IN ({others}) OR g.black IN ({others}))"));
    }
    if let Some(event) = &query.event {
        let ids = names("events", event, &mut args);
        clauses.push(format!("g.event IN ({ids})"));
    }
    if let Some(year) = query.year_from {
        args.push(SqlValue::Integer(i64::from(year)));
        clauses.push(format!("g.year >= ?{}", args.len()));
    }
    if let Some(year) = query.year_to {
        args.push(SqlValue::Integer(i64::from(year)));
        clauses.push(format!("g.year <= ?{}", args.len()));
    }
    if let Some(elo) = query.min_elo {
        args.push(SqlValue::Integer(i64::from(elo)));
        let n = args.len();
        clauses.push(format!("g.low_elo >= ?{n}"));
    }
    if let Some(result) = query.result {
        clauses.push(
            match result {
                GameDbResult::White => "g.result = 1",
                GameDbResult::Draw => "g.result = 0",
                GameDbResult::Black => "g.result = -1",
            }
            .to_string(),
        );
    }
    if let Some(eco) = &query.eco {
        args.push(SqlValue::Text(format!("{eco}%")));
        clauses.push(format!("g.eco LIKE ?{}", args.len()));
    }
    (clauses.join(" AND "), args)
}

pub(super) fn row(row: &Row<'_>) -> rusqlite::Result<GameDbRow> {
    let elo = |value: Option<i64>| value.and_then(|elo| u32::try_from(elo).ok());
    Ok(GameDbRow {
        id: row.get::<_, i64>(0)? as u64,
        white: row.get(1)?,
        black: row.get(2)?,
        white_elo: elo(row.get(3)?),
        black_elo: elo(row.get(4)?),
        result: Outcome::from_score(row.get(5)?).header().to_string(),
        event: row.get(6)?,
        site: row.get(7)?,
        date: row.get(8)?,
        round: row.get(9)?,
        eco: row.get(10)?,
        opening: row.get(11)?,
        plies: row.get(12)?,
    })
}

pub(super) const COLUMNS: &str =
    "g.id, w.name, b.name, g.white_elo, g.black_elo, g.result, e.name, g.site,
    g.date, g.round, g.eco, g.opening, g.plies";

pub(super) const JOINS: &str = "JOIN players w ON w.id = g.white JOIN players b ON b.id = g.black
    LEFT JOIN events e ON e.id = g.event";

/// One page of the games `query` matches, and how many it matches.
pub fn games(db: &Connection, query: &GameDbQuery) -> Result<GameDbPage> {
    let (clauses, mut args) = filters(query, visible(db)?);
    let total: i64 = db
        .query_row(
            &format!("SELECT COUNT(*) FROM games g WHERE {clauses}"),
            params_from_iter(args.iter()),
            |row| row.get(0),
        )
        .sql()?;
    let total = u64::try_from(total).unwrap_or(0);
    let order = match query.sort.unwrap_or(GameDbSort::Newest) {
        GameDbSort::Newest => "g.date_key DESC, g.id DESC",
        GameDbSort::Oldest => "g.date_key ASC, g.id ASC",
        GameDbSort::Rating => "g.rating DESC, g.id DESC",
    };
    args.push(SqlValue::Integer(i64::from(query.limit)));
    args.push(SqlValue::Integer(i64::from(query.offset)));
    let (limit, offset) = (args.len() - 1, args.len());
    let mut select = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM games g {JOINS} WHERE {clauses}
             ORDER BY {order} LIMIT ?{limit} OFFSET ?{offset}"
        ))
        .sql()?;
    let games = select
        .query_map(params_from_iter(args.iter()), row)
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    Ok(GameDbPage {
        games,
        total,
        offset: query.offset,
    })
}

/// The game `id` as stored.
pub fn stored(db: &Connection, id: u64) -> Result<Option<Stored>> {
    let visible = visible(db)?;
    db.query_row(
        &format!(
            "SELECT w.name, b.name, e.name, g.site, g.date, g.year, g.round, g.white_elo,
                    g.black_elo, g.result, g.eco, g.opening, g.fen, g.plies, g.moves, g.extra
             FROM games g {JOINS} WHERE g.id = ?1 AND g.import_id <= ?2"
        ),
        rusqlite::params![id as i64, visible],
        |row| {
            let elo = |value: Option<i64>| value.and_then(|elo| u16::try_from(elo).ok());
            let date: Option<String> = row.get(4)?;
            Ok(Stored {
                white: row.get(0)?,
                black: row.get(1)?,
                event: row.get(2)?,
                site: row.get(3)?,
                month: date.as_deref().and_then(|d| gamedb::date_parts(d).1),
                date,
                year: row.get(5)?,
                round: row.get(6)?,
                white_elo: elo(row.get(7)?),
                black_elo: elo(row.get(8)?),
                outcome: Outcome::from_score(row.get(9)?),
                eco: row.get(10)?,
                opening: row.get(11)?,
                fen: row.get(12)?,
                plies: usize::try_from(row.get::<_, i64>(13)?).unwrap_or(0),
                moves: row.get(14)?,
                extra: row.get(15)?,
            })
        },
    )
    .optional()
    .sql()
}

/// The game `id`'s PGN, written again from what the database keeps of it.
pub fn pgn(db: &Connection, id: u64) -> Result<String> {
    let Some(stored) = stored(db, id)? else {
        return Err(CoreError::new("That game is not in this database."));
    };
    gamedb::pgn_of(&stored).ok_or_else(|| CoreError::new("This game could not be read."))
}
