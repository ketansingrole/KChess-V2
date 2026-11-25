//! Reading a PGN file into a database, on one blocking thread.
//!
//! A new database is written to a `.partial` file with no journal and renamed into place once it
//! is complete, so a failure or a cancellation leaves no database behind. Games added to an
//! existing database are written in batches under their import's id, invisible to searches until
//! the import finishes (`search::visible`), and the tree rows are added in the final transaction;
//! a failure removes the import's games and leaves the tree as it was. A database rebuilt in
//! place (`Target::Replace`) loses its earlier games in that same final transaction, so a search
//! sees the old games or the new ones, never a mix.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, OptionalExtension as _, params};

use super::deep::{self, DeepBuilder};
use super::schema;
use super::tree::{Stats, TreeBuilder};
use crate::store::sql::{Sql as _, transaction};
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::DatabaseSource;
use kchess_domain::gamedb::{self, Outcome, Stored};
use kchess_domain::pgn::{PgnItem, PgnReader};

/// Games written to the file in one transaction.
const BATCH: u64 = 5_000;

/// Where an import writes.
pub enum Target {
    /// A new database, written to `partial` and renamed to `file` when it is complete.
    New {
        file: PathBuf,
        partial: PathBuf,
        name: String,
        source: DatabaseSource,
        /// Whose games they are and on what terms, when the source asks for it to be shown.
        attribution: Option<String>,
    },
    /// An existing database's file, added to.
    Existing { file: PathBuf },
    /// An existing database's file whose games and tree become the import's.
    Replace { file: PathBuf },
}

/// What an import reads: a PGN text, held as `packing` says, of `total_bytes` bytes, recorded
/// in the database as `label` (a file's name). With `keep`, only the games whose header lines it
/// keeps are read at all (`HeaderFilter`).
pub struct Input<R> {
    pub reader: R,
    pub total_bytes: u64,
    pub packing: PgnPacking,
    pub label: Option<String>,
    pub keep: Option<Keep>,
    /// Which files of a `.zip` or `.7z` are read, by name: every PGN file (`archived_pgn`) when
    /// absent.
    pub entry: Option<Entry>,
}

/// Which files of an archive an import reads, by their names in it.
pub type Entry = fn(&str) -> bool;

/// Which games of a text an import reads, by their header lines (`[White "…"]` and the rest,
/// each with its line ending), before their moves are read.
pub type Keep = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// The most header text one game may have: past it, the game is left out.
const MAX_HEADERS: usize = 64 * 1024;

/// A PGN text with only the games `keep` keeps, by their header lines: the others' lines are
/// passed over unread, so a huge text of mostly unwanted games costs a scan of its lines, not a
/// parse of every game. A game starts at a header line (`[`) after moves; text before the first
/// game is left out.
pub struct HeaderFilter<R> {
    inner: R,
    keep: Keep,
    line: Vec<u8>,
    headers: Vec<u8>,
    /// Text kept, waiting to be read.
    out: Vec<u8>,
    at: usize,
    state: Filtering,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filtering {
    /// Reading a game's header lines.
    Headers,
    /// Reading a game's moves: kept or passed over.
    Moves { kept: bool },
}

impl<R: io::BufRead> HeaderFilter<R> {
    pub fn new(inner: R, keep: Keep) -> HeaderFilter<R> {
        HeaderFilter {
            inner,
            keep,
            line: Vec::new(),
            headers: Vec::new(),
            out: Vec::new(),
            at: 0,
            state: Filtering::Moves { kept: false },
        }
    }

    /// Whether the game whose header lines are held is kept: given to `keep`, unless they ran
    /// past `MAX_HEADERS`.
    fn decide(&mut self) -> bool {
        let kept = self.headers.len() <= MAX_HEADERS && (self.keep)(&self.headers);
        if kept {
            self.out.extend_from_slice(&self.headers);
        }
        self.headers.clear();
        kept
    }

