//! Queries over the offline puzzle database: the Rust port of `core/src/services/puzzleQueries.ts`
//! and the schema of `core/src/services/puzzleWorker.ts`. Only the puzzle database's single owner
//! calls these; they take the connection, never open one.
//!
//! Randomness: the TypeScript uses `Math.random()`. Here the caller supplies the generator as
//! `random`, a function returning values in `[0, 1)` (the worker supplies a real one, tests a
//! seeded one). No SQL `RANDOM()` is used, so the indexed plans stay the same.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Statement, ToSql, params};
use serde::{Deserialize, Serialize, Serializer};

use kchess_domain::{position, rules};

/// A row of the puzzle table as the importer hands it over (`DbPuzzle` in `domain/puzzle.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbPuzzle {
    pub id: String,
    pub fen: String,
    pub moves: String,
    pub rating: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plays: Option<f64>,
    pub themes: String,
}

/// One puzzle, whichever source it came from (`Puzzle` in `contracts/types.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Puzzle {
    pub id: String,
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub solution: Vec<String>,
    #[serde(serialize_with = "serialize_js_number")]
    pub rating: f64,
    pub themes: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_js_number_opt"
    )]
    pub plays: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

/// `PuzzleDbStatus` in `contracts/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDbStatus {
    pub installed: bool,
    /// Approximate size on disk.
    pub count: i64,
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<i64>,
    /// True while a download runs; its progress arrives as events.
    pub busy: bool,
}

/// `LocalPuzzleQuery` in `contracts/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPuzzleQuery {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub min_rating: Option<f64>,
    #[serde(default)]
    pub max_rating: Option<f64>,
    pub count: f64,
}

/// `LocalLadderQuery` in `contracts/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLadderQuery {
    pub from: f64,
    pub to: f64,
    pub count: f64,
}

/// JavaScript prints whole numbers without a fraction; match that in the JSON.
fn serialize_js_number<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    if value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE {
        if *value < 0.0 {
            serializer.serialize_i64(*value as i64)
        } else {
            serializer.serialize_u64(*value as u64)
        }
    } else {
        serializer.serialize_f64(*value)
    }
}

fn serialize_js_number_opt<S: Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(number) => serialize_js_number(number, serializer),
        None => serializer.serialize_none(),
    }
}

const COLUMNS: &str = "SELECT id, fen, moves, rating, plays, themes FROM puzzles";

/// The `CREATE` statements and PRAGMAs of `puzzleWorker.ts`, run in the same order.
pub fn create_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS puzzles (id TEXT PRIMARY KEY, fen TEXT NOT NULL, moves TEXT NOT NULL, rating INTEGER NOT NULL, plays INTEGER NOT NULL DEFAULT 0, themes TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles(rating);
CREATE TABLE IF NOT EXISTS puzzle_meta (id INTEGER PRIMARY KEY CHECK(id = 1), importedAt INTEGER NOT NULL, count INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS worker_meta (id INTEGER PRIMARY KEY CHECK(id = 1));",
    )
    .map_err(|cause| cause.to_string())
}

fn sql_error(cause: rusqlite::Error) -> String {
    cause.to_string()
}

