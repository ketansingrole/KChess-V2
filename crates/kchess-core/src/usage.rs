//! Network accounting batches: the `host:usage` events the Lichess
//! policy and the puzzle download emit accumulate in memory per account and kind, and reach
//! `kchess.db` at most every five seconds (`flushUsage`), or on demand before a report. Traffic of
//! an account whose epoch moved on is dropped by the policy before it is emitted; `forget` drops
//! what is still pending for those accounts.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::host::{Host, Level};
use crate::store::Database;

/// How long counters wait in memory before they are written.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);

/// The counters of one account and kind, not yet written.
#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    requests: i64,
    bytes_in: i64,
}

#[derive(Default)]
struct Pending {
    cells: BTreeMap<(String, String), Cell>,
    timer: Option<JoinHandle<()>>,
}

pub struct UsageBatch {
    database: Arc<Database>,
    host: Arc<dyn Host>,
    data_dir: std::path::PathBuf,
    pending: Mutex<Pending>,
}

impl UsageBatch {
    pub fn new(
        database: Arc<Database>,
        host: Arc<dyn Host>,
        data_dir: std::path::PathBuf,
    ) -> Arc<UsageBatch> {
        Arc::new(UsageBatch {
            database,
            host,
            data_dir,
            pending: Mutex::new(Pending::default()),
        })
    }

    /// Counts `requests` and `bytes` for `account` and `kind` (`recordUsage`'s batching). The first
    /// count starts the flush timer; without a runtime the counters are written at once.
    pub fn record(self: &Arc<Self>, account: &str, kind: &str, requests: i64, bytes: i64) {
        let immediate = {
            let mut pending = lock(&self.pending);
            let cell = pending
                .cells
                .entry((account.to_lowercase(), kind.to_string()))
                .or_default();
            cell.requests += requests;
            cell.bytes_in += bytes;
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    if pending.timer.is_none() {
                        let this = Arc::clone(self);
                        pending.timer = Some(handle.spawn(async move {
                            tokio::time::sleep(FLUSH_EVERY).await;
                            lock(&this.pending).timer = None;
                            this.flush().await;
                        }));
                    }
                    false
                }
                Err(_) => true,
            }
        };
        if immediate {
            self.flush_sync();
        }
    }

    /// Writes every pending counter (`flushUsage`). A failed write drops the batch and logs it.
    fn take_rows(&self) -> Vec<Value> {
        {
            let mut pending = lock(&self.pending);
            if pending.cells.is_empty() {
                return Vec::new();
            }
            std::mem::take(&mut pending.cells)
                .into_iter()
                .map(|((account, kind), cell)| {
                    json!({
                        "account": account,
                        "kind": kind,
                        "requests": cell.requests,
                        "bytesIn": cell.bytes_in,
                    })
                })
                .collect()
        }
    }

    fn flush_sync(&self) {
        let rows = self.take_rows();
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        self.log_flush(
            self.database
                .call("store.usage.flushUsage", &[Value::Array(rows)]),
            count,
        );
    }

    pub async fn flush(&self) {
        let rows = self.take_rows();
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        self.log_flush(
            self.database
                .call_async("store.usage.flushUsage", &[Value::Array(rows)])
                .await,
            count,
        );
    }

    fn log_flush(&self, result: Result<Value>, count: usize) {
        if let Err(cause) = result {
            self.host.log(
                Level::Warn,
                "usage",
                &format!("Dropping usage batch: {count} {}", cause.message),
            );
        }
    }

    /// Drops the pending counters of `accounts` (`forgetUsage`).
    pub fn forget(&self, accounts: &[String]) {
        let mut pending = lock(&self.pending);
        pending
            .cells
            .retain(|(account, _), _| !accounts.iter().any(|name| name.to_lowercase() == *account));
    }

    /// `usageReport`: the pending counters are written first, then the stored report with the
    /// size of the databases on disk.
    pub async fn report(&self) -> Result<Value> {
        self.flush().await;
        let report = self
            .database
            .call_async("store.usage.usageReport", &[])
            .await?;
        let mut object: Map<String, Value> = match report {
            Value::Object(object) => object,
            _ => Map::new(),
        };
        let dir = self.data_dir.clone();
        let db_bytes = tokio::task::spawn_blocking(move || database_bytes(&dir))
            .await
            .map_err(|cause| crate::error::CoreError::wrap("Database size check failed", cause))?;
        object.insert("dbBytes".into(), json!(db_bytes));
        Ok(Value::Object(object))
    }

    /// `resetUsage`: the pending counters and the stored ones are cleared.
    pub async fn reset(&self) -> Result<Value> {
        lock(&self.pending).cells.clear();
        self.database
            .call_async("store.usage.resetUsage", &[])
            .await
    }

    /// `closeUsage`: stops the timer and drops what is pending.
    pub fn close(&self) {
        let mut pending = lock(&self.pending);
        if let Some(timer) = pending.timer.take() {
            timer.abort();
        }
        pending.cells.clear();
    }
}

