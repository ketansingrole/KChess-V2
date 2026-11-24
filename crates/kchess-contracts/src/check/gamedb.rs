//! The arguments of the game database methods.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::value::J;

use super::*;
use crate::types::{GameDbQuery, GameDbResult, GameDbSide, GameDbSort};

const DATABASE: &str = "Invalid database.";
const QUERY: &str = "Invalid game search.";
const POSITION: &str = "Invalid position.";

/// A database's id, as `gameDbStatus` lists it: 1 to 40 lowercase letters, digits and dashes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct DatabaseId(pub String);

/// Whether `id` is a database id as the core makes them.
pub fn is_database_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Validate for DatabaseId {
    fn validate(value: J) -> Result<Self, Refusal> {
        let id: String = decode(value, |_| Some(DATABASE))?;
        ensure(is_database_id(&id), Some(DATABASE), "", String::new)?;
        Ok(DatabaseId(id))
    }
}

/// A database's name as shown: 1 to 80 characters, trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct DatabaseName(pub String);

impl Validate for DatabaseName {
    fn validate(value: J) -> Result<Self, Refusal> {
        const NAME: &str = "A database name is 1 to 80 characters.";
        let name: String = decode(value, |_| Some(NAME))?;
        let name = name.trim();
        ensure(
            !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control),
            Some(NAME),
            "",
            String::new,
        )?;
        Ok(DatabaseName(name.to_string()))
    }
}

/// A game of a database, by the id a search listed.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct GameDbGameId(pub u64);

impl Validate for GameDbGameId {
    fn validate(value: J) -> Result<Self, Refusal> {
        const GAME: &str = "Invalid game.";
        let id: u64 = decode(value, |_| Some(GAME))?;
        within(id, 1, i64::MAX as u64, Some(GAME), "")?;
        Ok(GameDbGameId(id))
    }
}

/// A PGN file chosen to import: an absolute path ending in `.pgn` or `.pgn.zst`. The core opens
/// it itself (it may be gigabytes), and checks there that it is a readable file outside the
/// profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PgnFilePath(pub String);

impl PgnFilePath {
    /// Whether the file is zstd-compressed.
    pub fn compressed(&self) -> bool {
        self.0.to_ascii_lowercase().ends_with(".zst")
    }
}

impl Validate for PgnFilePath {
    fn validate(value: J) -> Result<Self, Refusal> {
        const PATH: &str = "Choose a .pgn or .pgn.zst file.";
        let path: String = decode(value, |_| Some(PATH))?;
        let lower = path.to_ascii_lowercase();
        ensure(
            std::path::Path::new(&path).is_absolute()
                && path.len() <= 4096
                && !path.contains('\0')
                && (lower.ends_with(".pgn") || lower.ends_with(".pgn.zst")),
            Some(PATH),
            "",
            String::new,
        )?;
        Ok(PgnFilePath(path))
    }
}

/// How many of the master games archive's newest months to keep to: 1 to 600 (fifty years).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct MasterMonths(pub u32);

