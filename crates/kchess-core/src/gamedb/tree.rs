//! The opening tree: built while an import reads its games, in bounded memory, and read back one
//! position at a time.
//!
//! A row counts the games that played a move from a position (`tree`, keyed by the position's
//! Zobrist key and the packed move). An import adds each of its games' first `index_depth` plies
//! to a map; when the map holds `capacity` rows it is written out sorted to a temporary run file.
//! At the end the runs are merged in key order into the database, adding to rows already there
//! when the import adds to an existing database. Millions of games thus never need more memory
//! than one map, and SQLite receives its rows in the order its B-tree stores them.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};

use rusqlite::{Connection, params};

use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_domain::gamedb::Outcome;

/// What a row counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub white: u32,
    pub draws: u32,
    pub black: u32,
    pub elo_sum: u64,
    pub elo_n: u32,
    /// The game with the highest average rating through this move, and that rating.
    pub best_game: i64,
    pub best_elo: u16,
    /// The latest game through this move, and its date (`YYYYMMDD`).
    pub last_game: i64,
    pub last_date: u32,
}

impl Stats {
    pub fn of(game: i64, outcome: Outcome, rating: Option<u16>, date: u32) -> Stats {
        let mut stats = Stats {
            best_game: game,
            best_elo: rating.unwrap_or(0),
            last_game: game,
            last_date: date,
            ..Stats::default()
        };
        match outcome {
            Outcome::White => stats.white = 1,
            Outcome::Black => stats.black = 1,
            Outcome::Draw => stats.draws = 1,
            Outcome::Unknown => {}
        }
        if let Some(rating) = rating {
            stats.elo_sum = u64::from(rating);
            stats.elo_n = 1;
        }
        stats
    }

    pub fn add(&mut self, other: &Stats) {
        self.white += other.white;
        self.draws += other.draws;
        self.black += other.black;
        self.elo_sum += other.elo_sum;
        self.elo_n += other.elo_n;
        if other.best_elo > self.best_elo {
            self.best_elo = other.best_elo;
            self.best_game = other.best_game;
        }
        if (other.last_date, other.last_game) > (self.last_date, self.last_game) {
            self.last_date = other.last_date;
            self.last_game = other.last_game;
        }
    }

    pub fn games(&self) -> u64 {
        u64::from(self.white) + u64::from(self.draws) + u64::from(self.black)
    }
}

/// A row as a run file stores it: little-endian, fixed size.
const RECORD: usize = 8 + 2 + 4 * 3 + 8 + 4 + 8 + 2 + 8 + 4;

fn write_record(out: &mut impl Write, key: u64, mv: u16, s: &Stats) -> std::io::Result<()> {
    let mut bytes = [0u8; RECORD];
    let mut at = 0;
    let mut put = |part: &[u8]| {
        if let Some(slot) = bytes.get_mut(at..at + part.len()) {
            slot.copy_from_slice(part);
        }
        at += part.len();
    };
    put(&key.to_le_bytes());
    put(&mv.to_le_bytes());
    put(&s.white.to_le_bytes());
    put(&s.draws.to_le_bytes());
    put(&s.black.to_le_bytes());
    put(&s.elo_sum.to_le_bytes());
    put(&s.elo_n.to_le_bytes());
    put(&s.best_game.to_le_bytes());
    put(&s.best_elo.to_le_bytes());
    put(&s.last_game.to_le_bytes());
    put(&s.last_date.to_le_bytes());
    out.write_all(&bytes)
}

fn read_record(input: &mut impl Read) -> std::io::Result<Option<(u64, u16, Stats)>> {
    let mut bytes = [0u8; RECORD];
    match input.read_exact(&mut bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut at = 0;
    let mut take = |n: usize| {
        let part = bytes.get(at..at + n).unwrap_or_default();
        at += n;
        part
    };
    let u16_of = |b: &[u8]| u16::from_le_bytes(b.try_into().unwrap_or_default());
    let u32_of = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap_or_default());
    let u64_of = |b: &[u8]| u64::from_le_bytes(b.try_into().unwrap_or_default());
    let key = u64_of(take(8));
    let mv = u16_of(take(2));
    let stats = Stats {
        white: u32_of(take(4)),
        draws: u32_of(take(4)),
        black: u32_of(take(4)),
        elo_sum: u64_of(take(8)),
        elo_n: u32_of(take(4)),
        best_game: u64_of(take(8)) as i64,
        best_elo: u16_of(take(2)),
        last_game: u64_of(take(8)) as i64,
        last_date: u32_of(take(4)),
    };
    Ok(Some((key, mv, stats)))
}

/// The tree an import adds, gathered in memory up to `capacity` rows and in run files beyond.
pub struct TreeBuilder {
    rows: HashMap<(u64, u16), Stats>,
    capacity: usize,
    runs: Vec<File>,
    temp: tempfile::TempDir,
}

