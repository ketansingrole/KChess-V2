//! The arguments of the game database methods.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::value::J;

use super::*;
use crate::types::{
    ExplorerSpeed, GameDbQuery, GameDbResult, GameDbSide, GameDbSort, GameDbStructure, GameSite,
    LichessGames, PlayerGames,
};

const DATABASE: &str = "Invalid database.";
const QUERY: &str = "Invalid game search.";
const POSITION: &str = "Invalid position.";
const MATERIAL: &str = "Write the material as each side's pieces, like R vs B.";
const PIECES: &str = "Write the pieces as each side's, like Nd5 vs e6.";

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

/// A player's name as a game's header gives it: 1 to 200 characters, trimmed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PlayerName(pub String);

impl Validate for PlayerName {
    fn validate(value: J) -> Result<Self, Refusal> {
        const NAME: &str = "A player's name is 1 to 200 characters.";
        let name: String = decode(value, |_| Some(NAME))?;
        let name = name.trim();
        ensure(
            !name.is_empty() && name.chars().count() <= 200 && !name.chars().any(char::is_control),
            Some(NAME),
            "",
            String::new,
        )?;
        Ok(PlayerName(name.to_string()))
    }
}

/// Names of players to merge into another: 1 to 100 of them, each kept once.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PlayerNames(pub Vec<String>);

