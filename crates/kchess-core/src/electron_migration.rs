//! What the Electron app left in a profile, taken over at the first start of a release that can.
//!
//! The renderer's `localStorage` (documents and view preferences) moves into `kchess.db`, and
//! every Lichess login Electron's `safeStorage` sealed is sealed again with the profile's own
//! credentials, including accounts not in use. Only when both succeed does the profile record
//! `electron:migrated` (the release that did it); until then each start tries again. A login is
//! replaced only by its own token sealed anew, and only while it is still the one that was read,
//! so a failure never loses it.
//!
//! It runs in the background after the core starts, never before a read: opening a
//! `safeStorage` login may wait on a keychain prompt (`core_startup.rs`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result};
use crate::facade::args::normal;
use crate::host::{Host, Level};
use crate::secrets;
use crate::store::Database;

/// The document recording the release that finished the migration.
pub const MARKER: &str = "electron:migrated";

/// What the migration needs from the core.
#[derive(Clone)]
pub struct Migration {
    database: Arc<Database>,
    capabilities: Arc<Capabilities>,
    host: Arc<dyn Host>,
    data_dir: PathBuf,
    closed: Arc<AtomicBool>,
    /// One run at a time: the background start and an explicit call never overlap.
    running: Arc<tokio::sync::Mutex<()>>,
}

/// Whether `profile` holds the Electron renderer's `localStorage`.
pub fn has_local_storage(profile: &Path) -> bool {
    profile
        .join("Local Storage")
        .join("leveldb")
        .join("CURRENT")
        .is_file()
}

/// What the Electron renderer kept in `profile`'s `localStorage`; None when it cannot be read,
/// which is logged, so nothing is marked as taken over and the next start tries again.
pub(crate) async fn local_storage(
    host: &dyn Host,
    profile: &Path,
) -> Option<BTreeMap<String, String>> {
    let profile = profile.to_path_buf();
    let read =
        tokio::task::spawn_blocking(move || kchess_electron_legacy::local_storage::read(&profile))
            .await;
    match read {
        Ok(Ok(found)) => Some(found),
        Ok(Err(cause)) => {
            host.log(
                Level::Warn,
                "library",
                &format!("The Electron app's saved data could not be read: {cause}"),
            );
            None
        }
        Err(_) => None,
    }
}

/// Take over the library documents in `found`, once (`importLibrary` keeps it to once).
pub(crate) async fn take_over_library(
    database: &Arc<Database>,
    found: &BTreeMap<String, String>,
) -> Result<Value> {
    let mut documents = serde_json::Map::new();
    for (key, text) in found {
        if kchess_domain::misc::constants::LEGACY_DOCUMENT_KEYS.contains(&key.as_str()) {
            documents.insert(key.clone(), Value::String(text.clone()));
        }
    }
    let documents = normal("assertLegacyDocuments", &Value::Object(documents))?;
    database
        .call_async("store.library.importLibrary", &[documents])
        .await
}

/// Take over the view preferences in `found`, once (`importLegacy` keeps it to once).
pub(crate) async fn take_over_preferences(
    database: &Arc<Database>,
    found: &BTreeMap<String, String>,
) -> Result<Value> {
    let texts: serde_json::Map<String, Value> = found
        .iter()
        .filter(|(key, _)| {
            !kchess_domain::misc::constants::LEGACY_DOCUMENT_KEYS.contains(&key.as_str())
        })
        .map(|(key, text)| (key.clone(), Value::String(text.clone())))
        .collect();
    database
        .call_async(
            "store.viewPreferences.importLegacy",
            &[Value::Object(texts)],
        )
        .await
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An earlier start finished it.
    AlreadyDone,
    /// The profile holds nothing from the Electron app.
    NothingToDo,
    /// Everything was taken over; the marker is written.
    Migrated,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::AlreadyDone => "alreadyDone",
            Outcome::NothingToDo => "nothingToDo",
            Outcome::Migrated => "migrated",
        }
    }
}