    /// Reads lines until some text is kept, or the text ends.
    fn fill(&mut self) -> io::Result<()> {
        self.out.clear();
        self.at = 0;
        while self.out.len() < 64 * 1024 {
            self.line.clear();
            if self.inner.read_until(b'\n', &mut self.line)? == 0 {
                // A last game with no moves.
                if self.state == Filtering::Headers {
                    self.decide();
                    self.state = Filtering::Moves { kept: false };
                }
                break;
            }
            let header = self.line.first() == Some(&b'[');
            match self.state {
                Filtering::Headers if header => {
                    if self.headers.len() <= MAX_HEADERS {
                        self.headers.extend_from_slice(&self.line);
                    }
                }
                Filtering::Headers => {
                    let kept = self.decide();
                    if kept {
                        self.out.extend_from_slice(&self.line);
                    }
                    self.state = Filtering::Moves { kept };
                }
                Filtering::Moves { .. } if header => {
                    self.state = Filtering::Headers;
                    self.headers.extend_from_slice(&self.line);
                }
                Filtering::Moves { kept } => {
                    if kept {
                        self.out.extend_from_slice(&self.line);
                    }
                }
            }
        }
        Ok(())
    }
}

impl<R: io::BufRead> Read for HeaderFilter<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at == self.out.len() {
            self.fill()?;
        }
        let n = buf.len().min(self.out.len() - self.at);
        buf[..n].copy_from_slice(&self.out[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// The text `input` reads, through `keep` when given.
fn kept(input: impl io::BufRead + 'static, keep: Option<Keep>) -> Box<dyn io::BufRead> {
    match keep {
        Some(keep) => Box::new(BufReader::with_capacity(
            1 << 20,
            HeaderFilter::new(input, keep),
        )),
        None => Box::new(input),
    }
}

/// How far an import has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub games: u64,
    pub skipped: u64,
    /// Bytes of the file read (compressed bytes for a `.zst`, `.zip` or `.7z`).
    pub bytes: u64,
    pub total_bytes: u64,
}

/// What an import is told along the way.
pub trait Watch {
    /// Games read so far; called often, so it should be cheap.
    fn reading(&mut self, count: Count);
    /// The games are in: the tree and indexes are being written.
    fn indexing(&mut self, count: Count);
    fn cancelled(&self) -> bool;
}

/// A file's bytes, counted as they are read: how far into the file reading has come, which a
/// zip's reader moves by seeking (to its directory at the end, then to each file in turn).
struct Counted<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(out)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl<R: Seek> Seek for Counted<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = self.inner.seek(to)?;
        self.read.store(at, Ordering::Relaxed);
        Ok(at)
    }
}

/// Whether an archive's entry named `name` is a PGN file to read: a `.pgn` that is not macOS's
/// record of another file's attributes (`__MACOSX/`, `._name.pgn`).
pub fn archived_pgn(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    name.to_ascii_lowercase().ends_with(".pgn")
        && !name.starts_with("__MACOSX/")
        && !base.starts_with("._")
}

/// Every PGN file a zip archive holds, one after another, to `sink`, in the archive's order. A
/// blank line ends each, so the last game of one never runs into the first of the next.
fn unzip_pgn(
    reader: impl Read + Seek,
    wanted: Entry,
    mut sink: impl FnMut(&[u8]) -> io::Result<()>,
) -> Result<()> {
    let unreadable = |cause: zip::result::ZipError| {
        if let zip::result::ZipError::UnsupportedArchive(why) = cause
            && why == zip::result::ZipError::PASSWORD_REQUIRED
        {
            return CoreError::new("That .zip is password-protected.");
        }
        CoreError::wrap("That .zip could not be read", cause)
    };
    let stopped = |cause: io::Error| CoreError::wrap("That .zip could not be read", cause);
    let mut archive = zip::ZipArchive::new(reader).map_err(unreadable)?;
    let mut found = false;
    let mut chunk = vec![0u8; 64 * 1024];
    for index in 0..archive.len() {
        let mut entry = {
            let about = archive.by_index_data(index).map_err(unreadable)?;
            let name = about.name().map_err(unreadable)?;
            if about.is_dir() || !wanted(&name) {
                continue;
            }
            archive.by_index(index).map_err(unreadable)?
        };
        found = true;
        loop {
            let n = entry.read(&mut chunk).map_err(stopped)?;
            if n == 0 {
                break;
            }
            sink(&chunk[..n]).map_err(stopped)?;
        }
        sink(b"\n\n").map_err(stopped)?;
    }
    if !found {
        return Err(CoreError::new("That .zip holds no .pgn file."));
    }
    Ok(())
}

