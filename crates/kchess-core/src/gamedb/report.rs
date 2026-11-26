//! A player's games in one database at a glance (`gameDbPlayer`): their results and openings as
//! White and as Black, their rating month by month, and their wins over the strongest opponents.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension as _};

use super::search;
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{GameDbOpening, GameDbPlayerReport, GameDbRating, GameDbRecord};
use kchess_domain::gamedb::{self, Outcome};
use kchess_domain::rules;

/// The openings listed for each colour, the most played first.
pub const OPENINGS: usize = 8;
/// The best wins listed.
pub const BEST: usize = 5;

/// An opening's name as a player's report groups it: the family of its name ("Sicilian
/// Defense" of "Sicilian Defense: Najdorf Variation"), else its ECO code, else its first two
/// moves ("1. e4 c5"); "Other" when the game has none of them.
pub fn opening_name(opening: Option<&str>, eco: Option<&str>, first: &[String]) -> String {
    let family = opening
        .and_then(|name| name.split([':', ',']).next())
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "?");
    if let Some(family) = family {
        return family.to_string();
    }
    if let Some(eco) = eco.map(str::trim).filter(|eco| !eco.is_empty()) {
        return eco.to_string();
    }
    match first {
        [] => "Other".to_string(),
        [white] => format!("1. {white}"),
        [white, black, ..] => format!("1. {white} {black}"),
    }
}

/// A game's first two moves in SAN, when it starts from the standard position.
pub(super) fn first_moves(fen: Option<&str>, moves: &[u8]) -> Vec<String> {
    if fen.is_some() {
        return Vec::new();
    }
    let Some(start) = gamedb::start_of(None) else {
        return Vec::new();
    };
    let Some(line) = gamedb::decode_line(&start, &moves[..moves.len().min(2)]) else {
        return Vec::new();
    };
    let mut pos = start;
    line.into_iter()
        .map(|m| rules::san_and_play(&mut pos, m))
        .collect()
}

fn count(record: &mut GameDbRecord, outcome: Outcome, white: bool) {
    record.games += 1;
    match (outcome, white) {
        (Outcome::Draw, _) => record.draws += 1,
        (Outcome::White, true) | (Outcome::Black, false) => record.wins += 1,
        (Outcome::White, false) | (Outcome::Black, true) => record.losses += 1,
        (Outcome::Unknown, _) => {}
    }
}

/// The report of the player named `name` in `db`.
pub fn player(db: &Connection, name: &str) -> Result<GameDbPlayerReport> {
    let visible = search::visible(db)?;
    let id: Option<i64> = db
        .query_row("SELECT id FROM players WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .optional()
        .sql()?;
    let Some(id) = id else {
        return Err(CoreError::new("That player has no games in this database."));
    };
    let mut games = db
        .prepare(
            "SELECT id, white = ?1, result, opening, eco, year, date_key, white_elo, black_elo,
                    fen, substr(moves, 1, 2)
             FROM games WHERE (white = ?1 OR black = ?1) AND import_id <= ?2
             ORDER BY date_key, id",
        )
        .sql()?;
    let mut rows = games.query([id, visible]).sql()?;
    let mut report = GameDbPlayerReport {
        name: name.to_string(),
        ..GameDbPlayerReport::default()
    };
    let mut openings: [HashMap<String, GameDbRecord>; 2] = [HashMap::new(), HashMap::new()];
    // Each month's rating, the last game's of the month standing.
    let mut ratings: Vec<GameDbRating> = Vec::new();
    // Wins: the opponent's rating, then the date, for the best first.
    let mut wins: Vec<(u32, i64, i64)> = Vec::new();
    while let Some(row) = rows.next().sql()? {
        let game: i64 = row.get(0).sql()?;
        let white: bool = row.get(1).sql()?;
        let outcome = Outcome::from_score(row.get(2).sql()?);
        let opening: Option<String> = row.get(3).sql()?;
        let eco: Option<String> = row.get(4).sql()?;
        let year: Option<i32> = row.get(5).sql()?;
        let date: i64 = row.get(6).sql()?;
        let elo = |value: Option<i64>| value.and_then(|elo| u32::try_from(elo).ok());
        let (own, other) = {
            let (w, b) = (elo(row.get(7).sql()?), elo(row.get(8).sql()?));
            if white { (w, b) } else { (b, w) }
        };
        let fen: Option<String> = row.get(9).sql()?;
        let moves: Vec<u8> = row.get(10).sql()?;

        let side = if white {
            &mut report.white
        } else {
            &mut report.black
        };
        count(&mut side.record, outcome, white);
        let name = opening_name(
            opening.as_deref(),
            eco.as_deref(),
            &first_moves(fen.as_deref(), &moves),
        );
        count(
            openings[usize::from(!white)].entry(name).or_default(),
            outcome,
            white,
        );
        if let Some(year) = year.filter(|year| *year > 0) {
            report.first_year = Some(report.first_year.map_or(year, |first| first.min(year)));
            report.last_year = Some(report.last_year.map_or(year, |last| last.max(year)));
            let month = ((date / 100) % 100) as u8;
            if let (Some(rating), 1..=12) = (own, month) {
                match ratings.last_mut() {
                    Some(last) if (last.year, last.month) == (year, month) => last.rating = rating,
                    _ => ratings.push(GameDbRating {
                        year,
                        month,
                        rating,
                    }),
                }
            }
        }
        let won = matches!(
            (outcome, white),
            (Outcome::White, true) | (Outcome::Black, false)
        );
        if won {
            wins.push((other.unwrap_or(0), date, game));
        }
    }
    for (side, played) in [&mut report.white, &mut report.black]
        .into_iter()
        .zip(openings)
    {
        let mut played: Vec<GameDbOpening> = played
            .into_iter()
            .map(|(name, record)| GameDbOpening { name, record })
            .collect();
        played.sort_by(|a, b| {
            b.record
                .games
                .cmp(&a.record.games)
                .then_with(|| a.name.cmp(&b.name))
        });
        played.truncate(OPENINGS);
        side.openings = played;
    }
    report.ratings = ratings;
    wins.sort_by(|a, b| b.cmp(a));
    let mut best = db
        .prepare_cached(&format!(
            "SELECT {} FROM games g {} WHERE g.id = ?1",
            search::COLUMNS,
            search::JOINS
        ))
        .sql()?;
    for (_, _, game) in wins.into_iter().take(BEST) {
        report.best.push(best.query_row([game], search::row).sql()?);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::opening_name;

    #[test]
    fn an_opening_is_named_by_its_family_its_code_or_its_first_moves() {
        let moves = |line: &[&str]| -> Vec<String> { line.iter().map(|m| m.to_string()).collect() };
        assert_eq!(
            opening_name(
                Some("Sicilian Defense: Najdorf Variation"),
                Some("B90"),
                &[]
            ),
            "Sicilian Defense"
        );
        assert_eq!(
            opening_name(Some("Queen's Gambit Declined, Exchange"), None, &[]),
            "Queen's Gambit Declined"
        );
        assert_eq!(opening_name(Some("?"), Some("C65"), &[]), "C65");
        assert_eq!(opening_name(None, None, &moves(&["e4", "c5"])), "1. e4 c5");
        assert_eq!(opening_name(None, Some(" "), &moves(&["d4"])), "1. d4");
        assert_eq!(opening_name(None, None, &[]), "Other");
    }
}
