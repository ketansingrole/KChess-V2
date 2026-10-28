//! The core's facade: every `CoreApi` method (`CORE_METHODS` in `crates/kchess-contracts/ts/core.ts`) and
//! the `KChessCore` extras, answered by the Rust core. This is what `service.ts` decided before the
//! migration: argument contracts (`kchess_domain::misc::validate_arguments`), assistance gating
//! during live games, account and login epochs, the settings that stop engines, the challenge and
//! tournament bookkeeping, the projections of settings and data, and the close sequence.
//!
//! Each method keeps the name and result of the TypeScript operation it replaces. The services it
//! calls keep their own internal names (`lichess.profile`, `store.games.loadData`, ...), reached
//! through `Core::dispatch`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{Map, Value, json};
use std::sync::Arc;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

mod accounts;
mod args;
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

use self::args::text_arg;

/// Every method the facade answers, in the order of `CORE_METHODS`, then the `KChessCore` extras.
pub const FACADE_METHODS: &[&str] = &[
    "positionLookup",
    "lichessStudies",
    "lichessStudyChapters",
    "syncLichessStudy",
    "exportToLichessStudy",
    "exportGame",
    "mastersGame",
    "openingDescription",
    "openingPopularity",
    "cloudEval",
    "loadData",
    "saveSettings",
    "addAccount",
    "logout",
    "logoutAll",
    "removeAccount",
    "syncGames",
    "gamePage",
    "gameLibraryOverview",
    "insights",
    "gameRatingHistory",
    "gamePgn",
    "cachedProfile",
    "profile",
    "ratingHistory",
    "connectLichess",
    "engineStatus",
    "installEngine",
    "deleteEngine",
    "stopEngine",
    "bestMove",
    "startAnalysis",
    "stopAnalysis",
    "reviewGet",
    "reviewRequest",
    "reviewCancel",
    "reviewStatus",
    "reviewSummaries",
    "startOnline",
    "resumeOnline",
    "cancelOnline",
    "playOnline",
    "onlineAction",
    "onlineChat",
    "sendChat",
    "stayConnected",
    "challenges",
    "acceptChallenge",
    "declineChallenge",
    "cancelChallenge",
    "ongoingGames",
    "openGame",
    "playerPerf",
    "crosstable",
    "recentGames",
    "sendMessage",
    "relation",
    "setRelation",
    "profiles",
    "leaderboard",
    "searchPlayers",
    "activity",
    "timeline",
    "importGame",
    "bookmarkedGames",
    "tvChannels",
    "watch",
    "watchBroadcast",
    "stopWatching",
    "broadcasts",
    "broadcastTour",
    "tournaments",
    "tournament",
    "joinTournament",
    "leaveTournament",
    "createTournament",
    "clearAccountData",
    "following",
    "addFriends",
    "usage",
    "resetUsage",
    "presence",
    "puzzleNext",
    "puzzleSolve",
    "puzzleDaily",
    "puzzleDashboard",
    "puzzleActivity",
    "stormDashboard",
    "puzzleDbStatus",
    "puzzleDbInstall",
    "puzzleDbCancel",
    "puzzleDbDelete",
    "localPuzzles",
    "localLadder",
    "recordPuzzleAttempt",
    "puzzleProgress",
    "saveRun",
    "runSummary",
    "clearRuns",
    "saveVoiceAttempt",
    "updateVoiceAttempt",
    "voiceHistory",
    "clearVoiceHistory",
    "voiceStart",
    "voiceAccept",
    "voiceFinish",
    "voiceStop",
    "library",
    "importLibrary",
    "studyCommand",
    "saveArchivedGame",
    "removeArchivedGame",
    "addMistakes",
    "answerMistake",
    "mistakeGames",
    "saveSession",
    "joinedTournaments",
    "recordRepertoireMiss",
    "clearRepertoireMisses",
    "viewPreferences",
    "setViewPreference",
    "customThemes",
    "settings",
    "desktopSettings",
    "saveDesktopSettings",
    "trustEnginePath",
    "voiceHistoryDocument",
    "suspend",
    "close",
];

/// The error of a closed core (`The KChess core is closed.`).
pub const CLOSED: &str = "The KChess core is closed.";

/// The puzzle database `puzzleDbInstall` downloads (`PUZZLE_DB_URL`, `crates/kchess-node/js/puzzleDb.ts`).
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

/// Whether `method` is one of the facade's (answered by `Core::facade`).
pub fn is_facade_method(method: &str) -> bool {
    FACADE_METHODS.contains(&method)
}