/// The most memory a `.7z`'s decoder may take, in KiB: room for 7-Zip's largest dictionary
/// (1.5 GiB), so an archive asking for more is refused rather than taking the computer's memory.
const SEVEN_ZIP_MEMORY_KB: usize = 2 * 1024 * 1024;

/// Every PGN file a 7-Zip archive holds, one after another, to `sink`, in the archive's order,
/// each ended by a blank line as a zip's are. A solid archive decodes as one stream, so the other
/// files in it are decoded and checked too, but never kept. It decodes on one thread: the import
/// reading the games is the slower side.
fn un7z_pgn(
    reader: impl Read + Seek,
    wanted_name: Entry,
    mut sink: impl FnMut(&[u8]) -> io::Result<()>,
) -> Result<()> {
    use sevenz_rust2::{ArchiveReader, Error, Password};
    let locked = || CoreError::new("That .7z is password-protected.");
    let unreadable = |cause: Error| match cause {
        Error::PasswordRequired | Error::MaybeBadPassword(_) => locked(),
        // Encryption is not built in: an encrypted archive names AES as its method.
        Error::UnsupportedCompressionMethod(method) if method.contains("AES") => locked(),
        Error::UnsupportedCompressionMethod(_) => {
            CoreError::new("That .7z uses a compression KChess can't read.")
        }
        Error::MaxMemLimited { .. } => {
            CoreError::new("That .7z needs more memory than KChess allows.")
        }
        cause => CoreError::wrap("That .7z could not be read", cause),
    };
    let mut archive =
        ArchiveReader::new_with_memory_limit(reader, Password::empty(), SEVEN_ZIP_MEMORY_KB)
            .map_err(unreadable)?;
    archive.set_thread_count(1);
    let wanted = |entry: &sevenz_rust2::ArchiveEntry| {
        entry.has_stream()
            && !entry.is_directory()
            && !entry.is_anti_item
            && wanted_name(entry.name())
    };
    let about = archive.archive();
    let pgn: Vec<usize> = (about.files.iter().enumerate())
        .filter(|(_, entry)| wanted(entry))
        .map(|(index, _)| index)
        .collect();
    if pgn.is_empty() {
        return Err(CoreError::new("That .7z holds no .pgn file."));
    }
    if pgn.iter().any(|&index| about.is_file_encrypted(index)) {
        return Err(locked());
    }
    let mut chunk = vec![0u8; 64 * 1024];
    archive
        .for_each_entries(|entry, read| {
            // Another file's bytes are left unread: the reader decodes and checks them itself.
            if !wanted(entry) {
                return Ok(true);
            }
            loop {
                let n = read.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                sink(&chunk[..n])?;
            }
            sink(b"\n\n")?;
            Ok(true)
        })
        .map_err(unreadable)
}

fn cancelled_error() -> CoreError {
    CoreError::new("Import cancelled.")
}

/// The file an import reads, opened, and the name its import records (`imports.file`).
pub struct Source {
    file: File,
    name: Option<String>,
    entry: Option<Entry>,
}