/// The size of KChess's databases on disk (`usageReport`'s `dbBytes`): the main and puzzle
/// databases with their write-ahead logs. A missing file counts as zero.
fn database_bytes(data_dir: &Path) -> u64 {
    ["kchess.db", "kchess.db-wal", "puzzles.db", "puzzles.db-wal"]
        .iter()
        .map(|name| {
            std::fs::metadata(data_dir.join(name))
                .map(|meta| meta.len())
                .unwrap_or(0)
        })
        .sum()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// The host the core's services use: `host:usage` events go to the batch, everything else to the
/// platform host unchanged.
pub struct UsageHost {
    pub inner: Arc<dyn Host>,
    pub batch: Arc<UsageBatch>,
}

impl Host for UsageHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.inner.log(level, scope, message);
    }

    fn emit(&self, event: &str, payload: Value) {
        if event != "host:usage" {
            self.inner.emit(event, payload);
            return;
        }
        let account = payload.get("account").and_then(Value::as_str).unwrap_or("");
        let kind = payload
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("other");
        let requests = payload.get("requests").and_then(Value::as_i64).unwrap_or(0);
        let bytes = payload.get("bytes").and_then(Value::as_i64).unwrap_or(0);
        self.batch.record(account, kind, requests, bytes);
    }

    fn emit_to(&self, connection: u64, event: &str, payload: Value) {
        if event == "host:usage" {
            self.emit(event, payload);
        } else {
            self.inner.emit_to(connection, event, payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;

    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    fn batch(dir: &Path) -> Arc<UsageBatch> {
        let host: Arc<dyn Host> = Arc::new(Quiet);
        let database = Arc::new(Database::new(
            crate::host::Config::new(dir.to_path_buf()),
            Arc::clone(&host),
        ));
        UsageBatch::new(database, host, dir.to_path_buf())
    }

    /// The counters of one batch read back as `usageReport` reports them: the same rows the
    /// TypeScript batch wrote, summed per account (case-insensitively) and kind.
    #[tokio::test]
    async fn counts_are_written_as_the_same_rows_per_account_and_kind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "profile", 1, 100);
        batch.record("alice", "profile", 2, 50);
        batch.record("Alice", "games", 1, 10);
        batch.record("Bob", "games", 3, 7);
        let report = batch.report().await.expect("report");
        let alice = &report["accounts"]["alice"];
        assert_eq!(alice["total"], json!({ "requests": 4, "bytesIn": 160 }));
        assert_eq!(
            alice["byKind"]["profile"],
            json!({ "requests": 3, "bytesIn": 150 })
        );
        assert_eq!(
            alice["byKind"]["games"],
            json!({ "requests": 1, "bytesIn": 10 })
        );
        assert_eq!(
            report["accounts"]["bob"]["total"],
            json!({ "requests": 3, "bytesIn": 7 })
        );
        assert!(report["dbBytes"].as_u64().is_some());
    }

    /// `forgetUsage` drops what is still pending for the account, and nothing else.
    #[tokio::test]
    async fn forgetting_an_account_drops_its_pending_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "games", 1, 10);
        batch.record("Bob", "games", 1, 20);
        batch.forget(&["ALICE".to_string()]);
        let report = batch.report().await.expect("report");
        assert!(report["accounts"].get("alice").is_none());
        assert_eq!(report["accounts"]["bob"]["total"]["bytesIn"], json!(20));
    }

    /// `resetUsage` clears the pending and the stored counters.
    #[tokio::test]
    async fn a_reset_clears_pending_and_stored_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "games", 1, 10);
        batch.flush().await;
        batch.record("Alice", "games", 1, 10);
        batch.reset().await.expect("reset");
        let report = batch.report().await.expect("report");
        assert!(
            report["accounts"]
                .as_object()
                .is_some_and(|accounts| accounts.is_empty())
        );
    }
}
