//! The player's own games as `kchess.db` holds them, for the game database the core keeps of
//! them ("Your games", `gamedb::own`): the synced Lichess and Chess.com games (`games`) and the
//! finished games of Game history (`archived_games`), read as one PGN text.
//!
//! The connection counts every change to those games in a temporary table its triggers keep
//! (`watch`), whichever store function wrote them, and `Database` tells the game database after
//! each write that changed one (`Database::own_games_changes`). Nothing of it is stored: the
//! table and triggers live with the connection.

use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

use super::sql::Sql as _;
use crate::host::Host;
use kchess_contracts::error::Result;
use kchess_domain::library::PlayedGame;
use kchess_domain::misc::constants::HistoryKind;
use kchess_domain::pgn::{Game, PgnNode, make_pgn};

/// Whether a played game is finished: a game still being played changes with every move, and
/// joins the database when it ends. A body that is not JSON counts as unfinished rather than
/// failing the write that stores it.
const FINISHED: &str = "CASE WHEN json_valid(body) THEN json_extract(body, '$.finished') END";

/// Changes the next fingerprint whatever the games hold: raise it when the PGN they are written
/// as changes, so every database is written again.
const FORMAT: &str = "own-games 1";

/// The triggers that count changes to the own games in `own_games_changes`.
pub fn watch(db: &Connection) -> Result<()> {
    let finished = |row: &str| FINISHED.replace("body", &format!("{row}.body"));
    let bump = "BEGIN UPDATE own_games_changes SET n = n + 1; END;";
    db.execute_batch(&format!(
        "CREATE TEMP TABLE IF NOT EXISTS own_games_changes (n INTEGER NOT NULL);
         DELETE FROM own_games_changes;
         INSERT INTO own_games_changes VALUES (0);
         CREATE TEMP TRIGGER IF NOT EXISTS own_games_added AFTER INSERT ON main.games {bump}
         CREATE TEMP TRIGGER IF NOT EXISTS own_games_changed AFTER UPDATE ON main.games {bump}
         CREATE TEMP TRIGGER IF NOT EXISTS own_games_removed AFTER DELETE ON main.games {bump}
         CREATE TEMP TRIGGER IF NOT EXISTS own_played_added AFTER INSERT ON main.archived_games
           WHEN {new} {bump}
         CREATE TEMP TRIGGER IF NOT EXISTS own_played_changed AFTER UPDATE ON main.archived_games
           WHEN {new} OR {old} {bump}
         CREATE TEMP TRIGGER IF NOT EXISTS own_played_removed AFTER DELETE ON main.archived_games
           WHEN {old} {bump}",
        new = finished("NEW"),
        old = finished("OLD"),
    ))
    .sql()
}

/// Whether the own games changed since the last time this was asked.
pub fn take_changes(db: &Connection) -> Result<bool> {
    let changed = db
        .execute("UPDATE own_games_changes SET n = 0 WHERE n != 0", [])
        .sql()?;
    Ok(changed > 0)
}

/// The own games, read when they differ from those a database was written from.
pub enum OwnGames {
    /// There are none.
    Empty,
    /// The same as the fingerprint the database was written from.
    Unchanged,
    /// Their PGN text, and the fingerprint the database written from it keeps.
    Changed { fingerprint: String, pgn: String },
}

/// The own games, unless their fingerprint is `stored`: the database written from them is
/// still theirs.
pub fn own_games(host: &dyn Host, db: &Connection, stored: Option<&str>) -> Result<OwnGames> {
    let (fingerprint, count) = fingerprint(db)?;
    if count == 0 {
        return Ok(OwnGames::Empty);
    }
    if stored == Some(fingerprint.as_str()) {
        return Ok(OwnGames::Unchanged);
    }
    let mut pgn = String::new();
    let mut add = |text: &str| {
        let text = text.trim();
        if !text.is_empty() {
            pgn.push_str(text);
            pgn.push_str("\n\n");
        }
    };
    let mut statement = db
        .prepare(
            "SELECT account, color, opponent, winner, status, createdAt, playerRating,
               opponentRating, moves, pgn
             FROM games ORDER BY createdAt, account, id",
        )
        .sql()?;
    let mut rows = statement.query([]).sql()?;
    while let Some(row) = rows.next().sql()? {
        let text: Option<String> = row.get(9).sql()?;
        match text.filter(|text| !text.trim().is_empty()) {
            Some(text) => add(&text),
            None => add(&written(row)?),
        }
    }
    let mut played = super::library_import::read_games(host, db)?;
    // Oldest first, as the synced games are.
    played.reverse();
    for game in played
        .iter()
        .filter(|game| game.finished && !game.moves.is_empty())
    {
        if let Some(text) = played_pgn(game) {
            add(&text);
        }
    }
    if pgn.is_empty() {
        return Ok(OwnGames::Empty);
    }
    Ok(OwnGames::Changed { fingerprint, pgn })
}