impl Source {
    /// A file the user chose: a regular file outside the profile, so an import can never read
    /// the profile's own files (its logins, its databases).
    pub fn chosen(path: &Path, profile: &Path) -> Result<Source> {
        let unreadable = |e| CoreError::wrap("Could not read that file", e);
        let real = std::fs::canonicalize(path).map_err(unreadable)?;
        if let Ok(profile) = std::fs::canonicalize(profile)
            && real.starts_with(&profile)
        {
            return Err(CoreError::new("Choose a file outside KChess's own folder."));
        }
        let file = File::open(&real).map_err(unreadable)?;
        if !file.metadata().map_err(unreadable)?.is_file() {
            return Err(CoreError::new("Choose a .pgn, .pgn.zst, .zip or .7z file."));
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        Ok(Source {
            file,
            name,
            entry: None,
        })
    }

    /// A file the core downloaded itself (`master`), recorded as the file `name` it came as.
    pub fn downloaded(path: &Path, name: &str) -> Result<Source> {
        let file =
            File::open(path).map_err(|e| CoreError::wrap("Could not read the download", e))?;
        Ok(Source {
            file,
            name: Some(name.to_string()),
            entry: None,
        })
    }

    /// The same file, of which only the archive's files `entry` names are read.
    pub fn only(self, entry: Entry) -> Source {
        Source {
            entry: Some(entry),
            ..self
        }
    }
}

/// Names (players, events) by id, added to their table the first time a game names them, with
/// each of their words (`words`).
struct Names {
    table: &'static str,
    words: &'static str,
    ids: HashMap<String, i64>,
}

impl Names {
    fn new(table: &'static str, words: &'static str) -> Names {
        Names {
            table,
            words,
            ids: HashMap::new(),
        }
    }

    fn id(&mut self, db: &Connection, name: &str) -> Result<i64> {
        if let Some(&id) = self.ids.get(name) {
            return Ok(id);
        }
        let (table, words) = (self.table, self.words);
        let folded = gamedb::fold_name(name);
        let added = db
            .prepare_cached(&format!(
                "INSERT INTO {table} (name, norm) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING"
            ))
            .sql()?
            .execute(params![name, folded])
            .sql()?;
        let id: i64 = db
            .prepare_cached(&format!("SELECT id FROM {table} WHERE name = ?1"))
            .sql()?
            .query_row([name], |row| row.get(0))
            .sql()?;
        if added > 0 {
            let mut add = db
                .prepare_cached(&format!(
                    "INSERT OR IGNORE INTO {words} (word, id) VALUES (?1, ?2)"
                ))
                .sql()?;
            for word in folded.split(' ').filter(|word| !word.is_empty()) {
                add.execute(params![word, id]).sql()?;
            }
        }
        // Bounded: a file with millions of distinct names reads them from the table instead.
        if self.ids.len() < 1_000_000 {
            self.ids.insert(name.to_string(), id);
        }
        Ok(id)
    }
}

/// `YYYYMMDD` of a stored date, with 0 for what is unknown; orders games by date.
fn date_key(stored: &Stored) -> u32 {
    let day = stored
        .date
        .as_deref()
        .and_then(|date| date.split(['.', '-', '/']).nth(2))
        .and_then(|day| day.parse::<u32>().ok())
        .filter(|day| (1..=31).contains(day))
        .unwrap_or(0);
    let year = stored.year.and_then(|y| u32::try_from(y).ok()).unwrap_or(0);
    year * 10_000 + u32::from(stored.month.unwrap_or(0)) * 100 + day
}

/// The lower rating of a game both of whose players have one: "both rated at least" is one
/// indexed range on it.
fn low_elo(stored: &Stored) -> Option<u16> {
    Some(stored.white_elo?.min(stored.black_elo?))
}

/// The average of the ratings a game has.
fn rating(stored: &Stored) -> Option<u16> {
    match (stored.white_elo, stored.black_elo) {
        (Some(w), Some(b)) => Some(((u32::from(w) + u32::from(b)) / 2) as u16),
        (one, other) => one.or(other),
    }
}

fn insert_game(
    db: &Connection,
    import: i64,
    stored: &Stored,
    players: &mut Names,
    events: &mut Names,
) -> Result<i64> {
    let white = players.id(db, &stored.white)?;
    let black = players.id(db, &stored.black)?;
    let event = match &stored.event {
        Some(event) => Some(events.id(db, event)?),
        None => None,
    };
    db.prepare_cached(
        "INSERT INTO games (import_id, white, black, event, site, date, date_key, year, round,
            white_elo, black_elo, rating, low_elo, result, eco, opening, fen, plies, moves, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20)",
    )
    .sql()?
    .execute(params![
        import,
        white,
        black,
        event,
        stored.site,
        stored.date,
        date_key(stored),
        stored.year,
        stored.round,
        stored.white_elo,
        stored.black_elo,
        rating(stored),
        low_elo(stored),
        stored.outcome.score(),
        stored.eco,
        stored.opening,
        stored.fen,
        stored.plies as i64,
        stored.moves,
        stored.extra,
    ])
    .sql()?;
    Ok(db.last_insert_rowid())
}