impl Validate for MasterMonths {
    fn validate(value: J) -> Result<Self, Refusal> {
        const MONTHS: &str = "Invalid number of months.";
        let months: u64 = decode(value, |_| Some(MONTHS))?;
        within(months, 1, 600, Some(MONTHS), "")?;
        Ok(MasterMonths(u32::try_from(months).unwrap_or(600)))
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase", deny_unknown_fields)]
struct SentQuery {
    database: String,
    player: Option<String>,
    side: Option<GameDbSide>,
    opponent: Option<String>,
    event: Option<String>,
    year_from: Option<i64>,
    year_to: Option<i64>,
    min_elo: Option<u64>,
    result: Option<GameDbResult>,
    eco: Option<String>,
    position: Option<String>,
    sort: Option<GameDbSort>,
    offset: u64,
    limit: u64,
}

/// A search text: trimmed, at most 80 characters; blank is no filter.
fn search_text(text: Option<String>) -> Result<Option<String>, Refusal> {
    let Some(text) = text else {
        return Ok(None);
    };
    let text = text.trim();
    max_chars(text, 80, Some(QUERY), "")?;
    Ok((!text.is_empty()).then(|| text.to_string()))
}

impl Validate for GameDbQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentQuery = decode(value, |keys| match keys {
            ["database"] => Some(DATABASE),
            ["yearFrom" | "yearTo"] => Some("Invalid year."),
            ["minElo"] => Some("Invalid rating."),
            ["position"] => Some(POSITION),
            _ => Some(QUERY),
        })?;
        ensure(
            is_database_id(&sent.database),
            Some(DATABASE),
            "",
            String::new,
        )?;
        for year in sent.year_from.iter().chain(&sent.year_to) {
            within(*year, 1000, 9999, Some("Invalid year."), "")?;
        }
        if let Some(elo) = sent.min_elo {
            within(elo, 0, 4000, Some("Invalid rating."), "")?;
        }
        let eco = search_text(sent.eco)?.map(|eco| eco.to_ascii_uppercase());
        if let Some(eco) = &eco {
            let mut chars = eco.chars();
            let valid = chars.next().is_some_and(|c| ('A'..='E').contains(&c))
                && eco.len() <= 3
                && chars.all(|c| c.is_ascii_digit());
            ensure(valid, Some("Invalid ECO code."), "", String::new)?;
        }
        if let Some(position) = &sent.position {
            analysis_position(position, None).map_err(|_| Refusal::from(POSITION))?;
        }
        within(sent.offset, 0, 10_000_000, Some(QUERY), "")?;
        within(sent.limit, 1, 200, Some(QUERY), "")?;
        let year = |year: Option<i64>| year.and_then(|year| i32::try_from(year).ok());
        Ok(GameDbQuery {
            database: sent.database,
            player: search_text(sent.player)?,
            side: sent.side,
            opponent: search_text(sent.opponent)?,
            event: search_text(sent.event)?,
            year_from: year(sent.year_from),
            year_to: year(sent.year_to),
            min_elo: sent.min_elo.and_then(|elo| u32::try_from(elo).ok()),
            result: sent.result,
            eco,
            position: sent.position,
            sort: sent.sort,
            offset: u32::try_from(sent.offset).unwrap_or(u32::MAX),
            limit: u32::try_from(sent.limit).unwrap_or(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sent(value: serde_json::Value) -> J {
        J::from(&value)
    }

    #[test]
    fn database_ids_are_the_cores_own() {
        assert!(check::<DatabaseId>(sent(json!("twic-2024"))).is_ok());
        for bad in [json!(""), json!("../x"), json!("A"), json!(3)] {
            assert_eq!(check::<DatabaseId>(sent(bad)).unwrap_err(), DATABASE);
        }
    }

    #[test]
    fn a_pgn_path_is_absolute_and_names_a_pgn() {
        let absolute = if cfg!(windows) {
            "C:\\games\\twic.PGN"
        } else {
            "/games/twic.PGN"
        };
        assert!(check::<PgnFilePath>(sent(json!(absolute))).is_ok());
        let zst = if cfg!(windows) {
            "C:\\g\\a.pgn.zst"
        } else {
            "/g/a.pgn.zst"
        };
        assert!(check::<PgnFilePath>(sent(json!(zst))).unwrap().compressed());
        for bad in ["games/twic.pgn", "/games/twic.txt", ""] {
            assert!(check::<PgnFilePath>(sent(json!(bad))).is_err(), "{bad}");
        }
    }

    #[test]
    fn master_months_are_bounded() {
        assert_eq!(
            check::<MasterMonths>(sent(json!(12))).unwrap(),
            MasterMonths(12)
        );
        for bad in [json!(0), json!(601), json!("12"), json!(1.5)] {
            assert_eq!(
                check::<MasterMonths>(sent(bad.clone())).unwrap_err(),
                "Invalid number of months.",
                "{bad}"
            );
        }
    }

    #[test]
    fn a_query_is_bounded_and_trimmed() {
        let query = check::<GameDbQuery>(sent(json!({
            "database": "mine", "player": "  Carlsen ", "event": " ", "eco": "b9",
            "side": "white", "offset": 0, "limit": 20
        })))
        .unwrap();
        assert_eq!(query.player.as_deref(), Some("Carlsen"));
        assert_eq!(query.event, None);
        assert_eq!(query.eco.as_deref(), Some("B9"));
        assert_eq!(query.side, Some(GameDbSide::White));
        let refused = |value| check::<GameDbQuery>(sent(value)).unwrap_err();
        assert_eq!(
            refused(json!({"database": "mine", "offset": 0, "limit": 0})),
            QUERY
        );
        assert_eq!(
            refused(json!({"database": "mine", "yearFrom": 99, "offset": 0, "limit": 5})),
            "Invalid year."
        );
        assert_eq!(
            refused(json!({"database": "mine", "eco": "F1", "offset": 0, "limit": 5})),
            "Invalid ECO code."
        );
        assert_eq!(
            refused(json!({"database": "mine", "x": 1, "offset": 0, "limit": 5})),
            QUERY
        );
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let at = check::<GameDbQuery>(sent(
            json!({"database": "mine", "position": start, "offset": 0, "limit": 5}),
        ))
        .unwrap();
        assert_eq!(at.position.as_deref(), Some(start));
        for bad in [
            json!("8/8/8/8/8/8/8/8 w - - 0 1"),
            json!("nonsense"),
            json!(3),
        ] {
            assert_eq!(
                refused(json!({"database": "mine", "position": bad, "offset": 0, "limit": 5})),
                POSITION
            );
        }
    }
}
