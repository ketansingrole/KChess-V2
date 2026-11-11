//! The core's API as `CoreApi` declares it (`kchess_contracts::core_api`), one file per area,
//! and what the methods share: the profile claim, the closed state, migration, the settings that
//! stop engines, account sessions and the close sequence. The generated dispatch checks the
//! arguments, refuses assistance during a live game and reports the profile areas a call changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kchess_contracts::core_api::CoreApi;
use kchess_contracts::types::{CoreSettings, ProfileArea};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use crate::store::{self, import};

mod accounts;
mod ai;
pub(crate) mod args;
mod chesscom;
mod engine;
mod games;
mod library;
mod online;
mod puzzles;
mod settings;
mod spectate;
mod studies;
mod system;
mod tournaments;
mod voice;

/// The error of a closed core (`The KChess core is closed.`).
pub const CLOSED: &str = "The KChess core is closed.";

/// The puzzle database `puzzleDbInstall` downloads (`PUZZLE_DB_URL`).
pub(crate) const PUZZLE_DB_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

/// The profiles in use by a live core, by canonical data directory; each holds the claim's token,
/// so a stale release never frees a profile a later core has claimed.
static PROFILES: LazyLock<Mutex<HashMap<PathBuf, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLAIMS: AtomicU64 = AtomicU64::new(1);

/// The lock file a live core holds in its profile directory: one live core per
/// profile, even across processes (a second app launch, the CLI on the same data dir).
const PROFILE_LOCK: &str = "profile.lock";

/// One core's claim on its profile directory: one live core per profile (`createKChessCore`).
pub struct ProfileClaim {
    path: PathBuf,
    token: u64,
    /// The lock file, held open: `release` (or drop) frees the OS advisory lock.
    lock: Mutex<Option<std::fs::File>>,
}

impl ProfileClaim {
    /// Claims the profile at `data_dir` (its real path), or fails when a core already runs there.
    pub fn claim(data_dir: &Path) -> Result<ProfileClaim> {
        let path = std::fs::canonicalize(data_dir)
            .map_err(|cause| CoreError::wrap("profile path", cause))?;
        let token = CLAIMS.fetch_add(1, Ordering::SeqCst);
        let mut profiles = PROFILES
            .lock()
            .map_err(|_| CoreError::new("The profiles are unavailable."))?;
        if profiles.contains_key(&path) {
            return Err(CoreError::new(
                "A KChess core is already running for this profile.",
            ));
        }
        // The in-process map cannot see a second process, so the claim also holds an OS
        // advisory lock on a lock file in the data dir for the core's lifetime (`flock` on
        // Unix, `LockFileEx` on Windows: both conflict across open file descriptions, which
        // is exactly how a second process opens the same file).
        let lock = std::fs::File::create(path.join(PROFILE_LOCK))
            .map_err(|cause| CoreError::wrap("profile lock", cause))?;
        if let Err(cause) = lock.try_lock() {
            return Err(match cause {
                std::fs::TryLockError::WouldBlock => {
                    CoreError::new("A KChess core is already running for this profile.")
                }
                std::fs::TryLockError::Error(locked) => CoreError::wrap("profile lock", locked),
            });
        }
        profiles.insert(path.clone(), token);
        Ok(ProfileClaim {
            path,
            token,
            lock: Mutex::new(Some(lock)),
        })
    }

    /// Frees the profile, once the core has closed it.
    pub fn release(&self) {
        if let Ok(mut profiles) = PROFILES.lock()
            && profiles.get(&self.path) == Some(&self.token)
        {
            profiles.remove(&self.path);
        }
        // Free the OS lock now, not at drop: a closed core must be reopenable
        // while its (shut down) object still exists.
        if let Ok(mut held) = self.lock.lock()
            && let Some(file) = held.take()
        {
            let _ = file.unlock();
        }
    }
}

impl Drop for ProfileClaim {
    fn drop(&mut self) {
        self.release();
    }
}