/// What one finished game adds to the indexes, in one pass over its moves: each of its first
/// `depth` positions and the move played there to the tree, and the structures of the rest to
/// the position index.
fn index_game(
    tree: &mut TreeBuilder,
    deep: &mut DeepBuilder,
    id: i64,
    stored: &Stored,
    depth: u32,
) -> Result<()> {
    // An unfinished game counts in no result, so the tree leaves it out, as Lichess's does.
    if stored.outcome == Outcome::Unknown {
        return Ok(());
    }
    let Some(start) = gamedb::start_of(stored.fen.as_deref()) else {
        return Ok(());
    };
    let stats = Stats::of(id, stored.outcome, rating(stored), date_key(stored));
    let mut ply = 0;
    let mut failed = None;
    let mut structure = None;
    gamedb::walk_line(&start, &stored.moves, |pos, m| {
        let added = if ply < depth {
            tree.add(gamedb::position_key(pos), gamedb::pack_move(m), &stats)
        } else {
            // A structure lasts until a capture or a pawn move, and never comes back.
            let key = gamedb::structure_key(pos);
            if structure == Some(key) {
                Ok(())
            } else {
                structure = Some(key);
                deep.add(key, id)
            }
        };
        ply += 1;
        match added {
            Ok(()) => true,
            Err(error) => {
                failed = Some(error);
                false
            }
        }
    });
    failed.map_or(Ok(()), Err)
}

/// The games of `source` into `target`. `dir` holds the tree's run files; `capacity` bounds the
/// tree rows held in memory.
pub fn run(
    source: Source,
    packing: PgnPacking,
    target: &Target,
    dir: &Path,
    capacity: usize,
    watch: &mut dyn Watch,
) -> Result<Count> {
    let Source {
        file,
        name: file_name,
        entry,
    } = source;
    let total_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let input = Input {
        reader: file,
        total_bytes,
        packing,
        label: file_name,
        keep: None,
        entry,
    };
    read_into(input, target, dir, capacity, watch)
}

