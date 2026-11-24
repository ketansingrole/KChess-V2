//! Positions past the opening tree. The tree counts each game's first `index_depth` plies; past
//! them a position is found through its structure (`gamedb::structure_key`: where the pawns stand
//! and how many pieces of each kind there are), which only a capture or a pawn move changes. A
//! game passes through a few dozen structures, not a hundred positions, so `deep` keeps, for each
//! structure a finished game has from ply `index_depth` on, that game; a lookup replays only the
//! games that passed through its position's structure, a handful where the tree would need every
//! position of every game.
//!
//! `deep` is one row a structure bucket (the key's high bits), holding its entries (the key's low
//! bits and the game) sorted: six bytes an entry, a lookup one row. An import gathers its entries
//! in memory up to a bound and in sorted run files beyond, and merges them into the buckets at
//! its end, as the tree does.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};

use rusqlite::{Connection, OptionalExtension as _, params};
use shakmaty::variant::VariantPosition;

use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_domain::gamedb;

/// A key's bits that pick its bucket (the high ones); the rest are stored with each entry.
const LOW_BITS: u32 = 16;
/// An entry in a bucket: the key's low 16 bits and the game, little-endian.
const ENTRY: usize = 2 + 4;
/// Entries an import holds in memory before it writes them out sorted: 64 MB.
pub const CAPACITY: usize = 8 << 20;
/// Entries building the index of an older file holds in memory: 128 MB.
const BUILD_CAPACITY: usize = 16 << 20;

fn io(error: std::io::Error) -> CoreError {
    CoreError::wrap("Could not write the position index", error)
}

/// An entry as the builder sorts it: the key above the game, so that entries sort by bucket,
/// then by the key's low bits, then by game.
fn entry(key: u32, game: u32) -> u64 {
    u64::from(key) << 32 | u64::from(game)
}

/// The structures a game passes through from ply `depth` on, each once, to `each`; the game's
/// start position and its encoded moves. False when the moves are damaged.
pub fn structures(
    start: &VariantPosition,
    code: &[u8],
    depth: usize,
    mut each: impl FnMut(u32),
) -> bool {
    let mut last = None;
    let mut ply = 0;
    gamedb::walk_line(start, code, |pos, _| {
        if ply >= depth {
            let key = gamedb::structure_key(pos);
            // Captures and pawn moves are for good, so a game never comes back to a structure
            // it left: each is a run of plies, and only the change to the next is new.
            if last != Some(key) {
                last = Some(key);
                each(key);
            }
        }
        ply += 1;
        true
    })
}

/// The entries an import adds, gathered in memory up to `capacity` and in run files beyond.
pub struct DeepBuilder {
    entries: Vec<u64>,
    capacity: usize,
    runs: Vec<File>,
    temp: tempfile::TempDir,
}

impl DeepBuilder {
    /// Run files go in a private directory inside `parent`, removed with the builder.
    pub fn new(parent: &std::path::Path, capacity: usize) -> Result<DeepBuilder> {
        Ok(DeepBuilder {
            entries: Vec::new(),
            capacity: capacity.max(1),
            runs: Vec::new(),
            temp: tempfile::Builder::new()
                .prefix(".deep-")
                .tempdir_in(parent)
                .map_err(io)?,
        })
    }

    /// The finished game `game` passes through structure `key` past the tree. A game whose id
    /// does not fit 32 bits (four billion games in) is left out of the index.
    pub fn add(&mut self, key: u32, game: i64) -> Result<()> {
        let Ok(game) = u32::try_from(game) else {
            return Ok(());
        };
        self.entries.push(entry(key, game));
        if self.entries.len() >= self.capacity {
            self.spill()?;
        }
        Ok(())
    }

    /// Every structure of a finished game from ply `depth` on.
    pub fn add_game(
        &mut self,
        game: i64,
        start: &VariantPosition,
        code: &[u8],
        depth: usize,
    ) -> Result<()> {
        let mut keys = Vec::new();
        structures(start, code, depth, |key| keys.push(key));
        for key in keys {
            self.add(key, game)?;
        }
        Ok(())
    }

