//! KChess's main database (`kchess.db`), the storage the TypeScript services used to own
//! (`crates/kchess-node/js/{db,migrations,store,library,reviewStore,runs,insights,usage,voiceLog,
//! setupPositionLookup}.ts`, now removed). The Rust core is its only owner: one connection, opened
//! and migrated on first use, with every storage method dispatched from here.

pub mod debug;
pub mod games;
pub mod import;
pub mod insights;
pub mod library;
pub mod lookups;
pub mod migrations;
pub mod reviews;
pub mod runs;
pub mod usage;
pub mod voice_log;

use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::error::{CoreError, Result};
use crate::host::{Config, Host, Level};

/// `kchess.db` as the core shares it: one connection, opened and migrated on first use, behind a
/// lock. The synchronous storage calls and the asynchronous services (the review queue) both
/// reach it through `call`, so there is never a second copy of the database. Callers must not
/// hold the lock across another `call`.
pub struct Database {
    connection: Mutex<Option<Connection>>,
    host: Arc<dyn Host>,
    config: Config,
}

impl Database {
    pub fn new(config: Config, host: Arc<dyn Host>) -> Database {
        Database {
            connection: Mutex::new(None),
            host,
            config,
        }
    }

    /// Runs one storage method (`store.<module>.<name>`) against the database, opening it first.
    pub fn call(&self, method: &str, args: &[Value]) -> Result<Value> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| CoreError::new("The database is unavailable."))?;
        if method == "store.debug.historical" && debug::enabled() {
            // Test-only: an earlier release's database, written before the store opens the file.
            if db.is_some() {
                return Err(CoreError::new("The database is already open."));
            }
            let path = self.config.data_dir.join("kchess.db");
            return debug::historical(&path, args);
        }
        if db.is_none() {
            let path = self.config.data_dir.join("kchess.db");
            *db = Some(open(&path, self.host.as_ref())?);
        }
        match db.as_ref() {
            Some(connection) => {
                let ctx = StoreContext {
                    db: connection,
                    host: self.host.as_ref(),
                    config: &self.config,
                };
                call(&ctx, method, args)
            }
            None => Err(CoreError::new("The database is unavailable.")),
        }
    }

    /// Releases the file; the next `call` opens it again.
    pub fn close(&self) {
        if let Ok(mut db) = self.connection.lock() {
            db.take();
        }
    }
}

/// What a storage method may use: the open database, the host (for logs) and the core's
/// configuration (the data directory and an earlier release's database, for the one-time imports).
pub struct StoreContext<'a> {
    pub db: &'a Connection,
    pub host: &'a dyn Host,
    pub config: &'a Config,
}

/// Open `kchess.db` as `db.ts` did: WAL, foreign keys, NORMAL sync, memory temp store, a 20 MB
/// cache, migrations, then 0600 permissions.
pub fn open(path: &Path, host: &dyn Host) -> Result<Connection> {
    let db = Connection::open(path).map_err(|e| CoreError::new(e.to_string()))?;
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -20000;",
    )
    .map_err(|e| CoreError::new(e.to_string()))?;
    migrations::migrate(&db)?;
    restrict(path, host);
    Ok(db)
}

#[cfg(unix)]
fn restrict(path: &Path, host: &dyn Host) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(cause) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        host.log(Level::Debug, "db", &format!("chmod failed: {cause}"));
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _host: &dyn Host) {}

type Dispatch = fn(&StoreContext, &str, &[Value]) -> Option<Result<Value>>;

/// Every storage method, `store.<module>.<name>`: each module dispatches its own.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Result<Value> {
    match method {
        // The database is opened and migrated before any call reaches this dispatch.
        "store.open" => return Ok(Value::Null),
        "store.debug.tables" if debug::enabled() => return debug::tables(ctx.db, args),
        "store.debug.exec" if debug::enabled() => return debug::exec(ctx.db, args),
        "store.debug.query" if debug::enabled() => return debug::query(ctx.db, args),
        _ => {}
    }
    let modules: [Dispatch; 9] = [
        migrations::call,
        games::call,
        library::call,
        reviews::call,
        runs::call,
        insights::call,
        usage::call,
        voice_log::call,
        lookups::call,
    ];
    modules
        .iter()
        .find_map(|call| call(ctx, method, args))
        .unwrap_or_else(|| Err(CoreError::new(format!("Unknown core method {method}."))))
}
