//! "Your games": the database `own`, which the service keeps in step with the player's own games
//! in `kchess.db` (`store::own_games`): the synced Lichess and Chess.com games and the finished
//! games of Game history.
//!
//! - Every write that changes those games, by whichever store function, is counted
//!   (`Database::own_games_changes`). The keeper waits until no write has come for `QUIET` (a
//!   sync's pages are one burst), then writes the database again in the background: a new file
//!   the first time, else in place (`import::Target::Replace`), so a search reads the old games
//!   or the new ones, never a mix.
//! - The games' fingerprint is kept in the database (its import's `file`); a pass whose games
//!   have the same fingerprint writes nothing. The first pass after the core starts is such a
//!   check, so a database missing or older than the games is written once.
//! - No own games, no database: its file is removed rather than listed empty.
//! - One import runs at a time. A rebuild waits for a player's import to end, and an import that
//!   starts cancels the rebuild, which runs again once the import has ended.
//! - The database is the games': it is never renamed, deleted or imported into (`refuse`).
//!   Removing the games (signing out, removing an account, deleting played games) is how it
//!   shrinks or goes.
//! - Closing the service ends the keeper and waits for it: a rebuild under way is cancelled and
//!   leaves the database as it was.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::OptionalExtension as _;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{Inner, import};
use crate::host::Level;
use crate::store::Database;
use crate::store::own_games::{self, OwnGames};
use crate::store::sql::Sql as _;
use crate::tasks::lock;
use kchess_contracts::CoreEvent;
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{DatabaseSource, ProfileArea, ProfileChange};

/// The database's id, which the explorer's choice of database keeps.
pub const OWN_ID: &str = "own";
/// Its name, as every list shows it.
pub const OWN_NAME: &str = "Your games";
/// How long the games stay unchanged before the database follows them.
pub const QUIET: Duration = Duration::from_secs(2);
/// Tree rows a rebuild holds in memory, some 15 MB: the own games are a few thousand, and a
/// larger tree spills to run files as an import's does.
const TREE_ROWS: usize = 229_376;

/// What the keeper has done, for the tests to wait on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Passes finished: looked at the games and, when they changed, wrote the database.
    pub passes: u64,
    /// The last `settle` ticket a finished pass began after.
    pub served: u64,
    /// Passes that wrote or removed the database.
    pub rebuilds: u64,
    /// A rebuild is writing the database.
    pub building: bool,
}

/// The keeper's state in the service.
pub(super) struct Own {
    /// Ends the keeper: the service is closing.
    lifetime: CancellationToken,
    /// The rebuild under way, held under the import slot's lock: an import cancels it.
    pub(super) build: std::sync::Mutex<Option<CancellationToken>>,
    /// An import ended.
    pub(super) idle: Notify,
    task: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// `settle` tickets handed out.
    asked: AtomicU64,
    wake: Notify,
    report: watch::Sender<Report>,
}

impl Own {
    pub(super) fn new() -> Own {
        Own {
            lifetime: CancellationToken::new(),
            build: std::sync::Mutex::new(None),
            idle: Notify::new(),
            task: std::sync::Mutex::new(None),
            asked: AtomicU64::new(0),
            wake: Notify::new(),
            report: watch::channel(Report::default()).0,
        }
    }

    /// Whether a rebuild is writing: the import slot's lock is held, so none starts meanwhile.
    pub(super) fn building(&self) -> bool {
        lock(&self.build).is_some()
    }
}

/// Starts keeping the database in step with `database`'s own games, once; never after `stop`.
pub(super) fn start(inner: &Arc<Inner>, database: Arc<Database>, quiet: Duration) {
    let mut task = lock(&inner.own.task);
    if task.is_some() || inner.own.lifetime.is_cancelled() {
        return;
    }
    *task = Some(tokio::spawn(keep(Arc::clone(inner), database, quiet)));
}

/// Ends the keeper; `close` waits for it.
pub(super) fn stop(inner: &Inner) {
    inner.own.lifetime.cancel();
}

/// Waits for the keeper to end, once `stop` has ended it.
pub(super) async fn close(inner: &Inner) {
    let task = lock(&inner.own.task).take();
    if let Some(task) = task
        && let Err(cause) = task.await
    {
        inner.host.log(
            Level::Error,
            "gamedb",
            &format!("Your games stopped: {cause}"),
        );
    }
}