    fn spill(&mut self) -> Result<()> {
        self.entries.sort_unstable();
        let path = self.temp.path().join(format!("run-{}", self.runs.len()));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io)?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        for value in &self.entries {
            out.write_all(&value.to_le_bytes()).map_err(io)?;
        }
        let mut file = out.into_inner().map_err(|e| io(e.into_error()))?;
        std::io::Seek::rewind(&mut file).map_err(io)?;
        self.runs.push(file);
        self.entries.clear();
        Ok(())
    }

    /// Every entry in order, to `each`.
    fn merged(mut self, mut each: impl FnMut(u64) -> Result<()>) -> Result<()> {
        if self.runs.is_empty() {
            self.entries.sort_unstable();
            for &value in &self.entries {
                each(value)?;
            }
            return Ok(());
        }
        if !self.entries.is_empty() {
            self.spill()?;
        }
        self.entries = Vec::new();
        let mut readers: Vec<_> = self
            .runs
            .drain(..)
            .map(|file| BufReader::with_capacity(1 << 16, file))
            .collect();
        let next = |reader: &mut BufReader<File>| -> Result<Option<u64>> {
            let mut bytes = [0u8; 8];
            match reader.read_exact(&mut bytes) {
                Ok(()) => Ok(Some(u64::from_le_bytes(bytes))),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
                Err(error) => Err(io(error)),
            }
        };
        let mut heap = BinaryHeap::new();
        for (index, reader) in readers.iter_mut().enumerate() {
            if let Some(value) = next(reader)? {
                heap.push(Reverse((value, index)));
            }
        }
        while let Some(Reverse((value, index))) = heap.pop() {
            if let Some(reader) = readers.get_mut(index)
                && let Some(following) = next(reader)?
            {
                heap.push(Reverse((following, index)));
            }
            each(value)?;
        }
        Ok(())
    }

    /// Writes the entries into `db`'s buckets, merged with those already there. The caller holds
    /// the transaction, if any.
    pub fn write(self, db: &Connection, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let mut bucket: Option<u32> = None;
        let mut added: Vec<u8> = Vec::new();
        let mut written = 0u64;
        self.merged(|value| {
            if written.is_multiple_of(1 << 20) && cancelled() {
                return Err(CoreError::new("Import cancelled."));
            }
            written += 1;
            let key = (value >> 32) as u32;
            let at = key >> LOW_BITS;
            if bucket != Some(at) {
                if let Some(done) = bucket {
                    merge_into(db, done, &added)?;
                }
                bucket = Some(at);
                added.clear();
            }
            added.extend_from_slice(&(key as u16).to_le_bytes());
            added.extend_from_slice(&(value as u32).to_le_bytes());
            Ok(())
        })?;
        if let Some(done) = bucket {
            merge_into(db, done, &added)?;
        }
        Ok(())
    }
}

/// An entry of a bucket's blob, as its order: the key's low bits, then the game.
fn read_entry(bytes: &[u8; ENTRY]) -> (u16, u32) {
    let low = u16::from_le_bytes([bytes[0], bytes[1]]);
    let game = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
    (low, game)
}

/// Bucket `bucket`'s entries with `added` (sorted entries) merged in.
fn merge_into(db: &Connection, bucket: u32, added: &[u8]) -> Result<()> {
    let earlier: Option<Vec<u8>> = db
        .prepare_cached("SELECT entries FROM deep WHERE bucket = ?1")
        .sql()?
        .query_row([bucket], |row| row.get(0))
        .optional()
        .sql()?;
    let merged = match earlier {
        None => added.to_vec(),
        Some(earlier) => {
            let mut merged = Vec::with_capacity(earlier.len() + added.len());
            let (mut a, mut b) = (
                earlier.as_chunks::<ENTRY>().0.iter(),
                added.as_chunks::<ENTRY>().0.iter(),
            );
            let (mut x, mut y) = (a.next(), b.next());
            loop {
                match (x, y) {
                    (Some(left), Some(right)) => {
                        if read_entry(left) <= read_entry(right) {
                            merged.extend_from_slice(left);
                            x = a.next();
                        } else {
                            merged.extend_from_slice(right);
                            y = b.next();
                        }
                    }
                    (Some(left), None) => {
                        merged.extend_from_slice(left);
                        x = a.next();
                    }
                    (None, Some(right)) => {
                        merged.extend_from_slice(right);
                        y = b.next();
                    }
                    (None, None) => break,
                }
            }
            merged
        }
    };
    db.prepare_cached("INSERT OR REPLACE INTO deep (bucket, entries) VALUES (?1, ?2)")
        .sql()?
        .execute(params![bucket, merged])
        .sql()?;
    Ok(())
}

/// The games that pass through structure `key` past the tree, in id order.
pub fn games(db: &Connection, key: u32) -> Result<Vec<i64>> {
    let blob: Option<Vec<u8>> = db
        .prepare_cached("SELECT entries FROM deep WHERE bucket = ?1")
        .sql()?
        .query_row([key >> LOW_BITS], |row| row.get(0))
        .optional()
        .sql()?;
    let Some(blob) = blob else {
        return Ok(Vec::new());
    };
    let low = key as u16;
    let entries = blob.as_chunks::<ENTRY>().0;
    let first = entries.partition_point(|bytes| read_entry(bytes).0 < low);
    Ok(entries[first..]
        .iter()
        .map(read_entry)
        .take_while(|(at, _)| *at == low)
        .map(|(_, game)| i64::from(game))
        .collect())
}

/// Whether `db`'s index holds every game (`meta.deep_built`); a file an older release wrote
/// has none until it is built (`ensure`), and a lookup then replays every game past the tree.
pub fn built(db: &Connection) -> Result<bool> {
    let has_column: bool = db
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('meta') WHERE name = 'deep_built')",
            [],
            |row| row.get(0),
        )
        .sql()?;
    if !has_column {
        return Ok(false);
    }
    db.query_row("SELECT deep_built FROM meta WHERE id = 1", [], |row| {
        row.get::<_, Option<bool>>(0)
    })
    .optional()
    .sql()
    .map(|built| built.flatten().unwrap_or(false))
}