fn io(error: std::io::Error) -> CoreError {
    CoreError::wrap("Could not write the opening tree", error)
}

impl TreeBuilder {
    /// Run files go in a private directory inside `parent` (the profile's databases folder, on
    /// the same disk as the database), removed with the builder.
    pub fn new(parent: &std::path::Path, capacity: usize) -> Result<TreeBuilder> {
        Ok(TreeBuilder {
            rows: HashMap::with_capacity(capacity.min(1 << 20)),
            capacity: capacity.max(1),
            runs: Vec::new(),
            temp: tempfile::Builder::new()
                .prefix(".tree-")
                .tempdir_in(parent)
                .map_err(io)?,
        })
    }

    pub fn add(&mut self, key: u64, mv: u16, stats: &Stats) -> Result<()> {
        self.rows.entry((key, mv)).or_default().add(stats);
        if self.rows.len() >= self.capacity {
            self.spill()?;
        }
        Ok(())
    }

    fn sorted(&mut self) -> Vec<((u64, u16), Stats)> {
        let mut rows: Vec<_> = self.rows.drain().collect();
        rows.sort_unstable_by_key(|(at, _)| *at);
        rows
    }

    /// The rows in memory written out as a sorted run.
    fn spill(&mut self) -> Result<()> {
        let rows = self.sorted();
        let path = self.temp.path().join(format!("run-{}", self.runs.len()));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io)?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        for ((key, mv), stats) in &rows {
            write_record(&mut out, *key, *mv, stats).map_err(io)?;
        }
        let mut file = out.into_inner().map_err(|e| io(e.into_error()))?;
        std::io::Seek::rewind(&mut file).map_err(io)?;
        self.runs.push(file);
        Ok(())
    }

    /// Every row in key order, those of several runs added together, to `each`.
    fn merged(mut self, mut each: impl FnMut(u64, u16, &Stats) -> Result<()>) -> Result<()> {
        if self.runs.is_empty() {
            for ((key, mv), stats) in self.sorted() {
                each(key, mv, &stats)?;
            }
            return Ok(());
        }
        if !self.rows.is_empty() {
            self.spill()?;
        }
        let mut readers: Vec<_> = self
            .runs
            .drain(..)
            .map(|file| BufReader::with_capacity(1 << 18, file))
            .collect();
        let mut heap = BinaryHeap::new();
        for (index, reader) in readers.iter_mut().enumerate() {
            if let Some((key, mv, stats)) = read_record(reader).map_err(io)? {
                heap.push(Reverse(((key, mv), index, StatsOrd(stats))));
            }
        }
        let mut current: Option<((u64, u16), Stats)> = None;
        while let Some(Reverse((at, index, StatsOrd(stats)))) = heap.pop() {
            if let Some(reader) = readers.get_mut(index)
                && let Some((key, mv, next)) = read_record(reader).map_err(io)?
            {
                heap.push(Reverse(((key, mv), index, StatsOrd(next))));
            }
            match &mut current {
                Some((open, sum)) if *open == at => sum.add(&stats),
                _ => {
                    if let Some(((key, mv), sum)) = current.replace((at, stats)) {
                        each(key, mv, &sum)?;
                    }
                }
            }
        }
        if let Some(((key, mv), sum)) = current {
            each(key, mv, &sum)?;
        }
        Ok(())
    }

    /// Writes the tree into `db`: inserted into an empty table, or added to the rows an earlier
    /// import left. The caller holds the transaction, if any.
    pub fn write(self, db: &Connection, adding: bool, cancelled: &dyn Fn() -> bool) -> Result<u64> {
        let sql = if adding {
            "INSERT INTO tree VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT (key, mv) DO UPDATE SET
               white = white + excluded.white, draws = draws + excluded.draws,
               black = black + excluded.black, elo_sum = elo_sum + excluded.elo_sum,
               elo_n = elo_n + excluded.elo_n,
               best_game = CASE WHEN excluded.best_elo > best_elo
                 THEN excluded.best_game ELSE best_game END,
               best_elo = MAX(best_elo, excluded.best_elo),
               last_game = CASE WHEN excluded.last_date >= last_date
                 THEN excluded.last_game ELSE last_game END,
               last_date = MAX(last_date, excluded.last_date)"
        } else {
            "INSERT INTO tree VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        };
        let mut insert = db.prepare_cached(sql).sql()?;
        let mut written = 0u64;
        self.merged(|key, mv, s| {
            if written.is_multiple_of(65_536) && cancelled() {
                return Err(CoreError::new("Import cancelled."));
            }
            insert
                .execute(params![
                    key as i64,
                    mv,
                    s.white,
                    s.draws,
                    s.black,
                    s.elo_sum as i64,
                    s.elo_n,
                    s.best_game,
                    s.best_elo,
                    s.last_game,
                    s.last_date
                ])
                .sql()?;
            written += 1;
            Ok(())
        })?;
        Ok(written)
    }
}