/// The games `input` reads into `target`, as `run` reads a file's: for games the core holds
/// itself rather than in a file ("Your games", `own`).
pub fn read_into<R: Read + Seek + Send + 'static>(
    input: Input<R>,
    target: &Target,
    dir: &Path,
    capacity: usize,
    watch: &mut dyn Watch,
) -> Result<Count> {
    let Input {
        reader,
        total_bytes,
        packing,
        label,
        keep,
        entry,
    } = input;
    let wanted = entry.unwrap_or(archived_pgn);
    let read = Arc::new(AtomicU64::new(0));
    let counted = Counted {
        inner: reader,
        read: Arc::clone(&read),
    };
    let now = crate::time::now_ms();
    let (db, adding, replacing) = match target {
        Target::New {
            partial,
            name,
            source,
            attribution,
            ..
        } => {
            remove_files(partial);
            let db = schema::create(partial, name, *source, attribution.as_deref(), now)?;
            (db, false, false)
        }
        Target::Existing { file } => (schema::open_write(file)?, true, false),
        Target::Replace { file } => (schema::open_write(file)?, true, true),
    };
    let result = (|| {
        let depth: u32 = db
            .query_row("SELECT index_depth FROM meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .sql()?;
        // An import that never finished (KChess was stopped under it) left games no search
        // sees; they go before this one's, which a later finished import would make visible.
        transaction(&db, || {
            db.execute_batch(
                "DELETE FROM games WHERE import_id IN
                   (SELECT id FROM imports WHERE finished_at IS NULL);
                 DELETE FROM imports WHERE finished_at IS NULL;",
            )
            .sql()
        })?;
        db.execute(
            "INSERT INTO imports (started_at, file) VALUES (?1, ?2)",
            params![now, label],
        )
        .sql()?;
        let import = db.last_insert_rowid();
        let count = if packing != PgnPacking::Plain {
            let (send, receive) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
            let decoder = std::thread::Builder::new()
                .name("kchess-gamedb-decode".into())
                .spawn(move || {
                    let sink = |chunk: &[u8]| {
                        send.blocking_send(chunk.to_vec())
                            .map_err(|_| io::Error::other("import stopped"))
                    };
                    match packing {
                        PgnPacking::Zip => unzip_pgn(counted, wanted, sink),
                        PgnPacking::SevenZip => un7z_pgn(counted, wanted, sink),
                        _ => crate::zst::decode(counted, sink).map_err(|cause| {
                            CoreError::wrap("That file is not a readable .pgn.zst", cause)
                        }),
                    }
                })
                .map_err(|e| CoreError::wrap("Could not start the import", e))?;
            let input = kept(
                BufReader::with_capacity(1 << 20, crate::zst::ChannelReader::new(receive)),
                keep,
            );
            let games = read_games(
                &db,
                import,
                input,
                depth,
                dir,
                capacity,
                &read,
                total_bytes,
                watch,
            );
            // The decoder ends once the reader is dropped; its failure (a damaged file) fails
            // the import even when the games read so far looked complete.
            let decoded = decoder
                .join()
                .map_err(|_| CoreError::new("The import stopped. Try again."))?;
            match (games, decoded) {
                (Ok(games), Ok(())) => Ok(games),
                (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            }?
        } else {
            let input = kept(BufReader::with_capacity(1 << 20, counted), keep);
            read_games(
                &db,
                import,
                input,
                depth,
                dir,
                capacity,
                &read,
                total_bytes,
                watch,
            )?
        };
        Ok::<_, CoreError>((import, count))
    })();
    let (import, (count, tree, deep)) = match result {
        Ok(done) => done,
        Err(error) => {
            undo(db, target);
            return Err(error);
        }
    };
    watch.indexing(count);
    let finished =
        (|| {
            if watch.cancelled() {
                return Err(cancelled_error());
            }
            let finish =
                || {
                    if replacing {
                        // The earlier games go with their tree rows, their position index and
                        // the names only they had: the indexes are the import's alone, whole.
                        db.execute("DELETE FROM games WHERE import_id != ?1", [import])
                            .sql()?;
                        db.execute("DELETE FROM imports WHERE id != ?1", [import])
                            .sql()?;
                        db.execute_batch(
                            "DELETE FROM tree;
                             DELETE FROM lone;
                             DELETE FROM deep;
                             DELETE FROM players WHERE id NOT IN
                               (SELECT white FROM games UNION SELECT black FROM games);
                             DELETE FROM player_words WHERE id NOT IN (SELECT id FROM players);
                             DELETE FROM events WHERE id NOT IN
                               (SELECT event FROM games WHERE event IS NOT NULL);
                             DELETE FROM event_words WHERE id NOT IN (SELECT id FROM events);
                             UPDATE meta SET deep_built = 1 WHERE id = 1;",
                        )
                        .sql()?;
                    }
                    tree.write(&db, adding && !replacing, &|| watch.cancelled())?;
                    deep.write(&db, &|| watch.cancelled())?;
                    db.execute(
                "UPDATE imports SET finished_at = ?2, games = ?3, skipped = ?4 WHERE id = ?1",
                params![import, crate::time::now_ms(), count.games as i64, count.skipped as i64],
            )
            .sql()?;
                    db.execute(
                        "UPDATE meta SET updated_at = ?1 WHERE id = 1",
                        [crate::time::now_ms()],
                    )
                    .sql()?;
                    Ok(())
                };
            if adding {
                transaction(&db, finish)?;
            } else {
                finish()?;
                if watch.cancelled() {
                    return Err(cancelled_error());
                }
                schema::finish(&db)?;
            }
            Ok(())
        })();
    if let Err(error) = finished {
        undo(db, target);
        return Err(error);
    }
    drop(db);
    if let Target::New { file, partial, .. } = target {
        std::fs::rename(partial, file).map_err(|e| {
            remove_files(partial);
            CoreError::wrap("Could not save the database", e)
        })?;
    }
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
fn read_games(
    db: &Connection,
    import: i64,
    input: impl std::io::BufRead,
    depth: u32,
    dir: &Path,
    capacity: usize,
    read: &AtomicU64,
    total_bytes: u64,
    watch: &mut dyn Watch,
) -> Result<(Count, TreeBuilder, DeepBuilder)> {
    let mut tree = TreeBuilder::new(dir, capacity)?;
    let mut deep = DeepBuilder::new(dir, deep::CAPACITY)?;
    let mut players = Names::new("players", "player_words");
    let mut events = Names::new("events", "event_words");
    let mut count = Count {
        total_bytes,
        ..Count::default()
    };
    let mut open = false;
    let result = (|| {
        for item in PgnReader::new(input) {
            let item = item.map_err(|e| CoreError::wrap("Could not read that file", e))?;
            let stored = match item {
                PgnItem::Game(game) => gamedb::store(&game).ok(),
                PgnItem::TooLarge => None,
            };
            let Some(stored) = stored else {
                count.skipped += 1;
                continue;
            };
            if !open {
                db.execute_batch("BEGIN").sql()?;
                open = true;
            }
            let id = insert_game(db, import, &stored, &mut players, &mut events)?;
            index_game(&mut tree, &mut deep, id, &stored, depth)?;
            count.games += 1;
            if (count.games + count.skipped).is_multiple_of(1_000) {
                if watch.cancelled() {
                    return Err(cancelled_error());
                }
                count.bytes = read.load(Ordering::Relaxed);
                watch.reading(count);
            }
            if count.games.is_multiple_of(BATCH) {
                db.execute_batch("COMMIT").sql()?;
                open = false;
            }
        }
        if open {
            db.execute_batch("COMMIT").sql()?;
            open = false;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if open {
            db.execute_batch("ROLLBACK").ok();
        }
        return Err(error);
    }
    count.bytes = read.load(Ordering::Relaxed);
    watch.reading(count);
    Ok((count, tree, deep))
}

/// The files of a database: the file itself and SQLite's journal beside it.
pub fn remove_files(file: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = file.as_os_str().to_owned();
        name.push(suffix);
        std::fs::remove_file(PathBuf::from(name)).ok();
    }
}

/// What a failed import leaves undone: a new database's file removed, or an existing one's games
/// of this import deleted (its tree rows were never written).
fn undo(db: Connection, target: &Target) {
    match target {
        Target::New { partial, .. } => {
            drop(db);
            remove_files(partial);
        }
        Target::Existing { .. } | Target::Replace { .. } => {
            let unfinished: Option<i64> = db
                .query_row(
                    "SELECT id FROM imports WHERE finished_at IS NULL ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .ok()
                .flatten();
            if let Some(import) = unfinished {
                transaction(&db, || {
                    db.execute("DELETE FROM games WHERE import_id = ?1", [import])
                        .sql()?;
                    db.execute("DELETE FROM imports WHERE id = ?1", [import])
                        .sql()?;
                    Ok(())
                })
                .ok();
            }
        }
    }
}
