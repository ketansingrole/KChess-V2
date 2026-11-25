//! One edition of an event in a database (`gameDbEvent`): the games under the event's name played
//! around one game's date, with each player's standing and results (`standings` has the rules).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension as _, params};

use super::search;
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{GameDbEventPlayer, GameDbEventReport, GameDbEventResult};
use kchess_domain::gamedb::Outcome;
use kchess_domain::gamedb::standings::{self, Played};

/// The most games of one event name read to find an edition: a name this common ("Rated Blitz
/// game", "Casual game") names no tournament.
pub const MOST_SCANNED: i64 = 100_000;
/// The most games of an edition tabled.
pub const MOST_GAMES: usize = 5_000;

const TOO_MANY: &str = "This event has too many games for one table.";

struct Game {
    id: i64,
    white: i64,
    black: i64,
    date: Option<String>,
    date_key: u32,
    round: Option<String>,
    white_elo: Option<u16>,
    black_elo: Option<u16>,
    outcome: Outcome,
}

/// A round's order: "2.10" after "2.9", and a round that is no number last.
fn round_key(round: Option<&str>) -> Vec<u32> {
    let parts: Option<Vec<u32>> = round
        .filter(|round| !round.is_empty() && *round != "?" && *round != "-")
        .and_then(|round| round.split('.').map(|part| part.parse().ok()).collect());
    parts.unwrap_or_else(|| vec![u32::MAX])
}

/// The edition of game `game`'s event in `db`.
pub fn report(db: &Connection, game: u64) -> Result<GameDbEventReport> {
    let visible = search::visible(db)?;
    let chosen: Option<(Option<i64>, Option<String>)> = db
        .query_row(
            "SELECT event, site FROM games WHERE id = ?1 AND import_id <= ?2",
            params![game as i64, visible],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .sql()?;
    let Some((event, site)) = chosen else {
        return Err(CoreError::new("That game is not in this database."));
    };
    let Some(event) = event else {
        return Err(CoreError::new("That game names no event."));
    };
    let name: String = db
        .query_row("SELECT name FROM events WHERE id = ?1", [event], |row| {
            row.get(0)
        })
        .sql()?;
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM games WHERE event = ?1 AND import_id <= ?2",
            params![event, visible],
            |row| row.get(0),
        )
        .sql()?;
    if count > MOST_SCANNED {
        return Err(CoreError::new(TOO_MANY));
    }
    let mut statement = db
        .prepare(
            "SELECT id, white, black, date, date_key, round, white_elo, black_elo, result
               FROM games WHERE event = ?1 AND import_id <= ?2",
        )
        .sql()?;
    let games: Vec<Game> = statement
        .query_map(params![event, visible], |row| {
            Ok(Game {
                id: row.get(0)?,
                white: row.get(1)?,
                black: row.get(2)?,
                date: row.get(3)?,
                date_key: row.get(4)?,
                round: row.get(5)?,
                white_elo: row.get(6)?,
                black_elo: row.get(7)?,
                outcome: Outcome::from_score(row.get(8)?),
            })
        })
        .sql()?
        .collect::<rusqlite::Result<_>>()
        .sql()?;
    let days: Vec<Option<u32>> = games
        .iter()
        .map(|game| standings::day(game.date_key))
        .collect();
    let at = games
        .iter()
        .position(|candidate| candidate.id == game as i64)
        .ok_or_else(|| CoreError::new("That game is not in this database."))?;
    let mut edition: Vec<&Game> = standings::edition(&days, at)
        .into_iter()
        .map(|i| &games[i])
        .collect();
    if edition.len() > MOST_GAMES {
        return Err(CoreError::new(TOO_MANY));
    }
    edition.sort_by_cached_key(|game| (game.date_key, round_key(game.round.as_deref()), game.id));

    // The players by first appearance, then numbered as the standings place them.
    let mut number: HashMap<i64, usize> = HashMap::new();
    let mut ids: Vec<i64> = Vec::new();
    let mut ratings: Vec<Option<u16>> = Vec::new();
    for game in &edition {
        for (id, elo) in [(game.white, game.white_elo), (game.black, game.black_elo)] {
            let at = *number.entry(id).or_insert_with(|| {
                ids.push(id);
                ratings.push(None);
                ids.len() - 1
            });
            if ratings[at].is_none() {
                ratings[at] = elo;
            }
        }
    }
    let played: Vec<Played> = edition
        .iter()
        .map(|game| Played {
            white: number[&game.white],
            black: number[&game.black],
            outcome: game.outcome,
            white_elo: game.white_elo,
            black_elo: game.black_elo,
        })
        .collect();
    let table = standings::standings(ids.len(), &played);
    let mut place = vec![0u32; ids.len()];
    for (at, standing) in table.iter().enumerate() {
        place[standing.player] = u32::try_from(at).unwrap_or(u32::MAX);
    }
    let mut names = db.prepare("SELECT name FROM players WHERE id = ?1").sql()?;
    let mut players = Vec::with_capacity(table.len());
    for standing in &table {
        let me = standing.player;
        let results = edition
            .iter()
            .zip(&played)
            .filter_map(|(game, played)| {
                let (white, black) = standings::half_points(played.outcome)?;
                let (white_side, opponent, points) = if played.white == me {
                    (true, played.black, white)
                } else if played.black == me {
                    (false, played.white, black)
                } else {
                    return None;
                };
                Some(GameDbEventResult {
                    game: game.id as u64,
                    opponent: place[opponent],
                    white: white_side,
                    half_points: u8::try_from(points).unwrap_or(0),
                    round: game.round.clone(),
                })
            })
            .collect();
        players.push(GameDbEventPlayer {
            name: names.query_row([ids[me]], |row| row.get(0)).sql()?,
            rating: ratings[me].map(u32::from),
            half_points: standing.half_points,
            wins: standing.wins,
            draws: standing.draws,
            losses: standing.losses,
            tiebreak_quarters: standing.tiebreak_quarters,
            performance: standing.performance,
            results,
        });
    }
    let dated = edition.iter().filter(|game| game.date_key / 10_000 > 0);
    let date = |game: Option<&&Game>| game.and_then(|game| game.date.clone());
    let (first, last) = {
        let dated: Vec<&&Game> = dated.collect();
        (date(dated.first().copied()), date(dated.last().copied()))
    };
    Ok(GameDbEventReport {
        name,
        site: site.filter(|site| !site.is_empty() && site != "?"),
        first_date: first,
        last_date: last,
        games: edition.len() as u64,
        players,
    })
}

#[cfg(test)]
mod tests {
    use super::round_key;

    #[test]
    fn rounds_order_as_numbers_and_unnumbered_rounds_last() {
        let mut rounds = vec![
            Some("2.10"),
            Some("?"),
            Some("2.9"),
            Some("10"),
            None,
            Some("3"),
        ];
        rounds.sort_by_key(|round| round_key(*round));
        assert_eq!(
            rounds,
            vec![
                Some("2.9"),
                Some("2.10"),
                Some("3"),
                Some("10"),
                Some("?"),
                None
            ]
        );
    }
}