impl Validate for PlayerNames {
    fn validate(value: J) -> Result<Self, Refusal> {
        const NAMES: &str = "Choose 1 to 100 names, each 1 to 200 characters.";
        let values: Vec<String> = decode(value, |_| Some(NAMES))?;
        let mut names: Vec<String> = Vec::with_capacity(values.len());
        for name in values {
            let name = name.trim().to_string();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        ensure(
            (1..=100).contains(&names.len())
                && names.iter().all(|name| {
                    !name.is_empty()
                        && name.chars().count() <= 200
                        && !name.chars().any(char::is_control)
                }),
            Some(NAMES),
            "",
            String::new,
        )?;
        Ok(PlayerNames(names))
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

/// Games of a database by the ids a search listed: 1 to 10,000 of them, each kept once.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct GameDbGameIds(pub Vec<u64>);

impl Validate for GameDbGameIds {
    fn validate(value: J) -> Result<Self, Refusal> {
        const GAMES: &str = "Choose 1 to 10,000 games.";
        let mut ids: Vec<u64> = decode(value, |_| Some(GAMES))?;
        ensure(
            (1..=10_000).contains(&ids.len())
                && ids.iter().all(|id| (1..=i64::MAX as u64).contains(id)),
            Some(GAMES),
            "",
            String::new,
        )?;
        ids.sort_unstable();
        ids.dedup();
        Ok(GameDbGameIds(ids))
    }
}

/// Where to write a PGN file the user chose: an absolute path ending in `.pgn`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PgnSavePath(pub String);

impl Validate for PgnSavePath {
    fn validate(value: J) -> Result<Self, Refusal> {
        const PATH: &str = "Save the games as a .pgn file.";
        let path: String = decode(value, |_| Some(PATH))?;
        ensure(
            std::path::Path::new(&path).is_absolute()
                && path.len() <= 4096
                && !path.contains('\0')
                && path.to_ascii_lowercase().ends_with(".pgn"),
            Some(PATH),
            "",
            String::new,
        )?;
        Ok(PgnSavePath(path))
    }
}

/// A PGN file chosen to import: an absolute path ending in `.pgn`, `.pgn.zst`, `.zip` or `.7z`
/// (an archive: the PGN files it holds). The core opens it itself (it may be gigabytes), and checks there that it is a
/// readable file outside the profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PgnFilePath(pub String);

/// How a chosen file holds its PGN text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgnPacking {
    /// The text itself.
    Plain,
    /// Zstandard-compressed (`.pgn.zst`).
    Zstd,
    /// A zip archive: every `.pgn` file in it (`.zip`).
    Zip,
    /// A 7-Zip archive: every `.pgn` file in it (`.7z`).
    SevenZip,
    /// bzip2-compressed (`.pgn.bz2`, as FICS packs its games), of one or more streams.
    Bzip2,
}

impl PgnFilePath {
    /// How the file holds its games, by its extension.
    pub fn packing(&self) -> PgnPacking {
        let lower = self.0.to_ascii_lowercase();
        if lower.ends_with(".zst") {
            PgnPacking::Zstd
        } else if lower.ends_with(".zip") {
            PgnPacking::Zip
        } else if lower.ends_with(".7z") {
            PgnPacking::SevenZip
        } else if lower.ends_with(".bz2") {
            PgnPacking::Bzip2
        } else {
            PgnPacking::Plain
        }
    }
}

impl Validate for PgnFilePath {
    fn validate(value: J) -> Result<Self, Refusal> {
        const PATH: &str = "Choose a .pgn, .pgn.zst, .pgn.bz2, .zip or .7z file.";
        let path: String = decode(value, |_| Some(PATH))?;
        let lower = path.to_ascii_lowercase();
        ensure(
            std::path::Path::new(&path).is_absolute()
                && path.len() <= 4096
                && !path.contains('\0')
                && [".pgn", ".pgn.zst", ".pgn.bz2", ".zip", ".7z"]
                    .iter()
                    .any(|extension| lower.ends_with(extension)),
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
struct SentPlayer {
    site: GameSite,
    username: String,
    months: Option<u64>,
    speeds: Option<Vec<ExplorerSpeed>>,
    rated: Option<bool>,
}

/// A player's games to download: a username as its site allows them, 1 to 600 months, and at
/// least one speed when speeds are given (each kept once).
impl Validate for PlayerGames {
    fn validate(value: J) -> Result<Self, Refusal> {
        const PLAYER: &str = "Invalid player download.";
        const MONTHS: &str = "Invalid number of months.";
        let sent: SentPlayer = decode(value, |keys| match keys {
            ["months"] => Some(MONTHS),
            _ => Some(PLAYER),
        })?;
        let username = match sent.site {
            GameSite::Lichess => username(&sent.username)?,
            GameSite::Chesscom => ChesscomUsername::validate(J::Str(sent.username))?.0,
        };
        if let Some(months) = sent.months {
            within(months, 1, 600, Some(MONTHS), "")?;
        }
        let speeds = match sent.speeds {
            Some(mut speeds) => {
                ensure(!speeds.is_empty(), Some(PLAYER), "", String::new)?;
                speeds.sort();
                speeds.dedup();
                Some(speeds)
            }
            None => None,
        };
        Ok(PlayerGames {
            site: sent.site,
            username,
            months: sent
                .months
                .map(|months| u32::try_from(months).unwrap_or(600)),
            speeds,
            rated: sent.rated.unwrap_or(false),
        })
    }
}

/// A month of Lichess's archive, `YYYY-MM`, from its first (2013-01).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct LichessMonth(pub String);

const MONTH: &str = "Invalid month.";

fn lichess_month(month: &str) -> Result<String, Refusal> {
    let month = month.trim();
    let (year, number) = month.split_once('-').unwrap_or_default();
    let digits =
        |text: &str, len: usize| text.len() == len && text.bytes().all(|b| b.is_ascii_digit());
    let year_ok = digits(year, 4)
        && year
            .parse::<u32>()
            .is_ok_and(|y| (2013..=2200).contains(&y));
    let number_ok = digits(number, 2) && number.parse::<u32>().is_ok_and(|m| (1..=12).contains(&m));
    ensure(year_ok && number_ok, Some(MONTH), "", String::new)?;
    Ok(month.to_string())
}

impl Validate for LichessMonth {
    fn validate(value: J) -> Result<Self, Refusal> {
        let month: String = decode(value, |_| Some(MONTH))?;
        Ok(LichessMonth(lichess_month(&month)?))
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase", deny_unknown_fields)]
struct SentLichess {
    month: Option<String>,
    min_rating: u64,
}

/// Strong Lichess games to download: a month of the archive, and a rating floor of 1000 to 3000
/// in hundreds.
impl Validate for LichessGames {
    fn validate(value: J) -> Result<Self, Refusal> {
        const RATING: &str = "Invalid rating.";
        let sent: SentLichess = decode(value, |keys| match keys {
            ["month"] => Some(MONTH),
            ["minRating"] => Some(RATING),
            _ => Some("Invalid Lichess download."),
        })?;
        within(sent.min_rating, 1000, 3000, Some(RATING), "")?;
        ensure(
            sent.min_rating.is_multiple_of(100),
            Some(RATING),
            "",
            String::new,
        )?;
        let month = sent.month.as_deref().map(lichess_month).transpose()?;
        Ok(LichessGames {
            month,
            min_rating: u32::try_from(sent.min_rating).unwrap_or(3000),
        })
    }
}

/// A Lichess download a frontend's form made, checked as the core will check it.
pub fn lichess_games(request: &LichessGames) -> Result<LichessGames, Refusal> {
    let value =
        serde_json::to_value(request).map_err(|_| Refusal::from("Invalid Lichess download."))?;
    check::<LichessGames>(J::from(&value))
}

/// A download a frontend's form made, checked as the core will check it: the form says what is
/// wrong before it sends it.
pub fn player_games(request: &PlayerGames) -> Result<PlayerGames, Refusal> {
    let value =
        serde_json::to_value(request).map_err(|_| Refusal::from("Invalid player download."))?;
    check::<PlayerGames>(J::from(&value))
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
    min_moves: Option<u64>,
    max_moves: Option<u64>,
    speed: Option<ExplorerSpeed>,
    material: Option<String>,
    structure: Option<GameDbStructure>,
    pieces: Option<String>,
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
            ["minMoves" | "maxMoves"] => Some("Invalid number of moves."),
            ["material"] => Some(MATERIAL),
            ["pieces"] => Some(PIECES),
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
        for moves in sent.min_moves.iter().chain(&sent.max_moves) {
            within(*moves, 0, 600, Some("Invalid number of moves."), "")?;
        }
        let material = search_text(sent.material)?;
        if let Some(material) = &material {
            ensure(
                kchess_domain::gamedb::patterns::Material::parse(material).is_some(),
                Some(MATERIAL),
                "",
                String::new,
            )?;
        }
        let pieces = search_text(sent.pieces)?;
        if let Some(pieces) = &pieces {
            ensure(
                kchess_domain::gamedb::patterns::Placement::parse(pieces).is_some(),
                Some(PIECES),
                "",
                String::new,
            )?;
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
            min_moves: sent.min_moves.and_then(|moves| u32::try_from(moves).ok()),
            max_moves: sent.max_moves.and_then(|moves| u32::try_from(moves).ok()),
            speed: sent.speed,
            material,
            structure: sent.structure,
            pieces,
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
        assert_eq!(
            check::<PgnFilePath>(sent(json!(zst))).unwrap().packing(),
            PgnPacking::Zstd
        );
        let zip = if cfg!(windows) {
            "C:\\g\\twic1620g.ZIP"
        } else {
            "/g/twic1620g.ZIP"
        };
        assert_eq!(
            check::<PgnFilePath>(sent(json!(zip))).unwrap().packing(),
            PgnPacking::Zip
        );
        assert_eq!(
            check::<PgnFilePath>(sent(json!(absolute)))
                .unwrap()
                .packing(),
            PgnPacking::Plain
        );
        let seven = if cfg!(windows) {
            "C:\\g\\CCRL-4040.[2494855].pgn.7z"
        } else {
            "/g/CCRL-4040.[2494855].pgn.7z"
        };
        assert_eq!(
            check::<PgnFilePath>(sent(json!(seven))).unwrap().packing(),
            PgnPacking::SevenZip
        );
        let fics = if cfg!(windows) {
            "C:\\g\\ficsgamesdb_2024_standard2000_nomovetimes_1.PGN.BZ2"
        } else {
            "/g/ficsgamesdb_2024_standard2000_nomovetimes_1.PGN.BZ2"
        };
        assert_eq!(
            check::<PgnFilePath>(sent(json!(fics))).unwrap().packing(),
            PgnPacking::Bzip2
        );
        for bad in [
            "games/twic.pgn",
            "/games/twic.txt",
            "/games/twic.rar",
            "/games/twic.tar.bz2",
            "",
        ] {
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
    fn a_player_download_names_a_player_of_its_site() {
        let player = check::<PlayerGames>(sent(json!({
            "site": "lichess", "username": " Some_Player ", "months": 12,
            "speeds": ["rapid", "blitz", "rapid"], "rated": true
        })))
        .unwrap();
        assert_eq!(player.username, "Some_Player");
        assert_eq!(player.months, Some(12));
        assert_eq!(
            player.speeds,
            Some(vec![ExplorerSpeed::Blitz, ExplorerSpeed::Rapid])
        );
        assert!(player.rated);
        let plain = check::<PlayerGames>(sent(json!({ "site": "chesscom", "username": "hikaru" })))
            .unwrap();
        assert_eq!(
            (plain.months, plain.speeds, plain.rated),
            (None, None, false)
        );
        let refused = |value| check::<PlayerGames>(sent(value)).unwrap_err();
        assert_eq!(
            refused(json!({ "site": "lichess", "username": "no spaces" })),
            "Enter a valid Lichess username."
        );
        assert_eq!(
            refused(json!({ "site": "chesscom", "username": "x" })),
            "Enter a valid Chess.com username."
        );
        assert_eq!(
            refused(json!({ "site": "lichess", "username": "ab", "months": 0 })),
            "Invalid number of months."
        );
        for bad in [
            json!({ "site": "lichess", "username": "ab", "speeds": [] }),
            json!({ "site": "lichess", "username": "ab", "speeds": ["fast"] }),
            json!({ "site": "fics", "username": "ab" }),
            json!({ "site": "lichess", "username": "ab", "max": 3 }),
        ] {
            assert_eq!(refused(bad.clone()), "Invalid player download.", "{bad}");
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
        let deep = check::<GameDbQuery>(sent(json!({
            "database": "mine", "minMoves": 40, "maxMoves": 80, "speed": "blitz",
            "material": " R vs B ", "structure": "carlsbad", "pieces": " Nd5 vs e6 ",
            "offset": 0, "limit": 5
        })))
        .unwrap();
        assert_eq!(deep.pieces.as_deref(), Some("Nd5 vs e6"));
        assert_eq!(
            refused(json!({"database": "mine", "pieces": "Nd9", "offset": 0, "limit": 5})),
            PIECES
        );
        assert_eq!((deep.min_moves, deep.max_moves), (Some(40), Some(80)));
        assert_eq!(deep.speed, Some(ExplorerSpeed::Blitz));
        assert_eq!(deep.material.as_deref(), Some("R vs B"));
        assert_eq!(deep.structure, Some(GameDbStructure::Carlsbad));
        assert_eq!(
            refused(json!({"database": "mine", "material": "rook", "offset": 0, "limit": 5})),
            MATERIAL
        );
        assert_eq!(
            refused(json!({"database": "mine", "minMoves": 601, "offset": 0, "limit": 5})),
            "Invalid number of moves."
        );
        assert_eq!(
            refused(json!({"database": "mine", "structure": "x", "offset": 0, "limit": 5})),
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
