//! The core's API as `CoreApi` declares it (`kchess_contracts::core_api`), one file per area,
//! and what the methods share: the closed state, migration, the settings that
//! stop engines, account sessions and the close sequence. The generated dispatch checks the
//! arguments, refuses assistance during a live game and reports the profile areas a call changed.

use kchess_contracts::CoreEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use kchess_contracts::core_api::{Assist, CoreApi};
use kchess_contracts::types::{AppData, Assistance, ProfileArea, ProfileChange, Settings};
use std::sync::Arc;

use crate::core::Core;
use crate::host::{Host, Level};
use crate::store::{self, import};
use crate::tasks::lock;
use kchess_contracts::error::{CoreError, Result};

mod accounts;
mod ai;
mod chesscom;
mod engine;
mod gamedb;
mod games;
mod library;
mod online;
mod puzzles;
mod settings;
mod social;
mod spectate;
mod studies;
mod system;
pub(crate) mod tournaments;
mod voice;

/// The error of a closed core (`The KChess core is closed.`).
pub const CLOSED: &str = "The KChess core is closed.";

/// The puzzle database `puzzleDbInstall` downloads.
pub(crate) const PUZZLE_DB_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

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

    fn emit(&self, event: CoreEvent) {
        if matches!(event, CoreEvent::HostRequest(_)) || !self.closed.load(Ordering::SeqCst) {
            self.inner.emit(event);
        }
    }

    fn emit_to(&self, connection: u64, event: CoreEvent) {
        if matches!(event, CoreEvent::HostRequest(_)) || !self.closed.load(Ordering::SeqCst) {
            self.inner.emit_to(connection, event);
        }
    }
}

impl CoreApi for Core {
    fn assistance(&self) -> Assistance {
        self.services.online().assistance()
    }

    fn refused_arguments(&self, method: &str, cause: &str) {
        self.host.log(
            Level::Warn,
            "core",
            &format!("Invalid method arguments: {method} {cause}"),
        );
    }

    fn changed(&self, areas: &'static [ProfileArea]) {
        // New games or accounts: the automatic reviews look for work again.
        if areas
            .iter()
            .any(|area| matches!(area, ProfileArea::LichessGames | ProfileArea::Accounts))
        {
            self.reviews.reviews_changed();
        }
        // An account connected or signed out: its games are watched, or no longer.
        if areas.contains(&ProfileArea::Accounts) {
            let online = self.services.online().clone();
            let host = Arc::clone(&self.host);
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return;
            };
            runtime.spawn(async move {
                if let Err(cause) = online.watch_accounts().await {
                    host.log(
                        Level::Warn,
                        "online",
                        &format!("The accounts to watch could not be read: {}", cause.message),
                    );
                }
            });
        }
        self.host.emit(CoreEvent::ProfileChanged(ProfileChange {
            areas: areas.to_vec(),
            origin: crate::connection::current_or_app(),
        }));
    }
}

impl Core {
    /// Migrates an earlier release's profile once, before the first call reads the store. The
    /// plaintext tokens of an earlier database are encrypted with the OS store here.
    pub(crate) async fn ensure_migrated(&self) -> Result<()> {
        let migration = Migration {
            host: Arc::clone(&self.host),
            capabilities: Arc::clone(&self.capabilities),
            database: Arc::clone(&self.database),
            running: Arc::clone(&self.migrating),
        };
        self.migrated
            .get_or_try_init(|| async {
                // On its own task: a caller cancelled during a keychain prompt (its window
                // closed) stopped it after the accounts were committed and before their logins
                // were, and the import, no longer pending, never ran again.
                self.tracker
                    .spawn(migration.run())
                    .await
                    .unwrap_or_else(|_| Err(CoreError::new("The profile import stopped.")))
            })
            .await
            .map(|_| ())
    }

    /// The saved settings.
    pub(crate) async fn get_settings(&self) -> Result<Settings> {
        self.database
            .read_async(|ctx| store::games::get_settings(ctx.db))
            .await
    }

    /// Settings, accounts and the library size.
    pub(crate) async fn app_data(&self) -> Result<AppData> {
        self.database
            .read_async(|ctx| store::games::load_app_data(ctx.db))
            .await
    }

