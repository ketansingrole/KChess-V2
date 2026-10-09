//! KChess's main database (`kchess.db`), migrating from `core/src/services/{db,migrations,store,
//! library,reviewStore,runs,insights,usage,voiceLog,setupPositionLookup}.ts`. Until every
//! module has moved, TypeScript keeps owning the file; the switch happens in one commit, since
//! two SQLite copies in one process must never hold the same file (POSIX locks are per process).

pub mod games;
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

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

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

type Dispatch = fn(&Connection, &str, &[Value]) -> Option<Result<Value>>;

/// Every storage method, `store.<module>.<name>`: each module dispatches its own.
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Result<Value> {
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
        .find_map(|call| call(db, method, args))
        .unwrap_or_else(|| Err(CoreError::new(format!("Unknown core method {method}."))))
}