/// Builds the index of a file an older release wrote (`path`, open to write as `db`), from
/// every finished game, on every core: a few seconds a million games, once.
pub fn ensure(db: &Connection, path: &std::path::Path) -> Result<()> {
    if built(db)? {
        return Ok(());
    }
    let depth: i64 = db
        .query_row("SELECT index_depth FROM meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()
        .sql()?
        .unwrap_or(0);
    let depth = usize::try_from(depth).unwrap_or(0);
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut builder = DeepBuilder::new(parent, BUILD_CAPACITY)?;
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    type Read = Vec<(i64, Option<String>, Vec<u8>)>;
    let (send, games) = std::sync::mpsc::sync_channel::<Read>(workers * 2);
    let games = std::sync::Mutex::new(games);
    let (answer, answers) = std::sync::mpsc::channel::<Vec<(u32, i64)>>();
    std::thread::scope(|scope| -> Result<()> {
        let (send, answer) = (send, answer);
        for _ in 0..workers {
            let (games, answer) = (&games, answer.clone());
            scope.spawn(move || {
                loop {
                    let next = crate::tasks::lock(games).recv();
                    let Ok(read) = next else {
                        break;
                    };
                    let mut found = Vec::new();
                    for (id, fen, code) in read {
                        if let Some(start) = gamedb::start_of(fen.as_deref()) {
                            structures(&start, &code, depth, |key| found.push((key, id)));
                        }
                    }
                    if answer.send(found).is_err() {
                        break;
                    }
                }
            });
        }
        drop(answer);
        let mut select = db
            .prepare("SELECT id, fen, moves FROM games WHERE result IS NOT NULL")
            .sql()?;
        let mut rows = select.query([]).sql()?;
        let mut read: Read = Vec::with_capacity(1024);
        let take = |builder: &mut DeepBuilder| -> Result<()> {
            for found in answers.try_iter() {
                for (key, id) in found {
                    builder.add(key, id)?;
                }
            }
            Ok(())
        };
        while let Some(row) = rows.next().sql()? {
            read.push((row.get(0).sql()?, row.get(1).sql()?, row.get(2).sql()?));
            if read.len() == 1024 {
                if send.send(std::mem::take(&mut read)).is_err() {
                    break;
                }
                take(&mut builder)?;
            }
        }
        if !read.is_empty() {
            send.send(read).ok();
        }
        drop(send);
        for found in answers.iter() {
            for (key, id) in found {
                builder.add(key, id)?;
            }
        }
        Ok(())
    })?;
    crate::store::sql::transaction(db, || {
        builder.write(db, &|| false)?;
        db.execute("UPDATE meta SET deep_built = 1 WHERE id = 1", [])
            .sql()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_contracts::types::DatabaseSource;
    use kchess_testkit::random;

    fn buckets(db: &Connection) -> Vec<(i64, Vec<u8>)> {
        let mut select = db
            .prepare("SELECT bucket, entries FROM deep ORDER BY bucket")
            .unwrap();
        select
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    /// The buckets are the same however small the memory, so however many runs they are merged
    /// from, and whether the entries come in one import or are added in two; and every game is
    /// found by its key.
    #[test]
    fn runs_and_additions_merge_to_what_one_builder_holds() {
        random::check(4, |rng| {
            let dir = tempfile::tempdir().unwrap();
            // Few distinct high bits, so that buckets hold many entries.
            let entries: Vec<(u32, i64)> = (0..2_000)
                .map(|_| {
                    let key = (rng.below(4) as u32) << LOW_BITS | rng.below(50) as u32;
                    (key, 1 + rng.below(500) as i64)
                })
                .collect();
            let written = |parts: &[&[(u32, i64)]], capacity: usize, name: &str| {
                let db = super::super::schema::create(
                    &dir.path().join(name),
                    "D",
                    DatabaseSource::Imported,
                    0,
                )
                .unwrap();
                for part in parts {
                    let mut builder = DeepBuilder::new(dir.path(), capacity).unwrap();
                    for &(key, game) in *part {
                        builder.add(key, game).unwrap();
                    }
                    builder.write(&db, &|| false).unwrap();
                }
                db
            };
            let split = rng.below(entries.len());
            let one = written(&[&entries], 1 << 20, "one.kdb");
            let runs = written(&[&entries], 1 + rng.below(40), "runs.kdb");
            let added = written(
                &[&entries[..split], &entries[split..]],
                1 + rng.below(40),
                "added.kdb",
            );
            assert_eq!(buckets(&runs), buckets(&one));
            assert_eq!(buckets(&added), buckets(&one));
            for &(key, game) in &entries {
                let found = games(&one, key).unwrap();
                assert!(found.contains(&game), "{key} {game}");
                assert!(found.windows(2).all(|pair| pair[0] <= pair[1]));
            }
            assert!(games(&one, 99 << LOW_BITS).unwrap().is_empty());
        });
    }
}