    /// The Lichess accounts that are signed in, as names; `username` restricts the list.
    pub(crate) async fn connected_accounts(&self, username: Option<&str>) -> Result<Vec<String>> {
        let accounts = self.services.lichess().connected_accounts().await?;
        Ok(accounts
            .into_iter()
            .filter(|name| {
                username.is_none_or(|wanted| name.to_lowercase() == wanted.to_lowercase())
            })
            .collect())
    }

    /// Ends the sessions of the signed-in accounts (`username`, or every one): their login, live
    /// play and all their work. Returns the accounts it ended.
    pub(crate) async fn end_sessions(&self, username: Option<&str>) -> Result<Vec<String>> {
        let accounts = self.connected_accounts(username).await?;
        self.services
            .online()
            .logout(username.is_some().then_some(accounts.as_slice()));
        self.end_account_work(&accounts, true).await?;
        Ok(accounts)
    }

    /// Ends what these accounts have running: syncs, reviews, challenges, usage and joined
    /// tournaments. `signed_out` also ends the login in progress.
    pub(crate) async fn end_account_work(
        &self,
        accounts: &[String],
        signed_out: bool,
    ) -> Result<()> {
        let lichess = self.services.lichess();
        if signed_out {
            lichess.invalidate_login(accounts);
        } else {
            lichess.cancel_account_syncs(accounts);
        }
        self.reviews.discard_account_reviews(accounts);
        self.usage.flush().await;
        self.services.forget_usage(accounts);
        for name in accounts {
            self.services.online().challenges().forget(name);
        }
        // Tournaments joined as a signed-out account would otherwise reopen its event streams.
        self.forget_tournaments_of(accounts.to_vec()).await
    }

    /// Signs `username` (or every account) out: sessions, logins kept by Lichess and the stored
    /// connection.
    pub(crate) async fn sign_out(&self, username: Option<String>) -> Result<AppData> {
        self.end_sessions(username.as_deref()).await?;
        self.services
            .lichess()
            .revoke_logins(username.as_ref().map(std::slice::from_ref))
            .await;
        self.logout_accounts(username.as_deref()).await
    }

    /// Forgets the tournaments these accounts joined.
    pub(crate) async fn forget_tournaments_of(&self, accounts: Vec<String>) -> Result<()> {
        self.database
            .write_async(move |ctx| {
                store::library::forget_tournaments_of(ctx.host, ctx.db, &accounts)
            })
            .await
    }

    /// Settings changed: store them, and stop what the new settings change.
    pub(crate) async fn persist_settings(&self, settings: Settings) -> Result<Settings> {
        let previous = self.get_settings().await?;
        // Only a path the user picked (`trustEnginePath`, sent when they choose one in a file
        // dialog), downloaded by KChess, or already saved is accepted, so a stray or confused
        // settings write cannot point the core at an executable. It is no boundary against a
        // client holding the profile's `core.token`: that client is this user, trusted like them
        // (`docs/shared-core.md`).
        let path = settings.core.engine_path.as_str();
        if !path.is_empty()
            && previous.core.engine_path != path
            && !self.search.is_trusted_engine_path(&PathBuf::from(path))
        {
            return Err(CoreError::new(
                "Choose the Stockfish executable with the file picker.",
            ));
        }
        let saved = self
            .database
            .write_async(move |ctx| store::games::save_settings(ctx.db, &settings))
            .await?;
        if previous.core.engine_path != saved.core.engine_path {
            crate::engine::stop_engines(&self.search, &self.analysis, &self.reviews, true);
        }
        self.host.emit(CoreEvent::SettingsSaved(saved.core.clone()));
        self.reviews.reviews_changed();
        Ok(saved)
    }

    /// An engine, review or explorer call refused while help is not lent, saying why.
    pub(crate) fn assistance_allowed(&self, gate: Assist) -> Result<()> {
        match self.services.online().assistance() {
            Assistance::Allowed => Ok(()),
            why => Err(gate.refused(why)),
        }
    }

    /// Whether `name` is a signed-in account.
    pub(crate) async fn is_connected(&self, name: &str) -> Result<bool> {
        Ok(!self.connected_accounts(Some(name)).await?.is_empty())
    }

    /// Signs `username` (or every account) out in the store, once its sessions have ended.
    pub(crate) async fn logout_accounts(&self, username: Option<&str>) -> Result<AppData> {
        let username = username.map(str::to_string);
        self.database
            .write_async(move |ctx| store::games::logout_accounts(ctx.db, username.as_deref()))
            .await
    }

