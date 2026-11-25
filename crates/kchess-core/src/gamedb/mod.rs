//! The local game databases (`databases/*.kdb` in the profile), which this service alone owns.
//!
//! Each database is one SQLite file: removing a database removes its file, and an import that
//! fails or is cancelled leaves every database as it was (`import`). Searches read each file
//! through its own read-only lane, so they go on while an import writes. One import (or a
//! download: the master games, `master`, or a player's games, `player`) runs at a time; its progress reaches every window as
//! `gamedb:progress`. One database, "Your games", is kept in step with the player's own games
//! (`own`), rebuilt when no import or download runs.

pub mod deep;
pub mod edit;
pub mod explore;
pub mod import;
pub mod lichess;
pub mod master;
pub mod names;
pub mod novelty;
pub mod own;
pub mod player;
pub mod report;
pub mod schema;
pub mod search;
pub mod tcec;
pub mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::host::{Config, Host, Level};
use crate::store::Database;
use crate::store::sql::{Lane, LaneMessages, Sql as _};
use crate::tasks::lock;
use kchess_contracts::CoreEvent;
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    DatabaseInfo, DatabaseSource, GameDbImported, GameDbNameGroup, GameDbNovelty, GameDbPage,
    GameDbPhase as Phase, GameDbPlayerReport, GameDbProgress, GameDbQuery, GameDbSaved,
    GameDbStatus, LookupOptions, PositionLookup,
};
use kchess_domain::gamedb;

/// Tree rows an import holds in memory before it writes a run file: 7/8 of a 2^21-slot table,
/// the most it holds without growing, at 64 bytes a slot some 130 MB.
const TREE_ROWS: usize = 1_835_008;
/// Tree rows held for a few games saved from the board: its full table would take 130 MB for
/// one game's forty rows.
const FEW_ROWS: usize = 4_096;
/// The files a copy writes its games to before it imports them, swept as the run files are.
const COPY_PREFIX: &str = ".copy-";
/// How often an import reports its progress.
const REPORT_EVERY: Duration = Duration::from_millis(250);

/// Sets its flag when dropped: a lookup's caller holds it, so the replay it stops ends with it.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The running import: cancelled through the token.
struct Running {
    cancel: CancellationToken,
    database: String,
}

struct Inner {
    host: Arc<dyn Host>,
    profile: PathBuf,
    dir: PathBuf,
    readers: Mutex<HashMap<String, Arc<Lane>>>,
    running: Mutex<Option<Arc<Running>>>,
    /// The games the last position search found, for its other pages.
    listed: Arc<explore::Listed>,
    /// "Your games" and the keeper that rebuilds it.
    own: own::Own,
}

pub struct GameDbService {
    inner: Arc<Inner>,
}

/// The running import, released when dropped: cancelled, and its end reported to every window
/// (cancelled, when it ended without saying how, as when its caller went away).
struct Importing<'a> {
    inner: &'a Inner,
    running: Arc<Running>,
    end: Option<GameDbProgress>,
}

impl Drop for Importing<'_> {
    fn drop(&mut self) {
        self.running.cancel.cancel();
        {
            let mut running = lock(&self.inner.running);
            if running
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.running))
            {
                running.take();
            }
        }
        let end = self.end.take().unwrap_or_else(|| GameDbProgress {
            phase: Phase::Cancelled,
            database: self.running.database.clone(),
            games: 0,
            skipped: 0,
            bytes: 0,
            total_bytes: None,
            message: None,
        });
        self.inner.host.emit(CoreEvent::GameDbProgress(end));
        // A rebuild of "Your games" waiting for the import to end goes ahead.
        self.inner.own.idle.notify_waiters();
    }
}

/// Reports an import's progress to every window, at most every `REPORT_EVERY`.
struct Reporter {
    host: Arc<dyn Host>,
    database: String,
    cancel: CancellationToken,
    last: Option<Instant>,
    /// What the same run added before this file: the master games' earlier months.
    earlier: import::Count,
    /// The month of the master games being brought in, as the progress names it.
    month: Option<String>,
}

impl Reporter {
    fn new(host: &Arc<dyn Host>, database: &str, cancel: &CancellationToken) -> Reporter {
        Reporter {
            host: Arc::clone(host),
            database: database.to_string(),
            cancel: cancel.clone(),
            last: None,
            earlier: import::Count::default(),
            month: None,
        }
    }

