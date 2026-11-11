//! The local training log in `puzzles.db`: every Practice, Downloaded and Rated attempt, kept by
//! the puzzle service alone. Totals and the day streak are derived from the log each time, never
//! kept as counters of their own. Deleting the downloaded puzzles leaves the log alone.

use kchess_contracts::types::{
    Puzzle, PuzzleAttempt, PuzzleAttemptInput, PuzzleProgress, PuzzleProgressQuery, PuzzleTotals,
    PuzzleWrongMove,
};
use rusqlite::{Connection, Row, params};

use crate::error::Result;
use crate::store::sql::Sql as _;

use kchess_domain::trainer::puzzle_progress::{Outcome, Verdict, classify, day_streak, moves};

/// Appended migration: the attempt log. Later changes append to it; nothing here is rewritten.
pub fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS puzzle_attempts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at INTEGER NOT NULL,
  day TEXT NOT NULL,
  puzzle_id TEXT NOT NULL,
  fen TEXT NOT NULL,
  last_move TEXT,
  solution TEXT NOT NULL,
  rating INTEGER NOT NULL,
  themes TEXT NOT NULL,
  game_id TEXT,
  mode TEXT NOT NULL,
  retry INTEGER NOT NULL,
  outcome TEXT NOT NULL,
  hinted INTEGER NOT NULL,
  found INTEGER NOT NULL,
  total INTEGER NOT NULL,
  tried INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_puzzle_attempts_day ON puzzle_attempts(day);",
    )
    .sql()?;
    // Appended migration: what was played at each wrong first try, as `index:uci` words.
    let has_wrong_moves = db
        .prepare("SELECT 1 FROM pragma_table_info('puzzle_attempts') WHERE name = 'wrong_moves'")
        .and_then(|mut statement| statement.exists([]))
        .sql()?;
    if !has_wrong_moves {
        db.execute_batch(
            "ALTER TABLE puzzle_attempts ADD COLUMN wrong_moves TEXT NOT NULL DEFAULT ''",
        )
        .sql()?;
    }
    Ok(())
}

/// How many attempts a recorded attempt's progress lists: the training page's strip.
pub const RECENT: u32 = 10;

/// Keep `attempt` (classified here, as the session did), at `at` ms since the epoch.
pub fn record(db: &Connection, attempt: &PuzzleAttemptInput, at: i64) -> Result<()> {
    let verdict = Verdict {
        win: attempt.win,
        revealed: attempt.revealed,
        hinted: attempt.hinted,
        found: attempt.found,
        wrong: attempt.wrong,
        total: attempt.total,
    };
    let outcome = classify(&verdict);
    let (found, tried) = moves(outcome, &verdict);
    let puzzle = &attempt.puzzle;
    let wrong_moves: Vec<String> = attempt
        .wrong_moves
        .iter()
        .flatten()
        .map(|entry| format!("{}:{}", entry.index, entry.uci))
        .collect();
    db.execute(
        "INSERT INTO puzzle_attempts (at, day, puzzle_id, fen, last_move, solution, rating, themes,
            game_id, mode, retry, outcome, hinted, found, total, tried, wrong_moves)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            at,
            attempt.day,
            puzzle.id,
            puzzle.fen,
            puzzle.last_move,
            puzzle.solution.join(" "),
            puzzle.rating as i64,
            puzzle.themes.join(" "),
            puzzle.game_id,
            attempt.mode.as_str(),
            attempt.retry,
            outcome.name(),
            attempt.hinted,
            found,
            attempt.total.max(found),
            tried,
            wrong_moves.join(" "),
        ],
    )
    .sql()?;
    Ok(())
}

fn words(text: &str) -> Vec<&str> {
    text.split(' ').filter(|word| !word.is_empty()).collect()
}

fn attempt_from(row: &Row<'_>) -> rusqlite::Result<PuzzleAttempt> {
    let solution: String = row.get("solution")?;
    let themes: String = row.get("themes")?;
    let owned = |text: &str| words(text).into_iter().map(str::to_owned).collect();
    let puzzle = Puzzle {
        id: row.get("puzzle_id")?,
        fen: row.get("fen")?,
        last_move: row.get("last_move")?,
        solution: owned(&solution),
        rating: row.get::<_, i64>("rating")? as f64,
        themes: owned(&themes),
        plays: None,
        game_id: row.get("game_id")?,
    };
    let wrong_moves: String = row.get("wrong_moves")?;
    let wrong_moves: Vec<PuzzleWrongMove> = words(&wrong_moves)
        .into_iter()
        .filter_map(|word| {
            let (index, uci) = word.split_once(':')?;
            Some(PuzzleWrongMove {
                index: index.parse().ok()?,
                uci: uci.to_owned(),
            })
        })
        .collect();
    let found: u32 = row.get("found")?;
    Ok(PuzzleAttempt {
        id: row.get("id")?,
        at: row.get("at")?,
        day: row.get("day")?,
        puzzle,
        mode: logged(row, "mode")?,
        retry: row.get("retry")?,
        outcome: logged(row, "outcome")?,
        hinted: row.get("hinted")?,
        found,
        wrong: row.get::<_, u32>("tried")?.saturating_sub(found),
        total: row.get("total")?,
        wrong_moves: (!wrong_moves.is_empty()).then_some(wrong_moves),
    })
}