impl Migration {
    /// The migration of `data_dir`'s profile over its database and capabilities; it stops
    /// before its next write once `closed` is set.
    pub fn new(
        database: Arc<Database>,
        capabilities: Arc<Capabilities>,
        host: Arc<dyn Host>,
        data_dir: PathBuf,
        closed: Arc<AtomicBool>,
    ) -> Migration {
        Migration {
            database,
            capabilities,
            host,
            data_dir,
            closed,
            running: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Run the migration unless it is done. An error leaves the marker unwritten.
    pub async fn run(&self) -> Result<Outcome> {
        let _one = self.running.lock().await;
        if self.done().await? {
            return Ok(Outcome::AlreadyDone);
        }
        let sealed = self.host_sealed().await?;
        let local = has_local_storage(&self.data_dir);
        if sealed.is_empty() && !local {
            return Ok(Outcome::NothingToDo);
        }
        let mut failures = Vec::new();
        if local && let Err(cause) = self.take_over_local_storage().await {
            failures.push(cause.message);
        }
        if let Err(cause) = self.reseal(sealed).await {
            failures.push(cause.message);
        }
        if !failures.is_empty() {
            return Err(CoreError::new(failures.join(" ")));
        }
        self.refuse_closed()?;
        self.database
            .call_async(
                "store.electron.markMigrated",
                &[json!(env!("CARGO_PKG_VERSION"))],
            )
            .await?;
        self.host.log(
            Level::Info,
            "migration",
            "Took over what the Electron app left in this profile.",
        );
        Ok(Outcome::Migrated)
    }

    /// The background run at startup: logs its outcome, never its secrets.
    pub(crate) async fn run_logged(self) {
        if let Err(cause) = self.run().await {
            self.host.log(
                Level::Warn,
                "migration",
                &format!(
                    "The Electron app's data was not all taken over; trying again next start: {}",
                    cause.message
                ),
            );
        }
    }

    async fn done(&self) -> Result<bool> {
        Ok(!self
            .database
            .call_async("store.electron.migrated", &[])
            .await?
            .is_null())
    }

    fn refuse_closed(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::new(crate::facade::CLOSED));
        }
        Ok(())
    }

    /// `(account, ciphertext)` of every stored login an earlier host sealed.
    async fn host_sealed(&self) -> Result<Vec<(String, String)>> {
        let rows = self
            .database
            .call_async("store.games.hostSealedTokens", &[])
            .await?;
        Ok(rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row["account"].as_str()?.to_string(),
                    row["encrypted"].as_str()?.to_string(),
                ))
            })
            .collect())
    }

    async fn take_over_local_storage(&self) -> Result<()> {
        let Some(found) = local_storage(self.host.as_ref(), &self.data_dir).await else {
            return Err(CoreError::new(
                "The Electron app's saved data could not be read.",
            ));
        };
        self.refuse_closed()?;
        take_over_library(&self.database, &found).await?;
        take_over_preferences(&self.database, &found).await?;
        Ok(())
    }

    /// Seal every login in `sealed` with the profile's credentials. Each is tried; any that
    /// fails keeps its original and fails the run.
    async fn reseal(&self, sealed: Vec<(String, String)>) -> Result<()> {
        if sealed.is_empty() {
            return Ok(());
        }
        if !self.capabilities.reseals() {
            return Err(CoreError::new(
                "This profile has no credentials to seal its logins with.",
            ));
        }
        let mut failed = 0usize;
        for (account, previous) in sealed {
            self.refuse_closed()?;
            let resealed = async {
                let token = self.capabilities.decrypt(&previous).await?;
                let sealed = self.capabilities.encrypt(&token).await?;
                if !secrets::is_sealed(&sealed) {
                    return Err(CoreError::new("The login was not sealed by this profile."));
                }
                self.database
                    .call_async(
                        "store.games.resealTokenCiphertext",
                        &[json!(account), json!(previous), json!(sealed)],
                    )
                    .await
            }
            .await;
            // `false`: a logout or a new login replaced it meanwhile, which is sealed already.
            if let Err(cause) = resealed {
                failed += 1;
                self.host.log(
                    Level::Warn,
                    "credentials",
                    &format!(
                        "The saved login of {account} was not resealed: {}",
                        cause.message
                    ),
                );
            }
        }
        if failed > 0 {
            return Err(CoreError::new(format!(
                "{failed} saved login(s) could not be resealed."
            )));
        }
        Ok(())
    }
}
