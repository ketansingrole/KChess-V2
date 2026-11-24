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

/// Whether the file indexes its names' words (`player_words`, `event_words`); an older file that
/// could not be upgraded is searched name by name instead, alike but slower.
pub(super) fn worded(db: &Connection) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'player_words')",
        [],
        |row| row.get(0),
    )
    .sql()
}

/// The SQL that keeps the ids of `table` (`players` or `events`) whose name has, for each of the
/// words searched, a word starting with it, in any order ("magnus c" finds "Carlsen, Magnus");
/// and its arguments. `worded`: by the words' index, in a millisecond rather than a scan of every
/// name.
pub(super) fn names(table: &str, words: &str, worded: bool, args: &mut Vec<SqlValue>) -> String {
    let folded = gamedb::fold_name(words);
    let words: Vec<&str> = folded.split(' ').filter(|word| !word.is_empty()).collect();
    if words.is_empty() {
        // Nothing a name can hold (only punctuation): no name matches.
        return format!("SELECT id FROM {table} WHERE 0");
    }
    if worded {
        let index = if table == "players" {
            "player_words"
        } else {
            "event_words"
        };
        // The words that start with one: from it up to it followed by the last character.
        let starting = words.iter().map(|word| {
            args.push(SqlValue::Text((*word).to_string()));
            args.push(SqlValue::Text(format!("{word}{}", char::MAX)));
            let (a, b) = (args.len() - 1, args.len());
            format!("SELECT id FROM {index} WHERE word >= ?{a} AND word < ?{b}")
        });
        return starting.collect::<Vec<_>>().join(" INTERSECT ");
    }
    // Folded names hold only letters, digits and spaces: no `LIKE` wildcard can reach a pattern.
    let clauses: Vec<String> = words
        .iter()
        .map(|word| {
            args.push(SqlValue::Text(format!("{word}%")));
            args.push(SqlValue::Text(format!("% {word}%")));
            let (a, b) = (args.len() - 1, args.len());
            format!("(norm LIKE ?{a} OR norm LIKE ?{b})")
        })
        .collect();
    format!("SELECT id FROM {table} WHERE {}", clauses.join(" AND "))
}

/// The clauses picking out the games of `query`'s player (on its side), opponent and event, on
/// the games' columns as `{games}white` and so on (`games` is `g.` or nothing); their arguments
/// join `args`.
pub(super) fn named(
    query: &GameDbQuery,
    worded: bool,
    args: &mut Vec<SqlValue>,
    games: &str,
) -> Vec<String> {
    let mut clauses = Vec::new();
    let side = query.side.unwrap_or(GameDbSide::Either);
    if let Some(player) = &query.player {
        let ids = names("players", player, worded, args);
        clauses.push(match side {
            GameDbSide::White => format!("{games}white IN ({ids})"),
            GameDbSide::Black => format!("{games}black IN ({ids})"),
            GameDbSide::Either => {
                format!("({games}white IN ({ids}) OR {games}black IN ({ids}))")
            }
        });
        if let Some(opponent) = &query.opponent {
            let others = names("players", opponent, worded, args);
            clauses.push(match side {
                GameDbSide::White => format!("{games}black IN ({others})"),
                GameDbSide::Black => format!("{games}white IN ({others})"),
                GameDbSide::Either => format!(
                    "(({games}white IN ({ids}) AND {games}black IN ({others}))
                      OR ({games}black IN ({ids}) AND {games}white IN ({others})))"
                ),
            });
        }
    } else if let Some(opponent) = &query.opponent {
        let others = names("players", opponent, worded, args);
        clauses.push(format!(
            "({games}white IN ({others}) OR {games}black IN ({others}))"
        ));
    }
    if let Some(event) = &query.event {
        let ids = names("events", event, worded, args);
        clauses.push(format!("{games}event IN ({ids})"));
    }
    clauses
}

/// The clause keeping the games of `query`'s result.
pub(super) fn result(query: &GameDbQuery) -> Option<&'static str> {
    query.result.map(|result| match result {
        GameDbResult::White => "g.result = 1",
        GameDbResult::Draw => "g.result = 0",
        GameDbResult::Black => "g.result = -1",
    })
}

/// The `WHERE` clause of a search, and its arguments.
fn filters(query: &GameDbQuery, visible: i64, worded: bool) -> (String, Vec<SqlValue>) {
    let mut args = vec![SqlValue::Integer(visible)];
    let mut clauses = vec!["g.import_id <= ?1".to_string()];
    clauses.extend(named(query, worded, &mut args, "g."));
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
    clauses.extend(result(query).map(str::to_string));
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
    let (clauses, mut args) = filters(query, visible(db)?, worded(db)?);
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