    fn send(&mut self, phase: Phase, count: import::Count, force: bool) {
        if !force && self.last.is_some_and(|last| last.elapsed() < REPORT_EVERY) {
            return;
        }
        self.last = Some(Instant::now());
        let count = import::Count {
            games: self.earlier.games + count.games,
            skipped: self.earlier.skipped + count.skipped,
            ..count
        };
        self.host.emit(CoreEvent::GameDbProgress(progress(
            phase,
            &self.database,
            count,
            self.month.clone(),
        )));
    }
}

impl import::Watch for Reporter {
    fn reading(&mut self, count: import::Count) {
        self.send(Phase::Reading, count, false);
    }

    fn indexing(&mut self, count: import::Count) {
        self.send(Phase::Indexing, count, true);
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

fn progress(
    phase: Phase,
    database: &str,
    count: import::Count,
    message: Option<String>,
) -> GameDbProgress {
    GameDbProgress {
        phase,
        database: database.to_string(),
        games: count.games,
        skipped: count.skipped,
        bytes: count.bytes,
        total_bytes: (count.total_bytes > 0).then_some(count.total_bytes),
        message,
    }
}

/// A new database's id: its name's letters, then a random suffix so names may repeat.
fn new_id(name: &str) -> String {
    let mut stem: String = gamedb::fold_name(name)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .filter(|c| c.is_ascii())
        .take(28)
        .collect();
    stem = stem.trim_matches('-').to_string();
    if stem.is_empty() {
        stem = "games".to_string();
    }
    let mut suffix = [0u8; 3];
    if getrandom::fill(&mut suffix).is_err() {
        // No random bytes: the clock still tells two imports a second apart.
        suffix.copy_from_slice(&crate::time::now_ms().to_le_bytes()[..3]);
    }
    format!("{stem}-{:02x}{:02x}{:02x}", suffix[0], suffix[1], suffix[2])
}

/// The name a file gives a database: its name without `.pgn`, `.pgn.zst`, `.zip` or `.7z`.
fn file_stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let stem = [
        ".pgn.zst", ".pgn.bz2", ".pgn.zip", ".pgn.7z", ".zip", ".7z", ".pgn",
    ]
    .iter()
    .find_map(|ext| {
        lower
            .ends_with(ext)
            .then(|| &name[..name.len() - ext.len()])
    })
    .unwrap_or(&name);
    let stem: String = stem.trim().chars().take(80).collect();
    if stem.is_empty() {
        "Games".to_string()
    } else {
        stem
    }
}

impl Inner {
    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{}", schema::EXTENSION))
    }

    /// The read lane of database `id`, made on first use.
    fn reader(&self, id: &str) -> Result<Arc<Lane>> {
        if !kchess_contracts::check::is_database_id(id) || !self.file(id).is_file() {
            return Err(CoreError::new("That database is not on this computer."));
        }
        let mut readers = lock(&self.readers);
        if let Some(lane) = readers.get(id) {
            return Ok(Arc::clone(lane));
        }
        let path = self.file(id);
        let lane = Arc::new(Lane::new(
            "gamedb",
            LaneMessages {
                closed: "Game database closed.",
                stopped: "Game database stopped. Retry the operation.",
            },
            Arc::clone(&self.host),
            move |host| {
                // A file an older release wrote gains what lookups and searches read (the
                // replay index, its names' words); one that cannot is still read, more slowly.
                if let Err(error) = schema::upgrade(&path) {
                    host.log(
                        Level::Warn,
                        "gamedb",
                        &format!("Could not upgrade a game database: {}", error.message),
                    );
                }
                schema::open_read(&path)
            },
        ));
        readers.insert(id.to_string(), Arc::clone(&lane));
        Ok(lane)
    }

    /// Stops reading database `id`: its file is released before it is replaced or removed.
    async fn release(&self, id: &str) {
        let lane = lock(&self.readers).remove(id);
        if let Some(lane) = lane {
            lane.close().await;
        }
    }

    fn running(&self) -> Option<Arc<Running>> {
        lock(&self.running).clone()
    }