/// Looks at the games now, without waiting for quiet, and answers once a pass that began after
/// this call has finished (at once when the keeper is not running).
pub(super) async fn settle(inner: &Inner) -> Report {
    let own = &inner.own;
    let ticket = own.asked.fetch_add(1, Ordering::SeqCst) + 1;
    let mut report = own.report.subscribe();
    if lock(&own.task).is_none() {
        return *report.borrow();
    }
    own.wake.notify_one();
    tokio::select! {
        served = report.wait_for(|report| report.served >= ticket) => {
            served.map(|report| *report).unwrap_or_default()
        }
        () = own.lifetime.cancelled() => *own.report.borrow(),
    }
}

/// What the keeper has done, as it does it.
#[cfg(test)]
pub(super) fn report(inner: &Inner) -> watch::Receiver<Report> {
    inner.own.report.subscribe()
}

/// The database is the games': it takes no imports and is never renamed or deleted.
pub(super) fn refuse(id: &str, refusal: &'static str) -> Result<()> {
    if id == OWN_ID {
        return Err(CoreError::new(refusal));
    }
    Ok(())
}

/// How a pass ended.
enum Pass {
    /// The database is the games' (`true` when the pass wrote or removed it).
    Done(bool),
    /// An import cancelled the rebuild: the games are looked at again once it ends.
    Interrupted,
    /// The service is closing.
    Closed,
}

async fn keep(inner: Arc<Inner>, database: Arc<Database>, quiet: Duration) {
    let own = &inner.own;
    let mut changes = database.own_games_changes();
    let asked = || own.asked.load(Ordering::SeqCst) > own.report.borrow().served;
    // The first pass checks the database the last run left against the games.
    let mut due = true;
    loop {
        if !due {
            tokio::select! {
                () = own.lifetime.cancelled() => return,
                changed = changes.changed() => if changed.is_err() { return },
                () = own.wake.notified() => {}
            }
        }
        // A burst of writes (a sync's pages) is followed once, after its last write.
        while !asked() {
            tokio::select! {
                () = own.lifetime.cancelled() => return,
                () = tokio::time::sleep(quiet) => break,
                changed = changes.changed() => if changed.is_err() { return },
                () = own.wake.notified() => {}
            }
        }
        let serving = own.asked.load(Ordering::SeqCst);
        // The pass reads every change made before this point.
        changes.borrow_and_update();
        let rebuilt = match pass(&inner, &database).await {
            Ok(Pass::Done(rebuilt)) => rebuilt,
            Ok(Pass::Interrupted) => {
                due = true;
                continue;
            }
            Ok(Pass::Closed) => return,
            Err(cause) => {
                inner.host.log(
                    Level::Warn,
                    "gamedb",
                    &format!("Your games not updated: {}", cause.message),
                );
                false
            }
        };
        due = false;
        own.report.send_modify(|report| {
            report.passes += 1;
            report.served = report.served.max(serving);
            report.rebuilds += u64::from(rebuilt);
        });
    }
}

/// Clears the rebuild under way however its pass ends.
struct Building<'a>(&'a Inner);

impl Drop for Building<'_> {
    fn drop(&mut self) {
        lock(&self.0.own.build).take();
        self.0
            .own
            .report
            .send_modify(|report| report.building = false);
    }
}

/// The import slot, once no import runs: a rebuild under way, which a later import cancels.
async fn claim(inner: &Inner) -> Option<(CancellationToken, Building<'_>)> {
    let own = &inner.own;
    loop {
        let idle = own.idle.notified();
        tokio::pin!(idle);
        idle.as_mut().enable();
        if own.lifetime.is_cancelled() {
            return None;
        }
        {
            let running = lock(&inner.running);
            if running.is_none() {
                let token = own.lifetime.child_token();
                *lock(&own.build) = Some(token.clone());
                return Some((token, Building(inner)));
            }
        }
        tokio::select! {
            () = idle => {}
            () = own.lifetime.cancelled() => return None,
        }
    }
}

