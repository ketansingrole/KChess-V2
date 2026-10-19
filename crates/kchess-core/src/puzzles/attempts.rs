//! The local training log in `puzzles.db`: every Practice, Downloaded and Rated attempt, kept by
//! the puzzle service alone. Totals and the day streak are derived from the log each time, never
//! kept as counters of their own. Deleting the downloaded puzzles leaves the log alone.

use rusqlite::{Connection, Row, params};
use serde::Deserialize;
use serde_json::{Value, json};

use kchess_domain::trainer::puzzle_progress::{Outcome, Verdict, classify, day_streak, moves};

/// Appended migration: the attempt log. Later changes append to it; nothing here is rewritten.
pub fn create_schema(db: &Connection) -> Result<(), String> {
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
    .map_err(|cause| cause.to_string())
}

/// `PuzzleAttemptInput`, already validated by the facade.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptInput {
    pub puzzle: AttemptPuzzle,
    pub mode: String,
    pub retry: bool,
    pub win: bool,
    pub revealed: bool,
    pub hinted: bool,
    pub found: u32,
    pub wrong: u32,
    pub total: u32,
    pub day: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptPuzzle {
    pub id: String,
    pub fen: String,
    #[serde(default)]
    pub last_move: Option<String>,
    pub solution: Vec<String>,
    pub rating: f64,
    pub themes: Vec<String>,
    #[serde(default)]
    pub game_id: Option<String>,
}

/// `PuzzleProgressQuery`.
#[derive(Debug, Clone, Deserialize)]
pub struct ProgressQuery {
    pub today: String,
    pub limit: u32,
}

/// How many attempts a recorded attempt's progress lists: the training page's strip.
pub const RECENT: u32 = 10;

fn sql_error(cause: rusqlite::Error) -> String {
    cause.to_string()
}

/// Keep `attempt` (classified here, as the session did), at `at` ms since the epoch.
pub fn record(db: &Connection, attempt: &AttemptInput, at: i64) -> Result<(), String> {
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
    db.execute(
        "INSERT INTO puzzle_attempts (at, day, puzzle_id, fen, last_move, solution, rating, themes,
            game_id, mode, retry, outcome, hinted, found, total, tried)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            attempt.mode,
            attempt.retry,
            outcome.name(),
            attempt.hinted,
            found,
            attempt.total.max(found),
            tried,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

fn words(text: &str) -> Vec<&str> {
    text.split(' ').filter(|word| !word.is_empty()).collect()
}

fn attempt_from(row: &Row<'_>) -> rusqlite::Result<Value> {
    let last_move: Option<String> = row.get("last_move")?;
    let game_id: Option<String> = row.get("game_id")?;
    let solution: String = row.get("solution")?;
    let themes: String = row.get("themes")?;
    let mut puzzle = json!({
        "id": row.get::<_, String>("puzzle_id")?,
        "fen": row.get::<_, String>("fen")?,
        "solution": words(&solution),
        "rating": row.get::<_, i64>("rating")?,
        "themes": words(&themes),
    });
    if let Some(last_move) = last_move {
        puzzle["lastMove"] = json!(last_move);
    }
    if let Some(game_id) = game_id {
        puzzle["gameId"] = json!(game_id);
    }
    Ok(json!({
        "id": row.get::<_, i64>("id")?,
        "at": row.get::<_, i64>("at")?,
        "day": row.get::<_, String>("day")?,
        "puzzle": puzzle,
        "mode": row.get::<_, String>("mode")?,
        "retry": row.get::<_, bool>("retry")?,
        "outcome": row.get::<_, String>("outcome")?,
        "hinted": row.get::<_, bool>("hinted")?,
        "found": row.get::<_, i64>("found")?,
        "wrong": row.get::<_, i64>("tried")? - row.get::<_, i64>("found")?,
        "total": row.get::<_, i64>("total")?,
    }))
}

/// `PuzzleTotals` of first attempts, of one day or of all.
fn totals(db: &Connection, day: Option<&str>) -> Result<Value, String> {
    let mut statement = db
        .prepare(
            "SELECT outcome, COUNT(*), SUM(found), SUM(tried) FROM puzzle_attempts
             WHERE retry = 0 AND (?1 IS NULL OR day = ?1) GROUP BY outcome",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![day], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(sql_error)?;
    let (mut attempts, mut found, mut tried) = (0i64, 0i64, 0i64);
    let mut by = [0i64; 5];
    for row in rows {
        let (outcome, count, moves_found, moves_tried) = row.map_err(sql_error)?;
        let Some(outcome) = Outcome::parse(&outcome) else {
            continue;
        };
        by[outcome as usize] += count;
        attempts += count;
        found += moves_found;
        tried += moves_tried;
    }
    Ok(json!({
        "attempts": attempts,
        "solved": by[Outcome::Solved as usize],
        "hinted": by[Outcome::Hinted as usize],
        "partial": by[Outcome::Partial as usize],
        "missed": by[Outcome::Missed as usize],
        "revealed": by[Outcome::Revealed as usize],
        "movesFound": found,
        "movesTried": tried,
    }))
}

/// `PuzzleProgress`: the newest `limit` attempts, the day streak as of `today`, and the totals.
pub fn progress(db: &Connection, query: &ProgressQuery) -> Result<Value, String> {
    let mut statement = db
        .prepare("SELECT * FROM puzzle_attempts ORDER BY id DESC LIMIT ?1")
        .map_err(sql_error)?;
    let recent = statement
        .query_map(params![query.limit], attempt_from)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<Value>>>()
        .map_err(sql_error)?;
    let mut statement = db
        .prepare(
            "SELECT DISTINCT day FROM puzzle_attempts
             WHERE retry = 0 AND outcome IN ('solved', 'hinted')",
        )
        .map_err(sql_error)?;
    let days = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(sql_error)?;
    let streak = day_streak(days.iter().map(String::as_str), &query.today);
    Ok(json!({
        "recent": recent,
        "dayStreak": streak.current,
        "bestDayStreak": streak.best,
        "today": totals(db, Some(&query.today))?,
        "all": totals(db, None)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(day: &str, win: bool, found: u32, revealed: bool, retry: bool) -> AttemptInput {
        AttemptInput {
            puzzle: AttemptPuzzle {
                id: "abc12".into(),
                fen: "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1".into(),
                last_move: Some("e7e8".into()),
                solution: vec!["d1d8".into()],
                rating: 1500.0,
                themes: vec!["mateIn1".into(), "short".into()],
                game_id: None,
            },
            mode: "practice".into(),
            retry,
            win,
            revealed,
            hinted: false,
            found,
            wrong: u32::from(!win && !revealed),
            total: 3,
            day: day.into(),
        }
    }

    fn setup() -> Connection {
        let db = Connection::open_in_memory().expect("memory db");
        create_schema(&db).expect("schema");
        db
    }

    fn query(today: &str) -> ProgressQuery {
        ProgressQuery {
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
        let progress = progress(&db, &query("2026-10-19")).expect("progress");
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
        progress(db, &query(today)).expect("progress")
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