    fn logged<T>(&self, method: &str, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            self.host.log(
                Level::Warn,
                "gamedb",
                &format!("Game database {method} failed {}", error.message),
            );
        }
        result
    }

    /// What an earlier run left when it was stopped mid-import: partial files and run files.
    /// Only while nothing writes a database: the slot's lock is held, so none starts meanwhile.
    fn sweep(&self) {
        let running = lock(&self.running);
        if running.is_some() || self.own.building() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            // The run files of the opening tree and of the position index.
            if name.starts_with(".tree-") || name.starts_with(".deep-") {
                std::fs::remove_dir_all(&path).ok();
            } else if name.starts_with(COPY_PREFIX) {
                std::fs::remove_file(&path).ok();
            } else if name.ends_with(".partial") {
                import::remove_files(&path);
            }
        }
    }

    fn ids(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()? == schema::EXTENSION)
                    .then(|| path.file_stem()?.to_str().map(str::to_string))?
            })
            .filter(|id| kchess_contracts::check::is_database_id(id))
            .collect();
        ids.sort();
        ids
    }

    async fn info(&self, id: &str) -> Result<Option<DatabaseInfo>> {
        let bytes = std::fs::metadata(self.file(id)).map_or(0, |m| m.len())
            + std::fs::metadata(self.dir.join(format!("{id}.{}-wal", schema::EXTENSION)))
                .map_or(0, |m| m.len());
        let lane = self.reader(id)?;
        let id = id.to_string();
        lane.run(move |db| schema::info(db, &id, bytes)).await
    }

    /// Where an import writes the new database `id`: a `.partial` file renamed into place.
    fn new_target(
        &self,
        id: &str,
        name: String,
        source: DatabaseSource,
        attribution: Option<String>,
    ) -> import::Target {
        import::Target::New {
            partial: self.dir.join(format!("{id}.{}.partial", schema::EXTENSION)),
            file: self.file(id),
            name,
            source,
            attribution,
        }
    }

    /// Runs one import on a blocking thread: the file `open` gives into `target`. A new
    /// database's file is made private once it is in place.
    async fn run_import(
        &self,
        target: import::Target,
        packing: PgnPacking,
        mut reporter: Reporter,
        open: impl FnOnce() -> Result<import::Source> + Send + 'static,
    ) -> Result<import::Count> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            let result = open().and_then(|source| {
                import::run(source, packing, &target, &dir, TREE_ROWS, &mut reporter)
            });
            if let import::Target::New { file, .. } = &target
                && result.is_ok()
            {
                crate::store::restrict_database(file, reporter.host.as_ref(), "gamedb");
            }
            result
        })
        .await
        .map_err(|_| CoreError::new("The import stopped. Try again."))?
    }
}

/// The last report of a run that ended in `error`: cancelled when it was asked to stop.
fn failed(running: &Running, database: &str, error: &CoreError) -> GameDbProgress {
    let phase = if running.cancel.is_cancelled() {
        Phase::Cancelled
    } else {
        Phase::Failed
    };
    progress(
        phase,
        database,
        import::Count::default(),
        (phase == Phase::Failed).then(|| error.message.clone()),
    )
}

impl GameDbService {
    pub fn new(config: &Config, host: Arc<dyn Host>) -> GameDbService {
        GameDbService {
            inner: Arc::new(Inner {
                host,
                profile: config.data_dir.clone(),
                dir: config.data_dir.join("databases"),
                readers: Mutex::new(HashMap::new()),
                running: Mutex::new(None),
                listed: Arc::default(),
                own: own::Own::new(),
            }),
        }
    }

    /// Keeps "Your games" in step with `database`'s own games, from now until `stop`.
    pub fn keep_own(&self, database: Arc<Database>) {
        own::start(&self.inner, database, own::QUIET);
    }

    /// `keep_own`, following the games once they have not changed for `quiet`.
    #[cfg(test)]
    fn keep_own_after(&self, database: Arc<Database>, quiet: Duration) {
        own::start(&self.inner, database, quiet);
    }

    /// What the keeper of "Your games" has done, as it does it.
    #[cfg(test)]
    fn own_report(&self) -> tokio::sync::watch::Receiver<own::Report> {
        own::report(&self.inner)
    }

    /// Tests: "Your games" follows the games as they are now, without waiting for quiet; what the
    /// keeper has done once it has.
    pub async fn settle_own(&self) -> own::Report {
        own::settle(&self.inner).await
    }

    pub async fn status(&self) -> Result<GameDbStatus> {
        let busy = self.inner.running().is_some();
        self.inner.sweep();
        let mut databases = Vec::new();
        for id in self.inner.ids() {
            match self.inner.info(&id).await {
                Ok(Some(info)) => databases.push(info),
                Ok(None) => {}
                // One unreadable file must not hide the others: it is logged and left out.
                Err(error) => {
                    self.inner.logged("status", Err::<(), _>(error)).ok();
                }
            }
        }
        databases.sort_by_key(|database| database.name.to_lowercase());
        Ok(GameDbStatus { databases, busy })
    }