/// A word column read as its wire enum; a word no build of KChess writes is an error.
fn logged<T: serde::de::DeserializeOwned>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    let word: String = row.get(column)?;
    serde_json::from_value(serde_json::Value::String(word)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
    })
}

/// `PuzzleTotals` of first attempts, of one day or of all.
fn totals(db: &Connection, day: Option<&str>) -> Result<PuzzleTotals> {
    let mut statement = db
        .prepare(
            "SELECT outcome, COUNT(*), SUM(found), SUM(tried) FROM puzzle_attempts
             WHERE retry = 0 AND (?1 IS NULL OR day = ?1) GROUP BY outcome",
        )
        .sql()?;
    let rows = statement
        .query_map(params![day], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .sql()?;
    let (mut attempts, mut found, mut tried) = (0, 0, 0);
    let mut by = [0i64; 5];
    for row in rows {
        let (outcome, count, moves_found, moves_tried) = row.sql()?;
        let Some(outcome) = Outcome::parse(&outcome) else {
            continue;
        };
        by[outcome as usize] += count;
        attempts += count;
        found += moves_found;
        tried += moves_tried;
    }
    Ok(PuzzleTotals {
        attempts,
        solved: by[Outcome::Solved as usize],
        hinted: by[Outcome::Hinted as usize],
        partial: by[Outcome::Partial as usize],
        missed: by[Outcome::Missed as usize],
        revealed: by[Outcome::Revealed as usize],
        moves_found: found,
        moves_tried: tried,
    })
}

/// `PuzzleProgress`: the newest `limit` attempts, the day streak as of `today`, and the totals.
pub fn progress(db: &Connection, query: &PuzzleProgressQuery) -> Result<PuzzleProgress> {
    let mut statement = db
        .prepare("SELECT * FROM puzzle_attempts ORDER BY id DESC LIMIT ?1")
        .sql()?;
    let recent = statement
        .query_map(params![query.limit], attempt_from)
        .sql()?
        .collect::<rusqlite::Result<Vec<PuzzleAttempt>>>()
        .sql()?;
    let mut statement = db
        .prepare(
            "SELECT DISTINCT day FROM puzzle_attempts
             WHERE retry = 0 AND outcome IN ('solved', 'hinted')",
        )
        .sql()?;
    let days = statement
        .query_map([], |row| row.get::<_, String>(0))
        .sql()?
        .collect::<rusqlite::Result<Vec<String>>>()
        .sql()?;
    let streak = day_streak(days.iter().map(String::as_str), &query.today);
    Ok(PuzzleProgress {
        recent,
        day_streak: streak.current,
        best_day_streak: streak.best,
        today: totals(db, Some(&query.today))?,
        all: totals(db, None)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_contracts::types::PuzzleTrainMode;
    use serde_json::{Value, json};

    fn attempt(
        day: &str,
        win: bool,
        found: u32,
        revealed: bool,
        retry: bool,
    ) -> PuzzleAttemptInput {
        PuzzleAttemptInput {
            puzzle: Puzzle {
                id: "abc12".into(),
                fen: "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1".into(),
                last_move: Some("e7e8".into()),
                solution: vec!["d1d8".into()],
                rating: 1500.0,
                themes: vec!["mateIn1".into(), "short".into()],
                plays: None,
                game_id: None,
            },
            mode: PuzzleTrainMode::Practice,
            retry,
            win,
            revealed,
            hinted: false,
            found,
            wrong: u32::from(!win && !revealed),
            total: 3,
            day: day.into(),
            wrong_moves: None,
        }
    }

    fn setup() -> Connection {
        let db = Connection::open_in_memory().expect("memory db");
        create_schema(&db).expect("schema");
        db
    }

    fn query(today: &str) -> PuzzleProgressQuery {
        PuzzleProgressQuery {
            today: today.into(),
            limit: 10,
        }
    }

    #[test]
    fn derives_totals_and_the_streak_from_the_log() {
        let db = setup();
        record(&db, &attempt("2026-10-17", true, 3, false, false), 1).expect("record");
        record(&db, &attempt("2026-10-18", true, 3, false, false), 2).expect("record");
        record(&db, &attempt("2026-10-19", false, 2, false, false), 3).expect("record");
        record(&db, &attempt("2026-10-19", false, 1, true, false), 4).expect("record");
        record(&db, &attempt("2026-10-19", true, 3, false, true), 5).expect("record");
        let progress = progress_at(&db, "2026-10-19");
        // Today has no first-attempt solve yet (the retry does not count): yesterday's run stands.
        assert_eq!(progress["dayStreak"], 2);
        assert_eq!(progress["bestDayStreak"], 2);
        assert_eq!(progress["today"]["attempts"], 2);
        assert_eq!(progress["today"]["partial"], 1);
        assert_eq!(progress["today"]["revealed"], 1);
        assert_eq!(progress["today"]["movesFound"], 3);
        assert_eq!(progress["today"]["movesTried"], 4);
        assert_eq!(progress["all"]["solved"], 2);
        assert_eq!(progress["all"]["movesFound"], 9);
        assert_eq!(progress["all"]["movesTried"], 10);
        let recent = progress["recent"].as_array().expect("recent");
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0]["retry"], true);
        assert_eq!(recent[1]["outcome"], "revealed");
        assert_eq!(recent[2]["outcome"], "partial");
        assert_eq!(recent[2]["found"], 2);
        assert_eq!(recent[2]["puzzle"]["solution"], json!(["d1d8"]));
        assert_eq!(recent[2]["puzzle"]["lastMove"], "e7e8");
        assert!(recent[2]["puzzle"].get("gameId").is_none());
        // A missed day ends the streak.
        assert_eq!(progress_at(&db, "2026-10-20")["dayStreak"], 0);
    }

    #[test]
    fn keeps_wrong_first_tries_after_the_first_mistake() {
        let db = setup();
        // Wrong on the first move, right on the second, wrong on the third.
        let mut mixed = attempt("2026-10-19", false, 1, false, false);
        mixed.wrong = 2;
        record(&db, &mixed, 1).expect("record");
        let progress = progress_at(&db, "2026-10-19");
        assert_eq!(progress["recent"][0]["outcome"], "partial");
        assert_eq!(progress["recent"][0]["found"], 1);
        assert_eq!(progress["recent"][0]["wrong"], 2);
        assert_eq!(progress["all"]["movesFound"], 1);
        assert_eq!(progress["all"]["movesTried"], 3);
    }

    fn progress_at(db: &Connection, today: &str) -> Value {
        serde_json::to_value(progress(db, &query(today)).expect("progress")).expect("json")
    }

    #[test]
    fn keeps_what_was_played_at_each_wrong_first_try() {
        let db = setup();
        let mut missed = attempt("2026-10-19", false, 1, false, false);
        missed.wrong_moves = Some(vec![
            PuzzleWrongMove {
                index: 0,
                uci: "d1d7".into(),
            },
            PuzzleWrongMove {
                index: 2,
                uci: "g1h1".into(),
            },
        ]);
        record(&db, &missed, 1).expect("record");
        record(&db, &attempt("2026-10-19", true, 3, false, false), 2).expect("record");
        let recent = progress_at(&db, "2026-10-19")["recent"].clone();
        assert!(recent[0].get("wrongMoves").is_none());
        assert_eq!(
            recent[1]["wrongMoves"],
            json!([{ "index": 0, "uci": "d1d7" }, { "index": 2, "uci": "g1h1" }])
        );
    }

    #[test]
    fn adds_the_wrong_moves_to_a_log_from_before_them() {
        let db = Connection::open_in_memory().expect("memory db");
        db.execute_batch(
            "CREATE TABLE puzzle_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
               day TEXT NOT NULL, puzzle_id TEXT NOT NULL, fen TEXT NOT NULL, last_move TEXT,
               solution TEXT NOT NULL, rating INTEGER NOT NULL, themes TEXT NOT NULL, game_id TEXT,
               mode TEXT NOT NULL, retry INTEGER NOT NULL, outcome TEXT NOT NULL,
               hinted INTEGER NOT NULL, found INTEGER NOT NULL, total INTEGER NOT NULL,
               tried INTEGER NOT NULL);
             INSERT INTO puzzle_attempts (at, day, puzzle_id, fen, solution, rating, themes, mode,
               retry, outcome, hinted, found, total, tried)
             VALUES (1, '2026-10-19', 'abc12', '6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1', 'd1d8',
               1500, 'mateIn1', 'practice', 0, 'missed', 0, 0, 1, 1);",
        )
        .expect("earlier log");
        create_schema(&db).expect("migrated");
        create_schema(&db).expect("migrated again");
        let recent = progress_at(&db, "2026-10-19")["recent"].clone();
        assert_eq!(recent[0]["outcome"], "missed");
        assert!(recent[0].get("wrongMoves").is_none());
    }

    #[test]
    fn the_schema_is_appended_to_an_earlier_database() {
        let db = Connection::open_in_memory().expect("memory db");
        db.execute_batch("CREATE TABLE puzzles (id TEXT PRIMARY KEY);")
            .expect("old schema");
        create_schema(&db).expect("migrated");
        record(&db, &attempt("2026-10-19", true, 3, false, false), 1).expect("record");
        create_schema(&db).expect("migrated again");
        assert_eq!(progress_at(&db, "2026-10-19")["all"]["attempts"], 1);
    }
}