/// The host of a core: its events stop reaching the frontend once the core is closed. Capability
/// requests still pass, so a call that is closing can be answered; logs always pass.
pub struct GatedHost {
    inner: Arc<dyn Host>,
    closed: Arc<AtomicBool>,
}

impl GatedHost {
    pub fn new(inner: Arc<dyn Host>, closed: Arc<AtomicBool>) -> GatedHost {
        GatedHost { inner, closed }
    }
}

impl Host for GatedHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.inner.log(level, scope, message);
    }

    fn emit(&self, event: &str, payload: Value) {
        if event == "host:request" || !self.closed.load(Ordering::SeqCst) {
            self.inner.emit(event, payload);
        }
    }

    fn emit_to(&self, connection: u64, event: &str, payload: Value) {
        if event == "host:request" || !self.closed.load(Ordering::SeqCst) {
            self.inner.emit_to(connection, event, payload);
        }
    }
}

/// The core's settings as a headless client sees them (`coreSettings`).
pub fn core_settings(settings: &Value) -> Result<Value> {
    let core: CoreSettings = serde_json::from_value(settings.clone())
        .map_err(|cause| CoreError::wrap("core settings", cause))?;
    serde_json::to_value(core).map_err(|cause| CoreError::wrap("core settings", cause))
}

/// The data with its settings reduced to the core's (`coreData`).
pub fn core_data(data: &Value) -> Result<Value> {
    let mut out = data.as_object().cloned().unwrap_or_default();
    out.insert(
        "settings".into(),
        core_settings(data.get("settings").unwrap_or(&Value::Null))?,
    );
    Ok(Value::Object(out))
}

/// The event a write to the profile reports (`ProfileChange` in `kchess-contracts`).
pub const PROFILE_CHANGED: &str = "profile:changed";

impl CoreApi for Core {
    fn assistance_blocked(&self) -> bool {
        self.services.online().assistance_blocked()
    }

    fn refused_arguments(&self, method: &str, cause: &str) {
        self.host.log(
            Level::Warn,
            "core",
            &format!("Invalid method arguments: {method} {cause}"),
        );
    }

    fn changed(&self, areas: &'static [ProfileArea]) {
        self.host.emit(
            PROFILE_CHANGED,
            json!({ "areas": areas, "origin": crate::connection::current_or_app() }),
        );
    }
}

impl Core {
    /// Migrates an earlier release's profile once, before the first store read (`ensureMigrated`).
    /// The plaintext tokens of an earlier database are encrypted with the OS store here.
    pub(crate) async fn ensure_migrated(&self) -> Result<()> {
        self.migrated
            .get_or_try_init(|| async { self.migrate().await })
            .await
            .map(|_| ())
    }

    pub(crate) async fn migrate(&self) -> Result<()> {
        // Asking the OS secret store can show a keychain prompt: only when tokens may be imported.
        let pending = self
            .database
            .read_async(import::legacy_import_pending)
            .await?;
        let available = pending && self.capabilities.secrets_available().await?;
        let legacy = self
            .database
            .write_async(move |ctx| import::migrate(ctx, available))
            .await?;
        if !available {
            return Ok(());
        }
        let imported = !legacy.is_empty();
        for entry in legacy {
            let account = entry.account;
            let stored = match self.capabilities.encrypt(&entry.token).await {
                Ok(encrypted) => {
                    let owner = account.clone();
                    self.database
                        .write_async(move |ctx| {
                            store::games::import_token_ciphertext(
                                ctx.db,
                                owner.as_deref(),
                                &encrypted,
                            )
                        })
                        .await
                }
                Err(cause) => Err(cause),
            };
            if let Err(cause) = stored {
                // The accounts row was already committed as connected: disconnect it
                // so a per-account encrypt failure does not leave a silent logged-out
                // state that `legacyImportPending` (now false) would never retry. Its
                // imported games stay: a logout would delete them (and with no account
                // name, every account's).
                let name = account.clone().unwrap_or_else(|| "?".into());
                self.host.log(
                    Level::Warn,
                    "store",
                    &format!("Legacy migration failed for {name}: {}", cause.message),
                );
                if let Some(account) = account {
                    let disconnected = self
                        .database
                        .write_async(move |ctx| store::games::disconnect_account(ctx.db, &account))
                        .await;
                    if let Err(cause) = disconnected {
                        self.host.log(
                            Level::Warn,
                            "store",
                            &format!("Could not disconnect {name}: {}", cause.message),
                        );
                    }
                }
            }
        }
        // Every login the earlier database held is sealed here now (or its account was
        // disconnected), so its plaintext copies go.
        if imported {
            self.database
                .read_async(import::scrub_legacy_tokens)
                .await?;
        }
        Ok(())
    }

