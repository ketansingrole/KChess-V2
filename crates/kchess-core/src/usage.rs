//! Network accounting batches: what the Lichess policy and the downloads count (`Meter`)
//! accumulates in memory per account and kind, and reaches `kchess.db` at most every five
//! seconds, or on demand before a report. Traffic of an account whose epoch moved
//! on is dropped by the policy before it is counted; `forget` drops what is still pending for
//! those accounts.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[cfg(test)]
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::host::{Host, Level};
use crate::store::{self, Database, usage::UsageRow};
use kchess_contracts::error::Result;
use kchess_contracts::types::UsageReport;

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
    /// This batch, for the flush timer it starts.
    me: Weak<UsageBatch>,
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
        Arc::new_cyclic(|me| UsageBatch {
            me: me.clone(),
            database,
            host,
            data_dir,
            pending: Mutex::new(Pending::default()),
        })
    }

    /// Counts `requests` and `bytes` for `account` and `kind`. The first
    /// count starts the flush timer; without a runtime the counters are written at once.
    pub fn record(&self, account: &str, kind: &str, requests: i64, bytes: i64) {
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
                    if pending.timer.is_none()
                        && let Some(this) = self.me.upgrade()
                    {
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

    fn take_rows(&self) -> Vec<UsageRow> {
        std::mem::take(&mut lock(&self.pending).cells)
            .into_iter()
            .map(|((account, kind), cell)| UsageRow {
                account,
                kind,
                requests: cell.requests,
                bytes_in: cell.bytes_in,
            })
            .collect()
    }

    fn flush_sync(&self) {
        let rows = self.take_rows();
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        self.log_flush(
            self.database
                .write(|ctx| store::usage::flush_usage(ctx.db, &rows)),
            count,
        );
    }

    /// Writes every pending counter. A failed write drops the batch and logs it.
    pub async fn flush(&self) {
        let rows = self.take_rows();
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        self.log_flush(
            self.database
                .write_async(move |ctx| store::usage::flush_usage(ctx.db, &rows))
                .await,
            count,
        );
    }

    fn log_flush(&self, result: Result<()>, count: usize) {
        if let Err(cause) = result {
            self.host.log(
                Level::Warn,
                "usage",
                &format!("Dropping usage batch: {count} {}", cause.message),
            );
        }
    }

    /// Drops the pending counters of `accounts`.
    pub fn forget(&self, accounts: &[String]) {
        let mut pending = lock(&self.pending);
        pending
            .cells
            .retain(|(account, _), _| !accounts.iter().any(|name| name.to_lowercase() == *account));
    }

    /// The pending counters are written first, then the stored report with the
    /// size of the databases on disk.
    pub async fn report(&self) -> Result<UsageReport> {
        self.flush().await;
        let mut report = self.database.read_async(store::usage::usage_report).await?;
        let dir = self.data_dir.clone();
        report.db_bytes = tokio::task::spawn_blocking(move || database_bytes(&dir))
            .await
            .map_err(|cause| {
                kchess_contracts::error::CoreError::wrap("Database size check failed", cause)
            })?;
        Ok(report)
    }

    /// The pending counters and the stored ones are cleared.
    pub async fn reset(&self) -> Result<()> {
        lock(&self.pending).cells.clear();
        self.database
            .write_async(|ctx| store::usage::reset_usage(ctx.db))
            .await
    }

    /// Stops the timer and drops what is pending.
    pub fn close(&self) {
        let mut pending = lock(&self.pending);
        if let Some(timer) = pending.timer.take() {
            timer.abort();
        }
        pending.cells.clear();
    }
}

/// The size of KChess's databases on disk (the report's `dbBytes`): the main and puzzle
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

use crate::tasks::lock;
use kchess_contracts::types::UsageKind;

/// Where metered network traffic is counted: every service that downloads reports its requests
/// and bytes here.
pub trait Meter: Send + Sync {
    fn count(&self, account: &str, kind: UsageKind, requests: u64, bytes: u64);
}

/// Traffic that is not counted.
pub struct Unmetered;

impl Meter for Unmetered {
    fn count(&self, _: &str, _: UsageKind, _: u64, _: u64) {}
}

/// Tests: every count, as `{ account, category, requests, bytes }`.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct UsageLog(Mutex<Vec<Value>>);

#[cfg(test)]
impl UsageLog {
    pub(crate) fn counted(&self) -> Vec<Value> {
        lock(&self.0).clone()
    }
}

#[cfg(test)]
impl Meter for UsageLog {
    fn count(&self, account: &str, kind: UsageKind, requests: u64, bytes: u64) {
        lock(&self.0).push(
            json!({ "account": account, "category": kind, "requests": requests, "bytes": bytes }),
        );
    }
}

impl Meter for UsageBatch {
    fn count(&self, account: &str, kind: UsageKind, requests: u64, bytes: u64) {
        let kind = serde_json::to_value(kind).unwrap_or_default();
        let kind = kind.as_str().unwrap_or("other");
        self.record(account, kind, requests as i64, bytes as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_host::TestHost;

    fn batch(dir: &Path) -> Arc<UsageBatch> {
        let host: Arc<dyn Host> = TestHost::quiet();
        let database = Arc::new(Database::new(
            crate::host::Config::new(dir.to_path_buf()),
            Arc::clone(&host),
        ));
        UsageBatch::new(database, host, dir.to_path_buf())
    }

    /// The counters of one batch read back in the report, summed per account (case-insensitively)
    /// and kind.
    #[tokio::test]
    async fn counts_are_written_as_the_same_rows_per_account_and_kind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "profile", 1, 100);
        batch.record("alice", "profile", 2, 50);
        batch.record("Alice", "games", 1, 10);
        batch.record("Bob", "games", 3, 7);
        let report = json!(batch.report().await.expect("report"));
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

    /// Each account's cached responses add up to its cache storage, keyed by the account before
    /// the key's first colon.
    #[tokio::test]
    async fn cached_responses_add_up_per_account() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch
            .database
            .write(|ctx| {
                // Stored as JSON text: 5, 3 and 2 bytes.
                store::games::write_api_cache(ctx.db, "alice:profile", &json!("abc"))?;
                store::games::write_api_cache(ctx.db, "alice:games", &json!([1]))?;
                store::games::write_api_cache(ctx.db, "bob:games", &json!(12))
            })
            .expect("cache rows");
        let report = json!(batch.report().await.expect("report"));
        assert_eq!(report["storage"]["alice"]["cacheBytes"], json!(8));
        assert_eq!(report["storage"]["bob"]["cacheBytes"], json!(2));
    }

    /// Forgetting an account drops what is still pending for the account, and nothing else.
    #[tokio::test]
    async fn forgetting_an_account_drops_its_pending_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "games", 1, 10);
        batch.record("Bob", "games", 1, 20);
        batch.forget(&["ALICE".to_string()]);
        let report = json!(batch.report().await.expect("report"));
        assert!(report["accounts"].get("alice").is_none());
        assert_eq!(report["accounts"]["bob"]["total"]["bytesIn"], json!(20));
    }

    /// A reset clears the pending and the stored counters.
    #[tokio::test]
    async fn a_reset_clears_pending_and_stored_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "games", 1, 10);
        batch.flush().await;
        batch.record("Alice", "games", 1, 10);
        batch.reset().await.expect("reset");
        let report = json!(batch.report().await.expect("report"));
        assert!(
            report["accounts"]
                .as_object()
                .is_some_and(|accounts| accounts.is_empty())
        );
    }

    /// Without a runtime to time a flush, a count is written at once.
    #[test]
    fn without_a_runtime_a_count_is_written_at_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let batch = batch(dir.path());
        batch.record("Alice", "games", 2, 30);
        assert!(lock(&batch.pending).cells.is_empty());
        let report = json!(
            batch
                .database
                .read(store::usage::usage_report)
                .expect("report")
        );
        assert_eq!(
            report["accounts"]["alice"]["total"],
            json!({ "requests": 2, "bytesIn": 30 })
        );
    }
}