/// What the own games are, in brief: each synced game's identity, its last move and the size of
/// its text, and each finished played game's identity and save. A cheap read that changes when
/// any of them does; and how many games there are.
fn fingerprint(db: &Connection) -> Result<(String, u64)> {
    let mut hasher = Sha256::new();
    hasher.update(FORMAT.as_bytes());
    let mut count = 0u64;
    let mut statement = db
        .prepare(
            "SELECT account, id, lastMoveAt, status, length(pgn), length(moves)
             FROM games ORDER BY account, id",
        )
        .sql()?;
    let mut rows = statement.query([]).sql()?;
    while let Some(row) = rows.next().sql()? {
        let line = format!(
            "g\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1e}",
            row.get::<_, Option<String>>(0).sql()?,
            row.get::<_, Option<String>>(1).sql()?,
            row.get::<_, Option<i64>>(2).sql()?,
            row.get::<_, Option<String>>(3).sql()?,
            row.get::<_, Option<i64>>(4).sql()?,
            row.get::<_, Option<i64>>(5).sql()?,
        );
        hasher.update(line.as_bytes());
        count += 1;
    }
    let mut statement = db
        .prepare(&format!(
            "SELECT id, seq FROM archived_games WHERE {FINISHED} ORDER BY id"
        ))
        .sql()?;
    let mut rows = statement.query([]).sql()?;
    while let Some(row) = rows.next().sql()? {
        let line = format!(
            "p\u{1f}{}\u{1f}{}\u{1e}",
            row.get::<_, String>(0).sql()?,
            row.get::<_, i64>(1).sql()?,
        );
        hasher.update(line.as_bytes());
        count += 1;
    }
    Ok((kchess_delta::hex(&hasher.finalize()), count))
}

/// A synced game stored without its PGN, written from its columns: its players, result, date,
/// ratings and moves.
fn written(row: &rusqlite::Row<'_>) -> Result<String> {
    let account: String = row.get(0).sql()?;
    let color: Option<String> = row.get(1).sql()?;
    let opponent: Option<String> = row.get(2).sql()?;
    let winner: Option<String> = row.get(3).sql()?;
    let status: Option<String> = row.get(4).sql()?;
    let created_at: Option<i64> = row.get(5).sql()?;
    let player_rating: Option<i64> = row.get(6).sql()?;
    let opponent_rating: Option<i64> = row.get(7).sql()?;
    let moves: Option<String> = row.get(8).sql()?;
    let Some(moves) = moves.filter(|moves| !moves.trim().is_empty()) else {
        return Ok(String::new());
    };
    let white_player = color.as_deref() != Some("black");
    let opponent = opponent.unwrap_or_default();
    let (white, black) = if white_player {
        (account.as_str(), opponent.as_str())
    } else {
        (opponent.as_str(), account.as_str())
    };
    let (white_elo, black_elo) = if white_player {
        (player_rating, opponent_rating)
    } else {
        (opponent_rating, player_rating)
    };
    let result = match (winner.as_deref(), status.as_deref()) {
        (Some("white"), _) => "1-0",
        (Some("black"), _) => "0-1",
        (None, Some("draw" | "stalemate")) => "1/2-1/2",
        _ => "*",
    };
    let mut game = Game::empty();
    game.put_header("White", white.to_string());
    game.put_header("Black", black.to_string());
    game.put_header("Result", result.to_string());
    if let Some(date) = created_at.and_then(pgn_date) {
        game.put_header("Date", date);
    }
    for (name, elo) in [("WhiteElo", white_elo), ("BlackElo", black_elo)] {
        if let Some(elo) = elo {
            game.put_header(name, elo.to_string());
        }
    }
    let mut parent = 0;
    for san in moves.split_whitespace() {
        parent = game.add_move(parent, PgnNode::with(san, None, None, None));
    }
    Ok(make_pgn(&game))
}

/// PGN's `Date` of a time in milliseconds: the UTC day, "2026.10.22".
fn pgn_date(millis: i64) -> Option<String> {
    let iso = kchess_domain::calendar::iso_millis(millis)?;
    Some(iso.get(..10)?.replace('-', "."))
}

/// A game of Game history as PGN, with the headers Game history exports it with.
fn played_pgn(game: &PlayedGame) -> Option<String> {
    let event = if game.source == HistoryKind::Computer {
        "KChess computer game"
    } else {
        "KChess over-the-board game"
    };
    let date = pgn_date(game.started_at as i64).unwrap_or_else(|| "????.??.??".to_string());
    kchess_domain::pgn::setup_pgn(
        game.setup.variant,
        &game.setup.fen,
        &game.moves,
        &[
            ("Event", event),
            ("White", &game.white),
            ("Black", &game.black),
            ("Result", &game.result),
            ("Date", &date),
        ],
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_the_utc_day() {
        assert_eq!(pgn_date(0).as_deref(), Some("1970.01.01"));
        assert_eq!(
            pgn_date(1_761_091_200_000 + 86_399_999).as_deref(),
            Some("2025.10.22")
        );
    }
}