    /// The saved settings (`getSettings`).
    pub(crate) async fn get_settings(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database
            .read_async(|ctx| store::games::get_settings(ctx.db))
            .await
    }

    /// Settings, accounts and the library size, once the profile is migrated.
    pub(crate) async fn app_data(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.stored_app_data().await
    }

    /// Settings, accounts and the library size as stored.
    pub(crate) async fn stored_app_data(&self) -> Result<Value> {
        self.database
            .read_async(|ctx| store::games::as_json(store::games::load_app_data(ctx.db)?))
            .await
    }

    /// The Lichess accounts that are signed in, as names; `username` restricts the list.
    pub(crate) async fn connected_accounts(&self, username: Option<&str>) -> Result<Vec<String>> {
        let data = self.app_data().await?;
        Ok(data["accounts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|account| account["connected"].as_bool() == Some(true))
            .filter_map(|account| account["username"].as_str().map(str::to_string))
            .filter(|name| {
                username.is_none_or(|wanted| name.to_lowercase() == wanted.to_lowercase())
            })
            .collect())
    }

    /// Ends everything a signed-in account has running: login, syncs, live play, reviews, usage
    /// (`endSessions`). Returns the accounts it ended.
    pub(crate) async fn end_sessions(&self, username: Option<&str>) -> Result<Vec<String>> {
        let accounts = self.connected_accounts(username).await?;
        self.services.lichess().invalidate_login(&accounts);
        let online = self.services.online();
        online.logout(username.is_some().then_some(accounts.as_slice()));
        self.reviews.discard_account_reviews(&accounts);
        self.usage.flush().await;
        self.services.forget_usage(&accounts);
        for name in &accounts {
            online.challenges().forget(name);
        }
        // Tournaments joined as a signed-out account would otherwise reopen its event streams.
        self.forget_tournaments_of(accounts.clone()).await?;
        Ok(accounts)
    }

    /// Forgets the tournaments these accounts joined.
    pub(crate) async fn forget_tournaments_of(&self, accounts: Vec<String>) -> Result<()> {
        self.database
            .write_async(move |ctx| {
                store::library::forget_tournaments_of(ctx.host, ctx.db, &accounts)
            })
            .await
    }

    /// Settings changed: store them, and stop what the new settings change (`persistSettings`).
    pub(crate) async fn persist_settings(&self, settings: Value) -> Result<Value> {
        let previous = self.get_settings().await?;
        // Only a path the user picked (`trustEnginePath`, sent when they choose one in a file
        // dialog), downloaded by KChess, or already saved is accepted, so a stray or confused
        // settings write cannot point the core at an executable. It is no boundary against a
        // client holding the profile's `core.token`: that client is this user, trusted like them
        // (`docs/shared-core.md`).
        let path = settings["enginePath"].as_str().unwrap_or_default();
        let previous_path = previous["enginePath"].as_str();
        if !path.is_empty()
            && previous_path != Some(path)
            && !self.search.is_trusted_engine_path(&PathBuf::from(path))
        {
            return Err(CoreError::new(
                "Choose the Stockfish executable with the file picker.",
            ));
        }
        self.ensure_migrated().await?;
        let settings: store::games::Settings = serde_json::from_value(settings)
            .map_err(|cause| CoreError::new(format!("Invalid settings: {cause}")))?;
        let saved = self
            .database
            .write_async(move |ctx| store::games::save_settings(ctx.db, &settings))
            .await?;
        if previous["enginePath"] != saved["enginePath"] {
            self.search.stop_engine(true);
            self.analysis.stop_analysis(true);
            self.reviews.restart_review_engine();
        }
        self.host.emit("settings:saved", core_settings(&saved)?);
        self.reviews.reviews_changed();
        Ok(saved)
    }

    /// An engine, review or explorer call while a live game is in progress (`assistanceAllowed`).
    pub(crate) fn assistance_allowed(&self, message: &str) -> Result<()> {
        if self.services.online().assistance_blocked() {
            return Err(CoreError::new(message));
        }
        Ok(())
    }

    /// Whether `name` is a signed-in account.
    pub(crate) async fn is_connected(&self, name: &str) -> Result<bool> {
        Ok(!self.connected_accounts(Some(name)).await?.is_empty())
    }

    /// Logs a user's logout result after the sessions have ended (`logoutAccounts`).
    pub(crate) async fn logout_accounts(&self, username: Option<&str>) -> Result<Value> {
        self.ensure_migrated().await?;
        let username = username.map(str::to_string);
        self.database
            .write_async(move |ctx| {
                store::games::as_json(store::games::logout_accounts(ctx.db, username.as_deref())?)
            })
            .await
    }

    /// A request id for a managed engine install or delete (the host cancels it by id).
    pub(crate) fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Closes the core: cancels its work, waits for its calls and releases its files
    /// (`createCore`'s `close`). Later calls fail with `The KChess core is closed.`.
    pub async fn close(&self) {
        self.closing.get_or_init(|| self.shutdown()).await;
    }

    async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.ai.lifetime().cancel();
        self.services.spectator().stop();
        self.services.online().close();
        self.search.stop_engine(true);
        self.analysis.stop_analysis(true);
        self.search.reset_engine();
        self.reviews.stop_reviews().await;
        self.voice.stop().await;
        self.usage.flush().await;
        // Cancel what is in flight: installs, pending host requests, lookups, downloads, streams.
        self.capabilities.close();
        self.battery.close();
        self.cancel_installs();
        self.puzzles.cancel();
        self.services.abort();
        self.chesscom.abort();
        self.tracker.close();
        if let Err(cause) = self.ctx.hub.close_engines().await {
            self.host.log(
                Level::Warn,
                "core",
                &format!("Engine shutdown failed: {}", cause.message),
            );
        }
        // The stopped analysis also ends (its final `interrupted` update sent, no longer running)
        // before close returns: on a busy Windows runner it was still current after close
        // (2026-11-10). Bounded: with its engine gone it ends at once.
        let _ = tokio::time::timeout(
            crate::engine::bridge::SETTLE_DEADLINE,
            self.analysis.settled(),
        )
        .await;
        self.tracker.wait().await;
        self.services.reset();
        self.usage.close();
        self.puzzles.close().await;
        self.database.close_async().await;
        self.claim.release();
    }
}

/// `assertStudyCommand`: a restored study is decoded by the rules; other commands pass through.
pub(crate) fn study_command(shape: &Value) -> Result<Value> {
    if shape["op"].as_str() != Some("restore") {
        return Ok(shape.clone());
    }
    let study = kchess_domain::library::decode_study(&shape["study"])
        .ok_or_else(|| CoreError::new("That study cannot be restored."))?;
    Ok(json!({ "op": "restore", "study": study }))
}

/// `assertArchivedGame`: the archived game as the rules decode it.
pub(crate) fn archived_game(shape: &Value) -> Result<Value> {
    kchess_domain::library::decode_archived_game(shape)
        .ok_or_else(|| CoreError::new("This game cannot be saved."))
}