async fn pass(inner: &Arc<Inner>, database: &Arc<Database>) -> Result<Pass> {
    let Some((token, _building)) = claim(inner).await else {
        return Ok(Pass::Closed);
    };
    let stored = fingerprint(inner).await;
    let games = database
        .read_async(move |ctx| own_games::own_games(ctx.host, ctx.db, stored.as_deref()))
        .await?;
    let file = inner.file(OWN_ID);
    let (fingerprint, pgn) = match games {
        OwnGames::Unchanged => return Ok(Pass::Done(false)),
        OwnGames::Empty => {
            if !file.is_file() {
                return Ok(Pass::Done(false));
            }
            remove(inner).await;
            return Ok(Pass::Done(true));
        }
        OwnGames::Changed { fingerprint, pgn } => {
            (fingerprint, Arc::<[u8]>::from(pgn.into_bytes()))
        }
    };
    std::fs::create_dir_all(&inner.dir)
        .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
    inner
        .own
        .report
        .send_modify(|report| report.building = true);
    let mut built = build(inner, file.is_file(), &fingerprint, &pgn, &token).await;
    if let Err(cause) = &built
        && file.is_file()
        && !token.is_cancelled()
    {
        // A file this release cannot write (damaged, or from a newer KChess) is the games'
        // copy only: it is written again from them.
        inner.host.log(
            Level::Warn,
            "gamedb",
            &format!("Your games written afresh: {}", cause.message),
        );
        inner.release(OWN_ID).await;
        import::remove_files(&file);
        built = build(inner, false, &fingerprint, &pgn, &token).await;
    }
    match built {
        Ok(count) => {
            if count.games == 0 {
                // None of the games could be stored (a variant's, say): no database.
                remove(inner).await;
            } else {
                changed(inner);
            }
            Ok(Pass::Done(true))
        }
        Err(_) if inner.own.lifetime.is_cancelled() => Ok(Pass::Closed),
        Err(_) if token.is_cancelled() => Ok(Pass::Interrupted),
        Err(cause) => Err(cause),
    }
}

/// Writes the database from `pgn`: in place when `existing`, else as a new file.
async fn build(
    inner: &Inner,
    existing: bool,
    fingerprint: &str,
    pgn: &Arc<[u8]>,
    token: &CancellationToken,
) -> Result<import::Count> {
    struct Quiet(CancellationToken);
    impl import::Watch for Quiet {
        fn reading(&mut self, _: import::Count) {}
        fn indexing(&mut self, _: import::Count) {}
        fn cancelled(&self) -> bool {
            self.0.is_cancelled()
        }
    }
    let target = if existing {
        import::Target::Replace {
            file: inner.file(OWN_ID),
        }
    } else {
        inner.new_target(OWN_ID, OWN_NAME.to_string(), DatabaseSource::Own, None)
    };
    let input = import::Input {
        reader: Cursor::new(Arc::clone(pgn)),
        total_bytes: pgn.len() as u64,
        packing: PgnPacking::Plain,
        label: Some(fingerprint.to_string()),
        keep: None,
    };
    let dir = inner.dir.clone();
    let host = Arc::clone(&inner.host);
    let mut watch = Quiet(token.clone());
    tokio::task::spawn_blocking(move || {
        let count = import::read_into(input, &target, &dir, TREE_ROWS, &mut watch)?;
        if let import::Target::New { file, .. } = &target {
            crate::store::restrict_database(file, host.as_ref(), "gamedb");
        }
        Ok(count)
    })
    .await
    .map_err(|_| CoreError::new("Your games stopped. It is written again next time."))?
}

/// The fingerprint of the games the database was written from; None without a readable one.
async fn fingerprint(inner: &Inner) -> Option<String> {
    if !inner.file(OWN_ID).is_file() {
        return None;
    }
    let read = match inner.reader(OWN_ID) {
        Ok(lane) => {
            lane.run(|db| {
                db.query_row(
                    "SELECT file FROM imports WHERE finished_at IS NOT NULL
                     ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .sql()
            })
            .await
        }
        Err(error) => Err(error),
    };
    match read {
        Ok(fingerprint) => fingerprint.flatten(),
        Err(cause) => {
            inner.host.log(
                Level::Debug,
                "gamedb",
                &format!("Your games unread: {}", cause.message),
            );
            None
        }
    }
}

/// No games: no database.
async fn remove(inner: &Inner) {
    inner.release(OWN_ID).await;
    import::remove_files(&inner.file(OWN_ID));
    changed(inner);
}

/// Every window reads the list of databases again; a position search finds its games again.
fn changed(inner: &Inner) {
    inner.listed.forget();
    inner.host.emit(CoreEvent::ProfileChanged(ProfileChange {
        areas: vec![ProfileArea::GameDb],
        origin: crate::connection::NOBODY,
    }));
}