    /// The one run (an import or the master games download) that may write, claimed for the
    /// database `id`; refused while another runs.
    fn claim(&self, id: &str) -> Result<Importing<'_>> {
        let running = Arc::new(Running {
            cancel: CancellationToken::new(),
            database: id.to_string(),
        });
        let mut slot = lock(&self.inner.running);
        if slot.is_some() {
            return Err(CoreError::new(
                "Another import is running. Wait for it or cancel it first.",
            ));
        }
        *slot = Some(Arc::clone(&running));
        // A rebuild of "Your games" gives way, and runs again once this run has ended.
        if let Some(build) = lock(&self.inner.own.build).as_ref() {
            build.cancel();
        }
        Ok(Importing {
            inner: &self.inner,
            running,
            end: None,
        })
    }

    /// Reads the PGN file at `path` into the database `into`, or a new one named `name`.
    pub async fn import(
        &self,
        path: String,
        packing: PgnPacking,
        into: Option<String>,
        name: Option<String>,
        source: DatabaseSource,
    ) -> Result<GameDbImported> {
        let result = self.import_inner(path, packing, into, name, source).await;
        self.inner.logged("import", result)
    }

    async fn import_inner(
        &self,
        path: String,
        packing: PgnPacking,
        into: Option<String>,
        name: Option<String>,
        source: DatabaseSource,
    ) -> Result<GameDbImported> {
        let path = PathBuf::from(path);
        let id = match &into {
            Some(id) => {
                own::refuse(id, "Your games holds only your own games.")?;
                if !self.inner.file(id).is_file() {
                    return Err(CoreError::new("That database is not on this computer."));
                }
                id.clone()
            }
            None => new_id(name.as_deref().unwrap_or(&file_stem(&path))),
        };
        let mut guard = self.claim(&id)?;
        let running = Arc::clone(&guard.running);
        std::fs::create_dir_all(&self.inner.dir)
            .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
        let target = match into {
            Some(_) => import::Target::Existing {
                file: self.inner.file(&id),
            },
            None => {
                self.inner
                    .new_target(&id, name.unwrap_or_else(|| file_stem(&path)), source, None)
            }
        };
        let mut reporter = Reporter::new(&self.inner.host, &id, &running.cancel);
        reporter.send(Phase::Reading, import::Count::default(), true);
        let profile = self.inner.profile.clone();
        let outcome = self
            .inner
            .run_import(target, packing, reporter, move || {
                import::Source::chosen(&path, &profile)
            })
            .await;
        let count = match outcome {
            Ok(count) => count,
            Err(error) => {
                guard.end = Some(failed(&running, &id, &error));
                return Err(error);
            }
        };
        let database =
            self.inner.info(&id).await?.ok_or_else(|| {
                CoreError::new("The database could not be read after the import.")
            })?;
        guard.end = Some(progress(Phase::Done, &id, count, None));
        drop(guard);
        Ok(GameDbImported {
            database,
            added: count.games,
            skipped: count.skipped,
        })
    }

    pub fn cancel(&self) {
        if let Some(running) = self.inner.running() {
            running.cancel.cancel();
        }
    }

    fn refuse_while_importing(&self, id: &str) -> Result<()> {
        match self.inner.running() {
            Some(running) if running.database == id => Err(CoreError::new(
                "Cancel the import into this database first.",
            )),
            _ => Ok(()),
        }
    }

    pub async fn delete(&self, id: &str) -> Result<GameDbStatus> {
        own::refuse(
            id,
            "Your games follows the games you sync and play, so it can't be deleted.",
        )?;
        self.refuse_while_importing(id)?;
        if !self.inner.file(id).is_file() {
            return Err(CoreError::new("That database is not on this computer."));
        }
        self.inner.release(id).await;
        import::remove_files(&self.inner.file(id));
        // A database made again under this id (the master games) counts its imports afresh.
        self.inner.listed.forget();
        self.status().await
    }

    pub async fn rename(&self, id: &str, name: String) -> Result<GameDbStatus> {
        own::refuse(id, "Your games can't be renamed.")?;
        self.refuse_while_importing(id)?;
        let file = self.inner.file(id);
        if !file.is_file() {
            return Err(CoreError::new("That database is not on this computer."));
        }
        let result = tokio::task::spawn_blocking(move || {
            let db = schema::open_write(&file)?;
            db.execute("UPDATE meta SET name = ?1 WHERE id = 1", [name])
                .sql()?;
            Ok(())
        })
        .await
        .map_err(|_| CoreError::new("Game database stopped. Retry the operation."))
        .and_then(|result| result);
        self.inner.logged("rename", result)?;
        self.status().await
    }

    /// The ids of the games `selection` names in database `id`, read through its reader.
    async fn selected(&self, id: &str, selection: edit::Selection) -> Result<Vec<i64>> {
        if let edit::Selection::Search(query) = &selection
            && query.database != id
        {
            return Err(CoreError::new("That search is of another database."));
        }
        let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));
        let stopped = Arc::clone(&stop.0);
        let lane = self.inner.reader(id)?;
        lane.run(move |db| edit::selected(db, &selection, &stopped))
            .await
    }

    /// Adds the games of `pgn` to database `id`; with `replacing`, then removes that game.
    async fn save_inner(
        &self,
        into: edit::Into,
        pgn: String,
        replacing: Option<u64>,
    ) -> Result<GameDbSaved> {
        let (id, target) = match into {
            edit::Into::Existing(id) => {
                own::refuse(&id, "Your games holds only your own games.")?;
                let file = self.inner.file(&id);
                if !file.is_file() {
                    return Err(CoreError::new("That database is not on this computer."));
                }
                (id, import::Target::Existing { file })
            }
            edit::Into::New(name) => {
                let id = new_id(&name);
                std::fs::create_dir_all(&self.inner.dir)
                    .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
                let target = self
                    .inner
                    .new_target(&id, name, DatabaseSource::Imported, None);
                (id, target)
            }
        };
        let id = id.as_str();
        let file = self.inner.file(id);
        let mut guard = self.claim(id)?;
        let running = Arc::clone(&guard.running);
        let mut reporter = Reporter::new(&self.inner.host, id, &running.cancel);
        let dir = self.inner.dir.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let total_bytes = pgn.len() as u64;
            let input = import::Input {
                reader: std::io::Cursor::new(pgn.into_bytes()),
                total_bytes,
                packing: PgnPacking::Plain,
                label: None,
                keep: None,
                entry: None,
            };
            let count = import::read_into(input, &target, &dir, FEW_ROWS, &mut reporter)?;
            if count.games == 0 {
                return Err(CoreError::new("No game could be read from that PGN."));
            }
            if let import::Target::New { file, .. } = &target {
                crate::store::restrict_database(file, reporter.host.as_ref(), "gamedb");
            }
            let game: i64 = schema::open_read(&file)?
                .query_row("SELECT MAX(id) FROM games", [], |row| row.get(0))
                .sql()?;
            // Added first, the old game goes after: a failure here leaves both, never neither.
            if let Some(old) = replacing {
                let old = i64::try_from(old).unwrap_or(i64::MAX);
                edit::remove(&file, &dir, &[old], TREE_ROWS, &mut reporter)?;
            }
            Ok((count, game as u64))
        })
        .await
        .map_err(|_| CoreError::new("The game could not be saved. Try again."))
        .and_then(|outcome| outcome);
        let (count, game) = match outcome {
            Ok(saved) => saved,
            Err(error) => {
                guard.end = Some(failed(&running, id, &error));
                return Err(error);
            }
        };
        self.inner.listed.forget();
        let database = self
            .inner
            .info(id)
            .await?
            .ok_or_else(|| CoreError::new("The database could not be read after saving."))?;
        guard.end = Some(progress(Phase::Done, id, count, None));
        Ok(GameDbSaved { database, game })
    }

    /// Adds the games of `pgn` (a game from the board) to database `id`.
    pub async fn add_games(&self, into: edit::Into, pgn: String) -> Result<GameDbSaved> {
        let result = self.save_inner(into, pgn, None).await;
        self.inner.logged("add", result)
    }

    /// Puts `pgn` in the place of the game `game` of database `id`.
    pub async fn replace_game(&self, id: &str, game: u64, pgn: String) -> Result<GameDbSaved> {
        let into = edit::Into::Existing(id.to_string());
        let result = self.save_inner(into, pgn, Some(game)).await;
        self.inner.logged("replace", result)
    }

    /// Removes the games `selection` names from database `id`; answers how many.
    pub async fn delete_games(&self, id: &str, selection: edit::Selection) -> Result<u64> {
        let result = self.delete_games_inner(id, selection).await;
        self.inner.logged("delete games", result)
    }

    async fn delete_games_inner(&self, id: &str, selection: edit::Selection) -> Result<u64> {
        own::refuse(id, "Your games follows the games you sync and play.")?;
        let mut guard = self.claim(id)?;
        let running = Arc::clone(&guard.running);
        let ids = match self.selected(id, selection).await {
            Ok(ids) => ids,
            Err(error) => {
                guard.end = Some(failed(&running, id, &error));
                return Err(error);
            }
        };
        if ids.is_empty() {
            guard.end = Some(progress(Phase::Done, id, import::Count::default(), None));
            return Ok(0);
        }
        let mut reporter = Reporter::new(&self.inner.host, id, &running.cancel);
        reporter.send(Phase::Indexing, import::Count::default(), true);
        let (file, dir) = (self.inner.file(id), self.inner.dir.clone());
        let outcome = tokio::task::spawn_blocking(move || {
            edit::remove(&file, &dir, &ids, TREE_ROWS, &mut reporter)
        })
        .await
        .map_err(|_| CoreError::new("The games could not be deleted. Try again."))
        .and_then(|outcome| outcome);
        match outcome {
            Ok(removed) => {
                self.inner.listed.forget();
                guard.end = Some(progress(Phase::Done, id, import::Count::default(), None));
                Ok(removed)
            }
            Err(error) => {
                guard.end = Some(failed(&running, id, &error));
                Err(error)
            }
        }
    }

    /// How many games of database `id` copy an earlier one.
    pub async fn duplicates(&self, id: &str) -> Result<u64> {
        let result = self
            .selected(id, edit::Selection::Duplicates)
            .await
            .map(|ids| ids.len() as u64);
        self.inner.logged("duplicates", result)
    }

    /// The player named `name` in database `id`, at a glance.
    pub async fn player_report(&self, id: &str, name: String) -> Result<GameDbPlayerReport> {
        let lane = self.inner.reader(id)?;
        let result = lane.run(move |db| report::player(db, &name)).await;
        self.inner.logged("player report", result)
    }

    /// The names of database `id` that are likely one player's.
    pub async fn name_groups(&self, id: &str) -> Result<Vec<GameDbNameGroup>> {
        let lane = self.inner.reader(id)?;
        let result = lane.run(names::groups).await;
        self.inner.logged("name groups", result)
    }

    /// Gives the games of each of `names` in database `id` the name `into`.
    pub async fn merge_names(&self, id: &str, into: String, names: Vec<String>) -> Result<u64> {
        own::refuse(id, "Your games names the players as their accounts do.")?;
        let file = self.inner.file(id);
        if !file.is_file() {
            return Err(CoreError::new("That database is not on this computer."));
        }
        let mut guard = self.claim(id)?;
        let result = tokio::task::spawn_blocking(move || names::merge(&file, &into, &names))
            .await
            .map_err(|_| CoreError::new("The names could not be merged. Try again."))
            .and_then(|result| result);
        guard.end = Some(match &result {
            Ok(_) => progress(Phase::Done, id, import::Count::default(), None),
            Err(error) => failed(&guard.running, id, error),
        });
        if result.is_ok() {
            self.inner.listed.forget();
        }
        self.inner.logged("merge names", result)
    }

    /// Copies the games `selection` names from database `from` into `into`, or a new database
    /// named `name`.
    pub async fn copy_games(
        &self,
        from: &str,
        selection: edit::Selection,
        into: Option<String>,
        name: Option<String>,
    ) -> Result<GameDbImported> {
        let result = self.copy_games_inner(from, selection, into, name).await;
        self.inner.logged("copy games", result)
    }

    async fn copy_games_inner(
        &self,
        from: &str,
        selection: edit::Selection,
        into: Option<String>,
        name: Option<String>,
    ) -> Result<GameDbImported> {
        let source = self
            .inner
            .info(from)
            .await?
            .ok_or_else(|| CoreError::new("That database is not on this computer."))?;
        let id = match &into {
            Some(into) if into == from => {
                return Err(CoreError::new("Choose another database to copy them into."));
            }
            Some(into) => {
                own::refuse(into, "Your games holds only your own games.")?;
                if !self.inner.file(into).is_file() {
                    return Err(CoreError::new("That database is not on this computer."));
                }
                into.clone()
            }
            None => new_id(name.as_deref().unwrap_or(&source.name)),
        };
        let mut guard = self.claim(&id)?;
        let running = Arc::clone(&guard.running);
        let ids = match self.selected(from, selection).await {
            Ok(ids) => ids,
            Err(error) => {
                guard.end = Some(failed(&running, &id, &error));
                return Err(error);
            }
        };
        let target = match into {
            Some(_) => import::Target::Existing {
                file: self.inner.file(&id),
            },
            None => self.inner.new_target(
                &id,
                name.unwrap_or_else(|| source.name.clone()),
                DatabaseSource::Imported,
                None,
            ),
        };
        let mut reporter = Reporter::new(&self.inner.host, &id, &running.cancel);
        reporter.send(Phase::Reading, import::Count::default(), true);
        let (from_file, dir) = (self.inner.file(from), self.inner.dir.clone());
        let label = source.name.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut copy = tempfile::Builder::new()
                .prefix(COPY_PREFIX)
                .suffix(".pgn")
                .tempfile_in(&dir)
                .map_err(|cause| CoreError::wrap("Could not copy the games", cause))?;
            {
                let db = schema::open_read(&from_file)?;
                let cancelled = || reporter.cancel.is_cancelled();
                let mut out = std::io::BufWriter::new(copy.as_file_mut());
                edit::write_pgn(&db, &ids, &mut out, &cancelled)?;
            }
            let file = copy
                .reopen()
                .map_err(|cause| CoreError::wrap("Could not copy the games", cause))?;
            let total_bytes = file.metadata().map_or(0, |meta| meta.len());
            let input = import::Input {
                reader: file,
                total_bytes,
                packing: PgnPacking::Plain,
                label: Some(label),
                keep: None,
                entry: None,
            };
            let count = import::read_into(input, &target, &dir, TREE_ROWS, &mut reporter)?;
            if let import::Target::New { file, .. } = &target {
                crate::store::restrict_database(file, reporter.host.as_ref(), "gamedb");
            }
            Ok(count)
        })
        .await
        .map_err(|_| CoreError::new("The games could not be copied. Try again."))
        .and_then(|outcome| outcome);
        let count = match outcome {
            Ok(count) => count,
            Err(error) => {
                guard.end = Some(failed(&running, &id, &error));
                return Err(error);
            }
        };
        let database = self
            .inner
            .info(&id)
            .await?
            .ok_or_else(|| CoreError::new("The database could not be read after the copy."))?;
        guard.end = Some(progress(Phase::Done, &id, count, None));
        Ok(GameDbImported {
            database,
            added: count.games,
            skipped: count.skipped,
        })
    }

    /// Writes the games `selection` names from database `id` as a PGN file at `path`, outside
    /// the profile; answers how many. The file appears whole or not at all.
    pub async fn export(&self, id: &str, selection: edit::Selection, path: String) -> Result<u64> {
        let result = self.export_inner(id, selection, path).await;
        self.inner.logged("export", result)
    }

    async fn export_inner(
        &self,
        id: &str,
        selection: edit::Selection,
        path: String,
    ) -> Result<u64> {
        let ids = self.selected(id, selection).await?;
        let (file, profile) = (self.inner.file(id), self.inner.profile.clone());
        tokio::task::spawn_blocking(move || {
            let path = PathBuf::from(path);
            let unwritable = |cause| CoreError::wrap("Could not save the games there", cause);
            let folder = path
                .parent()
                .ok_or_else(|| CoreError::new("Could not save the games there"))?;
            let real = std::fs::canonicalize(folder).map_err(unwritable)?;
            if let Ok(profile) = std::fs::canonicalize(&profile)
                && real.starts_with(&profile)
            {
                return Err(CoreError::new("Save them outside KChess's own folder."));
            }
            let mut out = tempfile::Builder::new()
                .prefix(".kchess-export-")
                .tempfile_in(&real)
                .map_err(unwritable)?;
            let db = schema::open_read(&file)?;
            let written = {
                let mut writer = std::io::BufWriter::new(out.as_file_mut());
                edit::write_pgn(&db, &ids, &mut writer, &|| false)?
            };
            out.persist(&path)
                .map_err(|cause| CoreError::wrap("Could not save the games there", cause.error))?;
            Ok(written)
        })
        .await
        .map_err(|_| CoreError::new("The games could not be saved. Try again."))?
    }

    pub async fn games(&self, query: GameDbQuery) -> Result<GameDbPage> {
        // A search the caller dropped (a newer one replaced it) stops replaying games for nobody.
        let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));
        let stopped = Arc::clone(&stop.0);
        let listed = Arc::clone(&self.inner.listed);
        let result = match self.inner.reader(&query.database) {
            Ok(lane) if query.position.is_some() => {
                lane.run(move |db| explore::games_at(db, &query, &listed, &stopped))
                    .await
            }
            Ok(lane) => lane.run(move |db| search::games(db, &query)).await,
            Err(error) => Err(error),
        };
        drop(stop);
        self.inner.logged("games", result)
    }

    pub async fn game(&self, id: &str, game: u64) -> Result<String> {
        let result = match self.inner.reader(id) {
            Ok(lane) => lane.run(move |db| search::pgn(db, game)).await,
            Err(error) => Err(error),
        };
        self.inner.logged("game", result)
    }

    /// The opening tree's moves from the position `key` in database `id`.
    pub async fn tree(&self, id: &str, key: u64) -> Result<Vec<tree::TreeMove>> {
        let result = match self.inner.reader(id) {
            Ok(lane) => lane.run(move |db| tree::moves(db, key)).await,
            Err(error) => Err(error),
        };
        self.inner.logged("tree", result)
    }

    /// The position `fen` looked up in the database `options` names, as the explorer shows it.
    pub async fn lookup(&self, fen: String, options: LookupOptions) -> Result<PositionLookup> {
        let Some(id) = options.database.clone() else {
            return Err(CoreError::new("Choose a game database."));
        };
        // A caller that drops the lookup (a newer one replaced it) stops its replay, which would
        // otherwise run on, holding the file's reader, for nobody.
        let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));
        let stopped = Arc::clone(&stop.0);
        let result = match self.inner.reader(&id) {
            Ok(lane) => {
                lane.run(move |db| {
                    explore::lookup(db, &id, &fen, &options, crate::time::now_ms(), &stopped)
                })
                .await
            }
            Err(error) => Err(error),
        };
        drop(stop);
        let saved = self.inner.logged("lookup", result)?;
        Ok(PositionLookup {
            saved,
            cached: false,
            stale: false,
            message: None,
        })
    }

    /// The line's first move no game of the databases played (Your games aside, which follows
    /// the games you play).
    pub async fn novelty(&self, line: novelty::Line) -> Result<GameDbNovelty> {
        let files: Vec<(String, PathBuf)> = self
            .inner
            .ids()
            .into_iter()
            .filter(|id| id != own::OWN_ID)
            .map(|id| {
                let file = self.inner.file(&id);
                (id, file)
            })
            .collect();
        // Dropped by its caller (the board moved on), the walk stops at its next lookup.
        let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));
        let stopped = Arc::clone(&stop.0);
        let result = tokio::task::spawn_blocking(move || {
            let mut databases = Vec::with_capacity(files.len());
            for (id, file) in files {
                // One being written for the first time has nothing to read yet.
                if let Ok(db) = schema::open_read(&file) {
                    databases.push((id, db));
                }
            }
            novelty::find(&databases, &line, &stopped)
        })
        .await
        .map_err(|_| CoreError::new("The databases could not be read. Try again."))
        .and_then(|found| found);
        drop(stop);
        self.inner.logged("novelty", result)
    }

    /// Stops the import and the rebuild of "Your games"; `close` waits for them to end.
    pub fn stop(&self) {
        self.cancel();
        own::stop(&self.inner);
    }

    /// Stops the import and the rebuild of "Your games", waits for the rebuild to end, and
    /// releases every file.
    pub async fn close(&self) {
        self.stop();
        own::close(&self.inner).await;
        let lanes: Vec<Arc<Lane>> = lock(&self.inner.readers).drain().map(|(_, l)| l).collect();
        for lane in lanes {
            lane.close().await;
        }
    }
}

