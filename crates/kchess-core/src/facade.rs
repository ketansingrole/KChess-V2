//! The core's API as `CoreApi` declares it (`kchess_contracts::core_api`), one file per area,
//! and what the methods share: the closed state, migration, the settings that
//! stop engines, account sessions and the close sequence. The generated dispatch checks the
//! arguments, refuses assistance during a live game and reports the profile areas a call changed.

use kchess_contracts::CoreEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use kchess_contracts::core_api::CoreApi;
use kchess_contracts::types::{AppData, ProfileArea, Settings};
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
mod social;
mod spectate;
mod studies;
mod system;
mod tournaments;
mod voice;

/// The error of a closed core (`The KChess core is closed.`).
pub const CLOSED: &str = "The KChess core is closed.";

/// The puzzle database `puzzleDbInstall` downloads (`PUZZLE_DB_URL`).
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
        // New games or accounts: the automatic reviews look for work again.
        if areas
            .iter()
            .any(|area| matches!(area, ProfileArea::LichessGames | ProfileArea::Accounts))
        {
            self.reviews.reviews_changed();
        }
        self.host.emit(CoreEvent::ProfileChanged(
            json!({ "areas": areas, "origin": crate::connection::current_or_app() }),
        ));
    }
}

impl Core {
    /// Migrates an earlier release's profile once, before the first call reads the store. The
    /// plaintext tokens of an earlier database are encrypted with the OS store here.
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

    /// Settings changed: store them, and stop what the new settings change (`persistSettings`).
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
            self.search.stop_engine(true);
            self.analysis.stop_analysis(true);
            self.reviews.restart_review_engine();
        }
        let core = serde_json::to_value(&saved.core)
            .map_err(|cause| CoreError::wrap("core settings", cause))?;
        self.host.emit(CoreEvent::SettingsSaved(core));
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