    /// A request id for a managed engine install or delete (the host cancels it by id).
    pub(crate) fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Runs `cleanup` when the core closes, while it still holds its profile's lock: the
    /// service removes its socket and discovery files there, which a successor that took the
    /// lock first would already have replaced (2026-11-23).
    pub fn before_release(&self, cleanup: impl FnOnce() + Send + 'static) {
        lock(&self.before_release).push(Box::new(cleanup));
    }

    /// Closes the core: cancels its work, waits for its calls and releases its files.
    /// Later calls fail with `The KChess core is closed.`.
    pub async fn close(&self) {
        self.closing.get_or_init(|| self.shutdown()).await;
    }

    async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.ai.lifetime().cancel();
        // A renewal OpenAI is answering ends with its save, while the keychain and the database
        // are still open: its old refresh token is used up.
        self.ai.close_renewals();
        self.ai.renewals.wait().await;
        self.services.spectator().stop();
        self.services.online().close();
        self.analysis.stop_analysis(true);
        // Stops and closes the computer's engine too.
        self.search.reset_engine();
        self.reviews.stop_reviews().await;
        self.voice.stop().await;
        // A model being prepared stops writing the voice folder before the profile is released.
        self.voice_model.shutdown().await;
        // Likewise a tablebase download, writing the tablebases folder.
        self.tablebases.shutdown().await;
        self.usage.flush().await;
        // Cancel what is in flight: installs, pending host requests, lookups, downloads, streams.
        self.capabilities.close();
        self.battery.close();
        self.cancel_installs();
        self.puzzles.cancel();
        self.gamedb.stop();
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
        self.gamedb.close().await;
        self.database.close_async().await;
        let before_release = std::mem::take(&mut *lock(&self.before_release));
        for cleanup in before_release {
            cleanup();
        }
        self.claim.release();
    }
}

/// The import of an earlier release's profile, as its own task runs it (`ensure_migrated`).
struct Migration {
    host: Arc<dyn Host>,
    capabilities: Arc<crate::capabilities::Capabilities>,
    database: Arc<crate::store::Database>,
    running: Arc<tokio::sync::Mutex<()>>,
}

impl Migration {
    async fn run(self) -> Result<()> {
        let _running = self.running.lock().await;
        self.database.write_async(import::migrate).await?;
        // The earlier database's logins, recorded by its import, are sealed here: on the start
        // that imported them, and on a later one when the keychain refused or the start ended.
        self.retry_deferred().await
    }

    /// Seals the waiting logins of the earlier database and reconnects their accounts; a login
    /// the keychain refuses waits for the next start (2026-11-23: one refusal once lost it).
    async fn retry_deferred(&self) -> Result<()> {
        let logins = self.database.write_async(import::due_legacy_logins).await?;
        let mut done = logins.done;
        // Asking can show a keychain prompt: only when a login waits.
        if !logins.due.is_empty() && self.capabilities.secrets_available().await? {
            for entry in logins.due {
                let Some(account) = entry.account else {
                    continue;
                };
                let restored = match self.capabilities.encrypt(&entry.token).await {
                    Ok(encrypted) => {
                        let owner = account.clone();
                        self.database
                            .write_async(move |ctx| {
                                import::restore_legacy_login(ctx.db, &owner, &encrypted)
                            })
                            .await
                    }
                    Err(cause) => Err(cause),
                };
                match restored {
                    Ok(()) => done.push(Some(account)),
                    Err(cause) => self.host.log(
                        Level::Warn,
                        "store",
                        &format!(
                            "Legacy login of {account} not sealed yet: {}",
                            cause.message
                        ),
                    ),
                }
            }
        }
        self.scrub(done).await
    }

    /// The earlier database's plaintext copies of these logins go, sealed or given up on; once
    /// gone they are no longer recorded, and a removal that failed is tried on the next start.
    async fn scrub(&self, accounts: Vec<Option<String>>) -> Result<()> {
        if accounts.is_empty() {
            return Ok(());
        }
        let removed = accounts.clone();
        let gone = self
            .database
            .read_async(move |ctx| import::scrub_legacy_tokens(ctx, &removed))
            .await?;
        if gone {
            self.database
                .write_async(move |ctx| import::forget_legacy_logins(ctx.db, &accounts))
                .await?;
        }
        Ok(())
    }
}