/// A database made from `pgn` in the profile `data_dir`, as an import leaves it: for test
/// profiles and fixtures. Answers the new database's id.
pub fn write_database(data_dir: &Path, name: &str, pgn: &str) -> Result<String> {
    struct Quiet;
    impl import::Watch for Quiet {
        fn reading(&mut self, _: import::Count) {}
        fn indexing(&mut self, _: import::Count) {}
        fn cancelled(&self) -> bool {
            false
        }
    }
    let source = tempfile::tempdir().map_err(|e| CoreError::wrap("No temporary folder", e))?;
    let path = source.path().join("games.pgn");
    std::fs::write(&path, pgn).map_err(|e| CoreError::wrap("Could not write the games", e))?;
    let dir = data_dir.join("databases");
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::wrap("No databases folder", e))?;
    let id = new_id(name);
    let file = dir.join(format!("{id}.{}", schema::EXTENSION));
    let target = import::Target::New {
        partial: dir.join(format!("{id}.{}.partial", schema::EXTENSION)),
        file,
        name: name.to_string(),
        source: DatabaseSource::Imported,
        attribution: None,
    };
    let source = import::Source::chosen(&path, data_dir)?;
    import::run(
        source,
        PgnPacking::Plain,
        &target,
        &dir,
        TREE_ROWS,
        &mut Quiet,
    )?;
    Ok(id)
}

#[cfg(test)]
mod tests;