/// Orders the heap by position alone; equal positions from different runs are added together.
#[derive(Clone, Copy, Debug)]
struct StatsOrd(Stats);

impl PartialEq for StatsOrd {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for StatsOrd {}
impl PartialOrd for StatsOrd {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for StatsOrd {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

/// One move from a position, as the tree counts it.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeMove {
    pub mv: u16,
    pub stats: Stats,
}

/// The moves played from the position `key`, most played first.
pub fn moves(db: &Connection, key: u64) -> Result<Vec<TreeMove>> {
    let mut query = db
        .prepare_cached(
            "SELECT mv, white, draws, black, elo_sum, elo_n, best_game, best_elo, last_game,
                    last_date
             FROM tree WHERE key = ?1",
        )
        .sql()?;
    let mut moves = query
        .query_map([key as i64], |row| {
            Ok(TreeMove {
                mv: row.get(0)?,
                stats: Stats {
                    white: row.get(1)?,
                    draws: row.get(2)?,
                    black: row.get(3)?,
                    elo_sum: row.get::<_, i64>(4)? as u64,
                    elo_n: row.get(5)?,
                    best_game: row.get(6)?,
                    best_elo: row.get(7)?,
                    last_game: row.get(8)?,
                    last_date: row.get(9)?,
                },
            })
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    moves.sort_by_key(|m| (Reverse(m.stats.games()), m.mv));
    Ok(moves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamedb::schema;
    use kchess_contracts::types::DatabaseSource;
    use kchess_testkit::random;

    fn database(dir: &std::path::Path) -> Connection {
        schema::create(&dir.join("t.kdb"), "T", DatabaseSource::Imported, 0).unwrap()
    }

    fn all(db: &Connection) -> HashMap<(u64, u16), Stats> {
        let mut rows = HashMap::new();
        let mut query = db.prepare("SELECT DISTINCT key FROM tree").unwrap();
        let keys: Vec<i64> = query
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|k| k.unwrap())
            .collect();
        for key in keys {
            for m in moves(db, key as u64).unwrap() {
                rows.insert((key as u64, m.mv), m.stats);
            }
        }
        rows
    }

    /// The tree is the same however small the map, so however many runs it is merged from; and
    /// adding a second import to rows already written sums them.
    #[test]
    fn runs_merge_to_what_one_map_holds() {
        random::check(20, |rng| {
            let dir = tempfile::tempdir().unwrap();
            let entries: Vec<(u64, u16, Stats)> = (0..400)
                .map(|game| {
                    let outcome = *rng
                        .pick(&[
                            Outcome::White,
                            Outcome::Draw,
                            Outcome::Black,
                            Outcome::Unknown,
                        ])
                        .unwrap();
                    let rating = rng.chance(70).then(|| 1500 + rng.below(1300) as u16);
                    let stats = Stats::of(game, outcome, rating, 20000101 + rng.below(9000) as u32);
                    (rng.below(40) as u64, rng.below(5) as u16, stats)
                })
                .collect();
            let mut expected: HashMap<(u64, u16), Stats> = HashMap::new();
            for (key, mv, stats) in &entries {
                expected.entry((*key, *mv)).or_default().add(stats);
            }
            let db = database(dir.path());
            let (first, second) = entries.split_at(150);
            let mut builder = TreeBuilder::new(dir.path(), 1 + rng.below(30)).unwrap();
            for (key, mv, stats) in first {
                builder.add(*key, *mv, stats).unwrap();
            }
            builder.write(&db, false, &|| false).unwrap();
            let mut builder = TreeBuilder::new(dir.path(), 1 + rng.below(30)).unwrap();
            for (key, mv, stats) in second {
                builder.add(*key, *mv, stats).unwrap();
            }
            builder.write(&db, true, &|| false).unwrap();
            let rows = all(&db);
            assert_eq!(rows.len(), expected.len());
            for (at, stats) in &expected {
                let row = rows[at];
                assert_eq!(
                    (row.white, row.draws, row.black, row.elo_sum, row.elo_n),
                    (
                        stats.white,
                        stats.draws,
                        stats.black,
                        stats.elo_sum,
                        stats.elo_n
                    )
                );
                assert_eq!(row.best_elo, stats.best_elo);
                assert_eq!(row.last_date, stats.last_date);
            }
        });
    }

    #[test]
    fn run_files_are_removed_with_the_builder() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = TreeBuilder::new(dir.path(), 2).unwrap();
        for key in 0..10 {
            builder
                .add(key, 0, &Stats::of(1, Outcome::Draw, None, 0))
                .unwrap();
        }
        let db = database(dir.path());
        assert_eq!(builder.write(&db, false, &|| false).unwrap(), 10);
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tree-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }
}