pub fn read_status(db: &Connection, busy: bool) -> Result<PuzzleDbStatus, String> {
    struct Meta {
        imported_at: i64,
        count: i64,
        bytes: i64,
    }
    let mut statement = db
        .prepare("SELECT importedAt, count, bytes FROM puzzle_meta WHERE id = 1")
        .map_err(sql_error)?;
    let meta = statement
        .query_row([], |row| {
            Ok(Meta {
                imported_at: row.get(0)?,
                count: row.get(1)?,
                bytes: row.get(2)?,
            })
        })
        .optional()
        .map_err(sql_error)?;
    Ok(PuzzleDbStatus {
        installed: meta.as_ref().is_some_and(|meta| meta.count > 0),
        count: meta.as_ref().map_or(0, |meta| meta.count),
        bytes: meta.as_ref().map_or(0, |meta| meta.bytes),
        imported_at: meta.as_ref().map(|meta| meta.imported_at),
        busy,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Replace the stored puzzles with `rows` in one transaction; a failure keeps the old ones.
pub fn store_sample(
    db: &Connection,
    rows: &[DbPuzzle],
    cancelled: &dyn Fn() -> bool,
) -> Result<usize, String> {
    let mut insert = db
        .prepare(
            "INSERT OR IGNORE INTO puzzles (id, fen, moves, rating, plays, themes) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .map_err(sql_error)?;
    db.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)?;
    match write_sample(db, &mut insert, rows, cancelled) {
        Ok(count) => Ok(count),
        Err(cause) => {
            // A failing ROLLBACK replaces the cause, as it would throw in the TypeScript.
            db.execute_batch("ROLLBACK").map_err(sql_error)?;
            Err(cause)
        }
    }
}

fn write_sample(
    db: &Connection,
    insert: &mut Statement<'_>,
    rows: &[DbPuzzle],
    cancelled: &dyn Fn() -> bool,
) -> Result<usize, String> {
    db.execute_batch("DELETE FROM puzzles").map_err(sql_error)?;
    let mut count: usize = 0;
    let mut bytes: i64 = 0;
    for row in rows {
        // Skip anything the app could not play (a malformed line).
        if puzzle_from_db(row).is_none() {
            continue;
        }
        if cancelled() {
            return Err("Puzzle import cancelled.".to_string());
        }
        let changes = insert
            .execute(params![
                row.id,
                row.fen,
                row.moves,
                row.rating,
                row.plays.unwrap_or(0.0),
                row.themes
            ])
            .map_err(sql_error)?;
        count += changes;
        if changes > 0 {
            // Buffer.byteLength of the concatenation is the sum of the UTF-8 lengths.
            let length = row.id.len() + row.fen.len() + row.moves.len() + row.themes.len() + 24;
            bytes += i64::try_from(length).unwrap_or(i64::MAX);
        }
    }
    if count == 0 {
        return Err("The downloaded file held no usable puzzles.".to_string());
    }
    if cancelled() {
        return Err("Puzzle import cancelled.".to_string());
    }
    db.execute(
        "INSERT OR REPLACE INTO puzzle_meta (id, importedAt, count, bytes) VALUES (1, ?, ?, ?)",
        params![now_ms(), i64::try_from(count).unwrap_or(i64::MAX), bytes],
    )
    .map_err(sql_error)?;
    db.execute_batch("COMMIT").map_err(sql_error)?;
    Ok(count)
}

pub fn clear_stored(db: &Connection) -> Result<(), String> {
    db.execute_batch("DELETE FROM puzzles; DELETE FROM puzzle_meta;")
        .map_err(sql_error)?;
    // Give the space back to the file rather than leave it for reuse.
    db.execute_batch("VACUUM").map_err(sql_error)
}

fn require_installed(db: &Connection) -> Result<(), String> {
    let mut statement = db
        .prepare("SELECT count FROM puzzle_meta WHERE id = 1 AND count > 0")
        .map_err(sql_error)?;
    let found = statement
        .query_row([], |_| Ok(()))
        .optional()
        .map_err(sql_error)?;
    found.ok_or_else(|| {
        "Download the puzzle database first (Settings → Data & storage, or the Rush tab)."
            .to_string()
    })
}

/// `puzzleFromDb`: the first move of `moves` is the opponent's, played from `fen`; the rest is
/// the solution. Rows the rules cannot play are dropped.
fn puzzle_from_db(row: &DbPuzzle) -> Option<Puzzle> {
    let mut tokens = row.moves.split(' ').filter(|token| !token.is_empty());
    let first = tokens.next()?;
    let solution: Vec<String> = tokens.map(str::to_string).collect();
    // Position.fromFen re-derives the position from its normalized FEN before playing.
    let start = rules::standard_from_fen(&row.fen)?;
    let normalized = rules::make_fen(&start);
    let position = rules::standard_from_fen(&normalized)?;
    let played = position::play_uci(&position, first)?;
    if solution.is_empty() {
        return None;
    }
    Some(Puzzle {
        id: row.id.clone(),
        fen: played.position.fen,
        last_move: Some(first.to_string()),
        solution,
        rating: row.rating,
        themes: row
            .themes
            .split(' ')
            .filter(|theme| !theme.is_empty())
            .map(str::to_string)
            .collect(),
        plays: row.plays,
        game_id: None,
    })
}

fn to_puzzles(rows: &[DbPuzzle]) -> Vec<Puzzle> {
    rows.iter().filter_map(puzzle_from_db).collect()
}

fn fetch(db: &Connection, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<DbPuzzle>, String> {
    let mut statement = db.prepare(sql).map_err(sql_error)?;
    let rows = statement
        .query_map(params, |row| {
            Ok(DbPuzzle {
                id: row.get(0)?,
                fen: row.get(1)?,
                moves: row.get(2)?,
                rating: row.get(3)?,
                plays: Some(row.get(4)?),
                themes: row.get(5)?,
            })
        })
        .map_err(sql_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)
}

/// Indexed candidate pool (no sort): the rating index answers the range, randomness happens here.
const CANDIDATE_CAP: i64 = 5_000;

/// Partial Fisher-Yates over `items` (reordered in place), keeping `count` (clamped to the length,
/// truncated like `Array.prototype.slice`).
fn take_random<T: Clone>(items: &mut [T], count: f64, random: &mut dyn FnMut() -> f64) -> Vec<T> {
    let length = items.len();
    let keep = count.min(length as f64).max(0.0);
    let mut i = 0_usize;
    while (i as f64) < keep {
        let remaining = length - i;
        let offset = ((random() * remaining as f64).floor().max(0.0) as usize).min(remaining - 1);
        items.swap(i, i + offset);
        i += 1;
    }
    items.iter().take(keep.trunc() as usize).cloned().collect()
}

/// Whole-token theme match (`' ' + themes + ' '` contains `' ' + theme + ' '`).
fn has_theme(row: &DbPuzzle, theme: &str) -> bool {
    format!(" {} ", row.themes).contains(&format!(" {theme} "))
}

/// Random unused rows from one rating window with indexed seeks (no COUNT, no OFFSET walk, no
/// sort). A random pivot splits the window into two index-range scans.
fn pick_window(
    db: &Connection,
    min: f64,
    max: f64,
    used: &HashSet<String>,
    random: &mut dyn FnMut() -> f64,
) -> Result<Vec<Puzzle>, String> {
    if min > max {
        return Ok(Vec::new());
    }
    let pivot = min + random() * (max - min);
    let rows = fetch(
        db,
        &format!("{COLUMNS} WHERE rating BETWEEN ? AND ? AND rating >= ? LIMIT 8"),
        params![min, max, pivot],
    )?;
    let extra = if rows.len() < 8 {
        let below = fetch(
            db,
            &format!("{COLUMNS} WHERE rating BETWEEN ? AND ? AND rating < ? LIMIT 8"),
            params![min, max, pivot],
        )?;
        below
            .into_iter()
            .filter(|row| !rows.iter().any(|other| other.id == row.id))
            .collect()
    } else {
        Vec::new()
    };
    let mut combined = rows;
    combined.extend(extra);
    Ok(to_puzzles(&combined)
        .into_iter()
        .filter(|puzzle| !used.contains(&puzzle.id))
        .collect())
}

/// Random puzzles, optionally of one theme and rating range.
pub fn query_puzzles(
    db: &Connection,
    query: &LocalPuzzleQuery,
    random: &mut dyn FnMut() -> f64,
) -> Result<Vec<Puzzle>, String> {
    require_installed(db)?;
    let min = query.min_rating.unwrap_or(0.0);
    let max = query.max_rating.unwrap_or(4000.0);
    let count = query.count.max(0.0);
    if count == 0.0 {
        return Ok(Vec::new());
    }
    let theme = query
        .theme
        .as_deref()
        .filter(|theme| !theme.is_empty() && *theme != "mix");
    // The theme prefilter runs in SQLite so a rare theme no longer transfers 5000 non-matching
    // rows; `has_theme` stays authoritative for exact tokens (`_` is a LIKE wildcard).
    let rows = match theme {
        Some(theme) => fetch(
            db,
            &format!(
                "{COLUMNS} WHERE rating BETWEEN ? AND ? AND (' ' || themes || ' ' LIKE ' %' || ? || ' %') LIMIT ?"
            ),
            params![min, max, theme, CANDIDATE_CAP],
        )?,
        None => fetch(
            db,
            &format!("{COLUMNS} WHERE rating BETWEEN ? AND ? LIMIT ?"),
            params![min, max, CANDIDATE_CAP],
        )?,
    };
    let mut pool: Vec<DbPuzzle> = match theme {
        Some(theme) => rows
            .into_iter()
            .filter(|row| has_theme(row, theme))
            .collect(),
        None => rows,
    };
    let mut picked = take_random(&mut pool, count, random);
    // A rare theme in a wide range can truncate below `count`: top up with indexed pivot seeks
    // (up to 8 candidates per seek, no OFFSET walk).
    if let Some(theme) = theme
        && (picked.len() as f64) < count
    {
        let mut seen: HashSet<String> = pool.iter().map(|row| row.id.clone()).collect();
        let like = "(' ' || themes || ' ' LIKE ' %' || ? || ' %')";
        let mut attempt = 0;
        while attempt < 25 && (picked.len() as f64) < count {
            attempt += 1;
            let pivot = min + random() * (max - min).max(0.0);
            let mut candidates: Vec<DbPuzzle> = fetch(
                db,
                &format!(
                    "{COLUMNS} WHERE rating BETWEEN ? AND ? AND rating >= ? AND {like} LIMIT 8"
                ),
                params![min, max, pivot, theme],
            )?
            .into_iter()
            .filter(|row| !seen.contains(&row.id))
            .collect();
            if candidates.len() < 8 {
                for row in fetch(
                    db,
                    &format!(
                        "{COLUMNS} WHERE rating BETWEEN ? AND ? AND rating < ? AND {like} LIMIT 8"
                    ),
                    params![min, max, pivot, theme],
                )? {
                    if !seen.contains(&row.id) {
                        candidates.push(row);
                    }
                    if candidates.len() >= 8 {
                        break;
                    }
                }
            }
            for row in candidates {
                if (picked.len() as f64) >= count {
                    break;
                }
                if seen.contains(&row.id) || !has_theme(&row, theme) {
                    continue;
                }
                seen.insert(row.id.clone());
                picked.push(row);
            }
        }
    }
    Ok(to_puzzles(&picked))
}

/// JavaScript's `Math.round`: halves round up, also for negative numbers.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// One random puzzle near each step from `from` up to `to`: an easy-to-hard run.
pub fn query_ladder(
    db: &Connection,
    query: &LocalLadderQuery,
    random: &mut dyn FnMut() -> f64,
) -> Result<Vec<Puzzle>, String> {
    require_installed(db)?;
    let mut used: HashSet<String> = HashSet::new();
    let mut ladder: Vec<Puzzle> = Vec::new();
    let mut step = 0.0_f64;
    while step < query.count {
        let target =
            js_round(query.from + ((query.to - query.from) * step) / (query.count - 1.0).max(1.0));
        // Widen the window until something unused turns up (the sample is thin at the extremes).
        for reach in [30.0, 80.0, 200.0, 500.0] {
            let pick = pick_window(db, target - reach, target + reach, &used, random)?
                .into_iter()
                .find(|puzzle| !used.contains(&puzzle.id));
            if let Some(pick) = pick {
                used.insert(pick.id.clone());
                ladder.push(pick);
                break;
            }
        }
        step += 1.0;
    }
    Ok(ladder)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    /// A seeded stand-in for `Math.random` (a 64-bit LCG, top 53 bits).
    fn seeded(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 11) as f64 / 9_007_199_254_740_992.0
        }
    }

    fn row(id: &str, rating: f64, themes: &str) -> DbPuzzle {
        DbPuzzle {
            id: id.to_string(),
            fen: FEN.to_string(),
            moves: "e2e4 e7e5 g1f3".to_string(),
            rating,
            plays: Some(10.0),
            themes: themes.to_string(),
        }
    }

    fn setup(count: usize) -> Connection {
        let db = Connection::open_in_memory().expect("in-memory database");
        db.execute_batch(
            "CREATE TABLE puzzles (id TEXT PRIMARY KEY, fen TEXT NOT NULL, moves TEXT NOT NULL, rating INTEGER NOT NULL, plays INTEGER NOT NULL DEFAULT 0, themes TEXT NOT NULL); CREATE INDEX idx_puzzles_rating ON puzzles(rating); CREATE TABLE puzzle_meta (id INTEGER PRIMARY KEY CHECK(id = 1), importedAt INTEGER NOT NULL, count INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("test schema");
        let themes = ["mate", "fork", "pin", "mate mateIn2", "fork pin"];
        let rows: Vec<DbPuzzle> = (0..count)
            .map(|i| {
                row(
                    &format!("p{i:05}"),
                    (800 + ((i * 37) % 1600)) as f64,
                    themes[i % themes.len()],
                )
            })
            .collect();
        store_sample(&db, &rows, &|| false).expect("store sample");
        db
    }

    #[test]
    fn returns_the_requested_count_within_the_rating_range() {
        let db = setup(500);
        let mut random = seeded(1);
        let puzzles = query_puzzles(
            &db,
            &LocalPuzzleQuery {
                theme: None,
                min_rating: Some(1000.0),
                max_rating: Some(1500.0),
                count: 20.0,
            },
            &mut random,
        )
        .expect("query");
        assert_eq!(puzzles.len(), 20);
        for puzzle in &puzzles {
            assert!(puzzle.rating >= 1000.0);
            assert!(puzzle.rating <= 1500.0);
        }
        let ids: HashSet<&str> = puzzles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), 20);
    }

    #[test]
    fn matches_whole_theme_tokens_not_substrings() {
        let db = setup(500);
        let mut random = seeded(2);
        let puzzles = query_puzzles(
            &db,
            &LocalPuzzleQuery {
                theme: Some("mate".to_string()),
                min_rating: Some(0.0),
                max_rating: Some(4000.0),
                count: 30.0,
            },
            &mut random,
        )
        .expect("query");
        assert!(!puzzles.is_empty());
        for puzzle in &puzzles {
            assert!(puzzle.themes.iter().any(|theme| theme == "mate"));
        }
        // 'mateIn2' must not match a query for 'mateIn' (substring guard).
        let none = query_puzzles(
            &db,
            &LocalPuzzleQuery {
                theme: Some("mateIn".to_string()),
                min_rating: Some(0.0),
                max_rating: Some(4000.0),
                count: 10.0,
            },
            &mut random,
        )
        .expect("query");
        assert!(none.is_empty());
    }

    #[test]
    fn uses_the_rating_index_no_temp_b_tree_sort() {
        let db = setup(500);
        let mut statement = db
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? LIMIT ?",
            )
            .expect("explain");
        let plan = statement
            .query_map(params![1000.0_f64, 1500.0_f64, 5000_i64], |row| {
                row.get::<_, String>(3)
            })
            .expect("explain rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("explain detail")
            .join(" | ");
        assert!(plan.contains("idx_puzzles_rating"));
        assert!(!plan.contains("TEMP B-TREE"));
    }

    #[test]
    fn builds_a_ladder_with_increasing_difficulty_and_no_repeats() {
        let db = setup(500);
        let mut random = seeded(3);
        let ladder = query_ladder(
            &db,
            &LocalLadderQuery {
                from: 900.0,
                to: 2000.0,
                count: 6.0,
            },
            &mut random,
        )
        .expect("ladder");
        assert_eq!(ladder.len(), 6);
        let ids: HashSet<&str> = ladder.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), 6);
        let ratings: Vec<f64> = ladder.iter().map(|p| p.rating).collect();
        assert!(ratings[0] < ratings[ratings.len() - 1]);
    }

    #[test]
    fn status_reports_an_empty_database_as_not_installed() {
        let db = Connection::open_in_memory().expect("in-memory database");
        create_schema(&db).expect("schema");
        let status = read_status(&db, false).expect("status");
        assert_eq!(
            serde_json::to_value(&status).expect("json"),
            serde_json::json!({ "installed": false, "count": 0, "bytes": 0, "busy": false })
        );
    }

    #[test]
    fn query_before_install_is_refused_with_the_app_message() {
        let db = Connection::open_in_memory().expect("in-memory database");
        create_schema(&db).expect("schema");
        let mut random = seeded(4);
        let error = query_puzzles(
            &db,
            &LocalPuzzleQuery {
                theme: None,
                min_rating: None,
                max_rating: None,
                count: 1.0,
            },
            &mut random,
        )
        .expect_err("not installed");
        assert_eq!(
            error,
            "Download the puzzle database first (Settings → Data & storage, or the Rush tab)."
        );
    }

    #[test]
    fn store_sample_drops_unplayable_rows_and_counts_bytes() {
        let db = Connection::open_in_memory().expect("in-memory database");
        create_schema(&db).expect("schema");
        let mut bad_move = row("bad", 1200.0, "mate");
        bad_move.moves = "e2e5 e7e5".to_string();
        let mut no_solution = row("short", 1200.0, "mate");
        no_solution.moves = "e2e4".to_string();
        let good = row("good", 1200.0, "mate");
        let count =
            store_sample(&db, &[bad_move, no_solution, good.clone()], &|| false).expect("store");
        assert_eq!(count, 1);
        let status = read_status(&db, false).expect("status");
        assert!(status.installed);
        assert_eq!(status.count, 1);
        assert_eq!(
            status.bytes,
            (good.id.len() + good.fen.len() + good.moves.len() + good.themes.len() + 24) as i64
        );
    }

    #[test]
    fn store_sample_keeps_the_old_puzzles_when_cancelled() {
        let db = setup(50);
        let before = read_status(&db, false).expect("status").count;
        let rows: Vec<DbPuzzle> = (0..10)
            .map(|i| row(&format!("new{i}"), 1200.0, "mate"))
            .collect();
        let error = store_sample(&db, &rows, &|| true).expect_err("cancelled");
        assert_eq!(error, "Puzzle import cancelled.");
        assert_eq!(read_status(&db, false).expect("status").count, before);
    }

    #[test]
    fn clear_stored_empties_the_database() {
        let db = setup(20);
        clear_stored(&db).expect("clear");
        let status = read_status(&db, false).expect("status");
        assert!(!status.installed);
        assert_eq!(status.count, 0);
        assert_eq!(status.imported_at, None);
    }

    #[test]
    fn puzzles_serialize_like_the_typescript_objects() {
        let db = setup(5);
        let mut random = seeded(5);
        let puzzles = query_puzzles(
            &db,
            &LocalPuzzleQuery {
                theme: None,
                min_rating: None,
                max_rating: None,
                count: 1.0,
            },
            &mut random,
        )
        .expect("query");
        let json = serde_json::to_value(&puzzles[0]).expect("json");
        let object = json.as_object().expect("object");
        assert!(object.contains_key("lastMove"));
        assert!(object.contains_key("solution"));
        assert!(object["rating"].is_u64());
        assert!(!object.contains_key("gameId"));
    }
}