/// The core's settings as a headless client sees them (`coreSettings`).
pub fn core_settings(settings: &Value) -> Value {
    let mut picked = Map::new();
    for key in kchess_domain::misc::constants::CORE_SETTINGS_KEYS {
        if let Some(value) = settings.get(*key) {
            picked.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(picked)
}

/// The data with its settings reduced to the core's (`coreData`).
pub fn core_data(data: &Value) -> Value {
    let mut out = data.as_object().cloned().unwrap_or_default();
    out.insert(
        "settings".into(),
        core_settings(data.get("settings").unwrap_or(&Value::Null)),
    );
    Value::Object(out)
}

/// The event a write to the profile reports (`ProfileChange` in `kchess-contracts`).
pub const PROFILE_CHANGED: &str = "profile:changed";

/// The parts of the profile a successful call of `method` changed (`ProfileArea` names), so the
/// other windows refetch them. `saveSettings` reports `settings:saved` instead.
pub fn changed_areas(method: &str) -> &'static [&'static str] {
    match method {
        "importLibrary" => &[
            "studies",
            "playedGames",
            "drills",
            "sessions",
            "tournaments",
            "repertoire",
        ],
        "studyCommand" => &["studies"],
        "saveArchivedGame" | "removeArchivedGame" => &["playedGames"],
        "addMistakes" | "answerMistake" => &["drills"],
        "saveSession" => &["sessions"],
        "joinTournament" | "leaveTournament" | "createTournament" => &["tournaments"],
        "recordRepertoireMiss" | "clearRepertoireMisses" => &["repertoire"],
        "syncGames" => &["lichessGames", "accounts"],
        "clearAccountData" => &["lichessGames", "accounts", "usage"],
        "addAccount" | "logout" | "logoutAll" | "removeAccount" | "connectLichess"
        | "addFriends" => &["accounts"],
        "saveDesktopSettings" => &["desktopSettings"],
        "recordPuzzleAttempt"
        | "saveRun"
        | "clearRuns"
        | "puzzleSolve"
        | "puzzleDbInstall"
        | "puzzleDbDelete" => &["puzzles"],
        "saveVoiceAttempt" | "updateVoiceAttempt" | "clearVoiceHistory" => &["voiceHistory"],
        "resetUsage" => &["usage"],
        "installEngine" | "deleteEngine" => &["engine"],
        "setViewPreference" => &["viewPreferences"],
        _ => &[],
    }
}

/// What a method's result shows a client: stored data reduces to its core settings, and a
/// sign-in's data likewise. Settings and the desktop settings are returned as they are.
fn project(method: &str, result: Value) -> Value {
    if matches!(method, "close" | "desktopSettings" | "saveDesktopSettings") {
        return result;
    }
    let is_data = result.get("settings").is_some() && result.get("accounts").is_some();
    if is_data {
        return core_data(&result);
    }
    if method == "connectLichess"
        && let Some(data) = result.get("data")
    {
        let mut out = result.as_object().cloned().unwrap_or_default();
        out.insert("data".into(), core_data(data));
        return Value::Object(out);
    }
    result
}

impl Core {
    /// Answers a facade method (see `is_facade_method`): checks the closed state and the
    /// arguments, then runs it and projects its result.
    pub(crate) async fn facade(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        if method == "close" {
            self.close().await;
            return Ok(Value::Null);
        }
        // Tracked before the closed check: closing waits for every call that got this far.
        let _in_flight = self.tracker.token();
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::new(CLOSED));
        }
        if let Some(Err(cause)) = kchess_domain::misc::validate_arguments(method, &args) {
            self.host.log(
                Level::Warn,
                "core",
                &format!("Invalid method arguments: {method} {cause}"),
            );
            return Err(CoreError::new(cause));
        }
        // Boxed: the method table is one large future, too big to lay out on a worker's stack.
        let result = Box::pin(self.run_facade(method, &args)).await?;
        let areas = changed_areas(method);
        if !areas.is_empty() {
            self.host.emit(
                PROFILE_CHANGED,
                json!({ "areas": areas, "origin": crate::connection::current_or_app() }),
            );
        }
        Ok(project(method, result))
    }

    /// The facade's synchronous names (`call_sync`): the settings the desktop reads and trusts.
    pub(crate) fn facade_sync(&self, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let result = match method {
            "trustEnginePath" => {
                if self.closed.load(Ordering::SeqCst) {
                    return Some(Err(CoreError::new(CLOSED)));
                }
                self.search
                    .trust_engine_path(&PathBuf::from(text_arg(args, 0)));
                Ok(Value::Null)
            }
            "voiceHistoryDocument" => {
                if self.closed.load(Ordering::SeqCst) {
                    return Some(Err(CoreError::new(CLOSED)));
                }
                self.database
                    .call("store.voiceLog.voiceHistoryDocument", &[])
            }
            _ => return None,
        };
        Some(result)
    }

    /// The internal method `name`, with the arguments the service takes.
    pub(crate) async fn inner(&self, name: &str, args: Vec<Value>) -> Result<Value> {
        Box::pin(self.dispatch(name, args)).await
    }

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
            .call_async("store.games.legacyImportPending", &[])
            .await?
            .as_bool()
            == Some(true);
        let available = pending && self.capabilities.secrets_available().await?;
        let legacy = self
            .database
            .call_async("store.games.migrate", &[json!(available)])
            .await?;
        if !available {
            return Ok(());
        }
        for entry in legacy["legacyTokens"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let account = entry["account"].clone();
            let token = entry["token"].as_str().unwrap_or_default().to_string();
            let stored = match self.capabilities.encrypt(&token).await {
                Ok(encrypted) => self
                    .database
                    .call_async(
                        "store.games.importTokenCiphertext",
                        &[account.clone(), json!(encrypted)],
                    )
                    .await
                    .map(|_| ()),
                Err(cause) => Err(cause),
            };
            if let Err(cause) = stored {
                // The accounts row was already committed as connected: disconnect it
                // so a per-account encrypt failure does not leave a silent logged-out
                // state that `legacyImportPending` (now false) would never retry. Its
                // imported games stay: a logout would delete them (and with no account
                // name, every account's).
                let name = account.as_str().unwrap_or("?");
                self.host.log(
                    Level::Warn,
                    "store",
                    &format!("Legacy migration failed for {name}: {}", cause.message),
                );
                let _ = self
                    .database
                    .call_async("store.games.disconnectAccount", &[account])
                    .await;
            }
        }
        Ok(())
    }

    /// The saved settings (`getSettings`).
    pub(crate) async fn get_settings(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database
            .call_async("store.games.getSettings", &[])
            .await
    }

    /// Settings, accounts and the library size (`loadData`).
    pub(crate) async fn load_data(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database.call_async("store.games.loadData", &[]).await
    }

    /// The Lichess accounts that are signed in, as names; `username` restricts the list.
    pub(crate) async fn connected_accounts(&self, username: Option<&str>) -> Result<Vec<String>> {
        let data = self.load_data().await?;
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
        let names = json!(accounts);
        self.inner("lichess.invalidateLogin", vec![names.clone()])
            .await?;
        let online_accounts = if username.is_none() {
            Value::Null
        } else {
            names.clone()
        };
        self.inner("online.logout", vec![online_accounts]).await?;
        self.reviews.discard_account_reviews(&accounts);
        self.usage.flush().await;
        self.services.forget_usage(&accounts);
        for name in &accounts {
            self.inner("challenges.forget", vec![json!(name)]).await?;
        }
        // Tournaments joined as a signed-out account would otherwise reopen its event streams.
        self.inner("store.library.forgetTournamentsOf", vec![names])
            .await?;
        Ok(accounts)
    }

    /// Settings changed: store them, and stop what the new settings change (`persistSettings`).
    pub(crate) async fn persist_settings(&self, settings: Value) -> Result<Value> {
        let previous = self.get_settings().await?;
        // A frontend may not point the core at an arbitrary executable: only a path the user
        // picked, downloaded by KChess, or already saved is accepted.
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
        let saved = self
            .database
            .call_async("store.games.saveSettings", &[settings])
            .await?;
        if previous["enginePath"] != saved["enginePath"] {
            self.search.stop_engine(true);
            self.analysis.stop_analysis(true);
            self.reviews.restart_review_engine();
        }
        self.host.emit("settings:saved", core_settings(&saved));
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

    /// `knownChallenge`: the open challenge `id`, or the error it is no longer open.
    pub(crate) async fn known_challenge(&self, id: &Value) -> Result<Value> {
        let found = self.inner("challenges.get", vec![id.clone()]).await?;
        if found.is_null() {
            return Err(CoreError::new("That challenge is no longer open."));
        }
        Ok(found)
    }

    /// `DateTime`-free check that `value` is a `who`-owned account, as `stayConnected` reads it.
    pub(crate) async fn is_connected(&self, name: &str) -> Result<bool> {
        Ok(!self.connected_accounts(Some(name)).await?.is_empty())
    }

    /// The facade method `method`, with its arguments already checked.
    async fn run_facade(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "positionLookup"
            | "lichessStudies"
            | "lichessStudyChapters"
            | "syncLichessStudy"
            | "exportToLichessStudy"
            | "exportGame"
            | "mastersGame"
            | "openingDescription"
            | "openingPopularity"
            | "cloudEval" => self.run_studies(method, args).await,
            "loadData"
            | "settings"
            | "desktopSettings"
            | "saveSettings"
            | "saveDesktopSettings" => self.run_settings(method, args).await,
            "addAccount" | "logout" | "logoutAll" | "removeAccount" => {
                self.run_accounts(method, args).await
            }
            "syncGames"
            | "gamePage"
            | "gameLibraryOverview"
            | "insights"
            | "gameRatingHistory"
            | "gamePgn"
            | "cachedProfile"
            | "profile"
            | "ratingHistory"
            | "connectLichess"
            | "stayConnected"
            | "clearAccountData"
            | "following"
            | "addFriends" => self.run_games(method, args).await,
            "engineStatus" | "installEngine" | "deleteEngine" | "stopEngine" | "bestMove"
            | "startAnalysis" | "stopAnalysis" | "reviewGet" | "reviewRequest" | "reviewCancel"
            | "reviewStatus" | "reviewSummaries" => self.run_engine(method, args).await,
            "startOnline" | "resumeOnline" | "cancelOnline" | "playOnline" | "onlineAction"
            | "presence" | "onlineChat" | "sendChat" | "challenges" | "acceptChallenge"
            | "declineChallenge" | "cancelChallenge" | "ongoingGames" | "openGame"
            | "playerPerf" | "crosstable" | "recentGames" | "sendMessage" | "relation"
            | "setRelation" | "profiles" | "leaderboard" | "searchPlayers" | "activity"
            | "timeline" | "importGame" | "bookmarkedGames" => self.run_online(method, args).await,
            "tvChannels" | "watch" | "watchBroadcast" | "stopWatching" | "broadcasts"
            | "broadcastTour" => self.run_spectate(method, args).await,
            "tournaments" | "tournament" | "joinTournament" | "leaveTournament"
            | "createTournament" => self.run_tournaments(method, args).await,
            "puzzleNext"
            | "puzzleSolve"
            | "puzzleDaily"
            | "puzzleDashboard"
            | "puzzleActivity"
            | "stormDashboard"
            | "puzzleDbStatus"
            | "puzzleDbInstall"
            | "puzzleDbCancel"
            | "puzzleDbDelete"
            | "localPuzzles"
            | "localLadder"
            | "recordPuzzleAttempt"
            | "puzzleProgress"
            | "saveRun"
            | "runSummary"
            | "clearRuns"
            | "saveVoiceAttempt"
            | "updateVoiceAttempt"
            | "voiceHistory"
            | "clearVoiceHistory" => self.run_puzzles(method, args).await,
            "voiceStart" | "voiceAccept" | "voiceFinish" | "voiceStop" => {
                self.run_voice(method, args).await
            }
            "library"
            | "importLibrary"
            | "studyCommand"
            | "saveArchivedGame"
            | "removeArchivedGame"
            | "addMistakes"
            | "answerMistake"
            | "mistakeGames"
            | "saveSession"
            | "joinedTournaments"
            | "recordRepertoireMiss"
            | "clearRepertoireMisses"
            | "viewPreferences"
            | "setViewPreference" => self.run_library(method, args).await,
            "usage"
            | "resetUsage"
            | "trustEnginePath"
            | "voiceHistoryDocument"
            | "suspend"
            | "customThemes" => self.run_system(method, args).await,
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }

    /// Logs a puzzle operation's failure, as the puzzle service's wrapper did.
    pub(crate) async fn puzzle_operation(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        match self.dispatch(method, args).await {
            Ok(value) => Ok(value),
            Err(cause) => {
                self.host.log(
                    Level::Warn,
                    "puzzles",
                    &format!("Puzzle operation failed: {}", cause.message),
                );
                Err(cause)
            }
        }
    }

    /// Logs a user's logout result after the sessions have ended (`logoutAccounts`).
    pub(crate) async fn logout_accounts(&self, username: Option<&str>) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database
            .call_async(
                "store.games.logoutAccounts",
                &[username.map_or(Value::Null, |name| json!(name))],
            )
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
        self.tracker.close();
        if let Err(cause) = self.ctx.hub.close_engines().await {
            self.host.log(
                Level::Warn,
                "core",
                &format!("Engine shutdown failed: {}", cause.message),
            );
        }
        self.tracker.wait().await;
        if let Err(cause) = self.dispatch("lichess.reset", vec![]).await {
            self.host.log(
                Level::Warn,
                "core",
                &format!("Lichess reset failed: {}", cause.message),
            );
        }
        let _ = self.dispatch("cloudEval.clear", vec![]).await;
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
