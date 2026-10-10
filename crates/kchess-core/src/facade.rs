//! The core's facade: every `CoreApi` method (`CORE_METHODS` in `core/src/contracts/core.ts`) and
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
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use std::sync::Arc;

use crate::core::Core;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// Every method the facade answers, in the order of `CORE_METHODS`, then the `KChessCore` extras.
pub const FACADE_METHODS: &[&str] = &[
    "positionLookup",
    "lichessStudies",
    "lichessStudyChapters",
    "syncLichessStudy",
    "exportToLichessStudy",
    "exportGame",
    "mastersGame",
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
    "saveRun",
    "runSummary",
    "clearRuns",
    "saveVoiceAttempt",
    "updateVoiceAttempt",
    "voiceHistory",
    "clearVoiceHistory",
    "library",
    "importLibrary",
    "studyCommand",
    "saveArchivedGame",
    "removeArchivedGame",
    "addMistakes",
    "answerMistake",
    "saveSession",
    "joinedTournaments",
    "recordRepertoireMiss",
    "clearRepertoireMisses",
    "settings",
    "desktopSettings",
    "saveDesktopSettings",
    "trustEnginePath",
    "voiceHistoryDocument",
    "suspend",
    "close",
];

/// The `CORE_SETTINGS_KEYS` a headless client sees (`core/src/contracts/settings.ts`).
const CORE_SETTINGS_KEYS: &[&str] = &[
    "enginePath",
    "engineLevels",
    "reviewAuto",
    "reviewOnBattery",
    "receiveChallenges",
    "onlineChat",
    "correspondencePoll",
    "cloudEval",
    "voiceHistory",
];

/// The error of a closed core (`The KChess core is closed.`).
pub const CLOSED: &str = "The KChess core is closed.";

/// The puzzle database `puzzleDbInstall` downloads (`PUZZLE_DB_URL`, `core/src/services/puzzleDb.ts`).
const PUZZLE_DB_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

/// The profiles in use by a live core, by canonical data directory; each holds the claim's token,
/// so a stale release never frees a profile a later core has claimed.
static PROFILES: LazyLock<Mutex<HashMap<PathBuf, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLAIMS: AtomicU64 = AtomicU64::new(1);

/// One core's claim on its profile directory: one live core per profile (`createKChessCore`).
pub struct ProfileClaim {
    path: PathBuf,
    token: u64,
}

impl ProfileClaim {
    /// Claims the profile at `data_dir` (its real path), or fails when a core already runs there.
    pub fn claim(data_dir: &Path) -> Result<ProfileClaim> {
        let path =
            std::fs::canonicalize(data_dir).map_err(|cause| CoreError::new(cause.to_string()))?;
        let token = CLAIMS.fetch_add(1, Ordering::SeqCst);
        let mut profiles = PROFILES
            .lock()
            .map_err(|_| CoreError::new("The profiles are unavailable."))?;
        if profiles.contains_key(&path) {
            return Err(CoreError::new(
                "A KChess core is already running for this profile.",
            ));
        }
        profiles.insert(path.clone(), token);
        Ok(ProfileClaim { path, token })
    }

    /// Frees the profile, once the core has closed it.
    pub fn release(&self) {
        if let Ok(mut profiles) = PROFILES.lock()
            && profiles.get(&self.path) == Some(&self.token)
        {
            profiles.remove(&self.path);
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
}

/// Whether `method` is one of the facade's (answered by `Core::facade`).
pub fn is_facade_method(method: &str) -> bool {
    FACADE_METHODS.contains(&method)
}

/// `Date.now()`.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Null)
}

fn text_arg(args: &[Value], index: usize) -> String {
    arg(args, index).as_str().unwrap_or_default().to_string()
}

/// A validator's normalized value (`assertX(value)` in the TypeScript).
fn normal(name: &str, value: &Value) -> Result<Value> {
    normal_args(name, std::slice::from_ref(value))
}

/// A validator of several arguments, normalized (`assertX(a, b)` in the TypeScript).
fn normal_args(name: &str, args: &[Value]) -> Result<Value> {
    kchess_domain::misc::normalize(name, args).map_err(CoreError::new)
}

/// The core's settings as a headless client sees them (`coreSettings`).
pub fn core_settings(settings: &Value) -> Value {
    let mut picked = Map::new();
    for key in CORE_SETTINGS_KEYS {
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
    async fn inner(&self, name: &str, args: Vec<Value>) -> Result<Value> {
        Box::pin(self.dispatch(name, args)).await
    }

    /// Migrates an earlier release's profile once, before the first store read (`ensureMigrated`).
    /// The plaintext tokens of an earlier database are encrypted with the OS store here.
    async fn ensure_migrated(&self) -> Result<()> {
        self.migrated
            .get_or_try_init(|| async { self.migrate().await })
            .await
            .map(|_| ())
    }

    async fn migrate(&self) -> Result<()> {
        let available = self.capabilities.secrets_available().await?;
        let legacy = self
            .database
            .call("store.games.migrate", &[json!(available)])?;
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
                    .call(
                        "store.games.importTokenCiphertext",
                        &[account, json!(encrypted)],
                    )
                    .map(|_| ()),
                Err(cause) => Err(cause),
            };
            if let Err(cause) = stored {
                self.host.log(
                    Level::Debug,
                    "store",
                    &format!("Legacy migration failed: {}", cause.message),
                );
            }
        }
        Ok(())
    }

    /// The saved settings (`getSettings`).
    async fn get_settings(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database.call("store.games.getSettings", &[])
    }

    /// Settings, accounts and the library size (`loadData`).
    async fn load_data(&self) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database.call("store.games.loadData", &[])
    }

    /// The Lichess accounts that are signed in, as names; `username` restricts the list.
    async fn connected_accounts(&self, username: Option<&str>) -> Result<Vec<String>> {
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
    async fn end_sessions(&self, username: Option<&str>) -> Result<Vec<String>> {
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
        self.usage.flush();
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
    async fn persist_settings(&self, settings: Value) -> Result<Value> {
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
            .call("store.games.saveSettings", &[settings])?;
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
    fn assistance_allowed(&self, message: &str) -> Result<()> {
        if self.services.online().assistance_blocked() {
            return Err(CoreError::new(message));
        }
        Ok(())
    }

    /// `knownChallenge`: the open challenge `id`, or the error it is no longer open.
    async fn known_challenge(&self, id: &Value) -> Result<Value> {
        let found = self.inner("challenges.get", vec![id.clone()]).await?;
        if found.is_null() {
            return Err(CoreError::new("That challenge is no longer open."));
        }
        Ok(found)
    }

    /// `DateTime`-free check that `value` is a `who`-owned account, as `stayConnected` reads it.
    async fn is_connected(&self, name: &str) -> Result<bool> {
        Ok(!self.connected_accounts(Some(name)).await?.is_empty())
    }

    /// The facade method `method`, with its arguments already checked.
    async fn run_facade(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            // Lichess studies, positions and the explorer.
            "positionLookup" => {
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                self.inner("positionLookup.lookup", args.to_vec()).await
            }
            "lichessStudies" => self.inner("studies.list", args.to_vec()).await,
            "lichessStudyChapters" => self.inner("studies.chapters", args.to_vec()).await,
            "syncLichessStudy" => {
                let request = normal("assertStudySyncRequest", &arg(args, 0))?;
                self.inner("studies.sync", vec![request]).await
            }
            "exportToLichessStudy" => {
                let name = text_arg(args, 2);
                self.inner(
                    "studies.export",
                    vec![arg(args, 0), arg(args, 1), json!(name.trim()), arg(args, 3)],
                )
                .await
            }
            "exportGame" => self.inner("lichess.exportGame", args.to_vec()).await,
            "mastersGame" => {
                self.assistance_allowed(
                    "Position lookups are unavailable during a live Lichess game.",
                )?;
                self.inner("positionLookup.mastersGame", args.to_vec())
                    .await
            }
            "cloudEval" => {
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                // Each request sends the position to Lichess, so it needs the setting turned on.
                if self.get_settings().await?["cloudEval"].as_bool() != Some(true) {
                    return Err(CoreError::new(
                        "Turn on cloud evaluation in Settings → Analysis first.",
                    ));
                }
                let request = normal(
                    "assertAnalysisRequest",
                    &json!({ "fen": arg(args, 0), "lines": arg(args, 1) }),
                )?;
                self.inner(
                    "cloudEval.cloudEval",
                    vec![request["fen"].clone(), request["lines"].clone()],
                )
                .await
            }

            // Accounts, settings and the library of games.
            "loadData" => self.load_data().await,
            "settings" => Ok(core_settings(&self.get_settings().await?)),
            "desktopSettings" => self.get_settings().await,
            "saveSettings" => {
                let settings = normal("assertCoreSettings", &arg(args, 0))?;
                let mut merged = self.get_settings().await?;
                if let (Some(target), Some(changes)) =
                    (merged.as_object_mut(), settings.as_object())
                {
                    for (key, value) in changes {
                        target.insert(key.clone(), value.clone());
                    }
                }
                let saved = self.persist_settings(merged).await?;
                Ok(core_settings(&saved))
            }
            "saveDesktopSettings" => {
                let settings = normal("assertSettings", &arg(args, 0))?;
                self.persist_settings(settings).await
            }
            "addAccount" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database
                    .call("store.games.addAccount", &[name, json!(false)])
            }
            "logout" => {
                let username = text_arg(args, 0);
                self.end_sessions(Some(&username)).await?;
                let data = self.logout_accounts(Some(&username)).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "logoutAll" => {
                self.end_sessions(None).await?;
                let data = self.logout_accounts(None).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "removeAccount" => {
                let name = text_arg(args, 0);
                if self.end_sessions(Some(&name)).await?.is_empty() {
                    self.inner("lichess.cancelAccountSyncs", vec![json!([name])])
                        .await?;
                    self.reviews
                        .discard_account_reviews(std::slice::from_ref(&name));
                    self.usage.flush();
                    self.services.forget_usage(std::slice::from_ref(&name));
                    self.inner("store.library.forgetTournamentsOf", vec![json!([name])])
                        .await?;
                }
                self.inner("challenges.forget", vec![json!(name)]).await?;
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call("store.games.removeAccount", &[json!(name)])?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "syncGames" => {
                let username = match args.first() {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertUsername", value)?,
                };
                let data = self.inner("lichess.syncGames", vec![username]).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "gamePage" => {
                let query = normal("assertGamePageQuery", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database.call("store.games.gamePage", &[query])
            }
            "gameLibraryOverview" => {
                self.ensure_migrated().await?;
                self.database.call("store.games.gameLibraryOverview", &[])
            }
            "insights" => {
                let query = normal("assertInsightsQuery", &arg(args, 0))?;
                self.database.call("store.insights.insights", &[query])
            }
            "gameRatingHistory" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                self.ensure_migrated().await?;
                self.database
                    .call("store.games.gameRatingHistory", &[account])
            }
            "gamePgn" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let id = normal("assertGameId", &arg(args, 1))?;
                self.ensure_migrated().await?;
                self.database.call("store.games.gamePgn", &[account, id])
            }
            "cachedProfile" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.cachedProfile", vec![name]).await
            }
            "profile" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.profile", vec![name]).await
            }
            "ratingHistory" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.ratingHistory", vec![name]).await
            }
            "connectLichess" => {
                // `oauthLook` falls back to the safe default colours for anything it cannot read.
                let look = match kchess_domain::records::call("oauthLook", &[arg(args, 0)]) {
                    Some(Ok(look)) => look,
                    _ => Value::Null,
                };
                self.inner("lichess.connectLichess", vec![look]).await
            }
            "stayConnected" => {
                let name = normal("assertOptionalAccount", &arg(args, 0))?;
                let wanted = name.as_str().unwrap_or_default().to_string();
                let connected = self.is_connected(&wanted).await?;
                self.inner(
                    "online.stayConnected",
                    vec![json!(if connected { wanted } else { String::new() })],
                )
                .await
            }
            "clearAccountData" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                self.inner("lichess.cancelAccountSyncs", vec![json!([name])])
                    .await?;
                self.reviews
                    .discard_account_reviews(&[name.as_str().unwrap_or_default().to_string()]);
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call("store.games.clearAccountData", std::slice::from_ref(&name))?;
                self.inner("lichess.forgetProfile", vec![name]).await?;
                self.reviews.reviews_changed();
                Ok(data)
            }
            "following" => self.inner("lichess.followedUsers", vec![]).await,
            "addFriends" => {
                let names = normal("assertFriendList", &arg(args, 0))?;
                self.ensure_migrated().await?;
                let data = self
                    .database
                    .call("store.games.addFriends", std::slice::from_ref(&names))?;
                self.inner("lichess.primeProfiles", vec![names]).await?;
                Ok(data)
            }

            // The computer's engine and analysis.
            "engineStatus" => {
                let configured = self.get_settings().await?["enginePath"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let mut status = self.inner("engine.status", vec![json!(configured)]).await?;
                if status["ready"].as_bool() == Some(true) {
                    let identity = self.inner("engine.identity", vec![status.clone()]).await?;
                    status["identity"] = identity;
                }
                Ok(status)
            }
            "installEngine" => {
                let id = self.next_request();
                let installed = self
                    .inner("managedEngine.install", vec![json!(id), Value::Null])
                    .await?;
                Ok(json!({
                    "path": installed["path"],
                    "version": installed["version"],
                    "updated": installed["updated"],
                }))
            }
            "deleteEngine" => {
                let id = self.next_request();
                self.inner("managedEngine.delete", vec![json!(id), Value::Null])
                    .await
            }
            "stopEngine" => {
                self.search.stop_engine(false);
                Ok(Value::Null)
            }
            "bestMove" => {
                self.assistance_allowed(
                    "Engine assistance is unavailable during a live Lichess game.",
                )?;
                let moves = normal("assertMoves", &arg(args, 0))?;
                let level = normal("assertLevel", &arg(args, 1))?;
                let options = normal("assertBestMoveOptions", &arg(args, 2))?;
                let configured = self.get_settings().await?["enginePath"].clone();
                self.inner("engine.bestMove", vec![moves, level, configured, options])
                    .await
            }
            "startAnalysis" => {
                self.assistance_allowed("Analysis is unavailable during a live Lichess game.")?;
                let configured = self.get_settings().await?["enginePath"].clone();
                self.inner("analysis.start", vec![arg(args, 0), configured])
                    .await
            }
            "stopAnalysis" => {
                self.analysis.stop_analysis(false);
                Ok(Value::Null)
            }

            // Game review.
            "reviewGet" => {
                self.assistance_allowed(
                    "Review is unavailable until your Lichess game status is verified.",
                )?;
                let request = normal(
                    "assertReviewRequest",
                    &json!({ "fen": arg(args, 0), "moves": arg(args, 1) }),
                )?;
                self.inner(
                    "reviews.get",
                    vec![request["fen"].clone(), request["moves"].clone()],
                )
                .await
            }
            "reviewRequest" => {
                self.assistance_allowed("Review is unavailable during a live Lichess game.")?;
                let request = normal("assertReviewRequest", &arg(args, 0))?;
                self.inner("reviews.request", vec![request]).await
            }
            "reviewCancel" => {
                let key = normal("assertReviewKey", &arg(args, 0))?;
                self.inner("reviews.cancel", vec![key]).await
            }
            "reviewStatus" => self.inner("reviews.status", vec![]).await,
            "reviewSummaries" => {
                let ids = normal("assertGameIds", &arg(args, 0))?;
                self.database
                    .call("store.reviewStore.reviewSummaries", &[ids])
            }

            // Online play.
            "startOnline" => {
                let options = normal("assertOnlineOptions", &arg(args, 0))?;
                self.inner("online.start", vec![options]).await
            }
            "resumeOnline" => self.inner("online.resume", vec![]).await,
            "cancelOnline" => self.inner("online.cancel", vec![]).await,
            "playOnline" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let uci = normal("assertUci", &arg(args, 1))?;
                self.inner("online.move", vec![id, uci]).await
            }
            "onlineAction" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let action = normal("assertAction", &arg(args, 1))?;
                self.inner("online.action", vec![id, action]).await
            }
            "presence" => {
                let usernames = normal("assertUsernames", &arg(args, 0))?;
                self.inner("online.presence", vec![usernames]).await
            }
            "onlineChat" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                self.inner("online.chat", vec![id]).await
            }
            "sendChat" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let room = normal("assertChatRoom", &arg(args, 1))?;
                let text = normal("assertChatText", &arg(args, 2))?;
                self.inner("online.sendChat", vec![id, room, text]).await
            }
            "challenges" => self.inner("challenges.list", vec![]).await,
            "acceptChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("in")
                    || challenge["playable"].as_bool() != Some(true)
                {
                    let problem = challenge["problem"]
                        .as_str()
                        .unwrap_or("Only challenges sent to you can be accepted.");
                    return Err(CoreError::new(problem));
                }
                self.inner("online.acceptChallenge", vec![challenge])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "declineChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("in") {
                    return Err(CoreError::new("Withdraw your own challenge instead."));
                }
                let reason = normal("assertDeclineReason", &arg(args, 1))?;
                self.inner("online.declineChallenge", vec![challenge, reason])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "cancelChallenge" => {
                let id = normal("assertGameId", &arg(args, 0))?;
                let challenge = self.known_challenge(&id).await?;
                if challenge["direction"].as_str() != Some("out") {
                    return Err(CoreError::new("Decline a challenge sent to you instead."));
                }
                self.inner("online.withdrawChallenge", vec![challenge])
                    .await?;
                self.inner("challenges.remove", vec![id]).await?;
                Ok(Value::Null)
            }
            "ongoingGames" => self.inner("online.ongoing", vec![]).await,
            "openGame" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let id = normal("assertGameId", &arg(args, 1))?;
                self.inner("online.open", vec![account, id]).await
            }

            // Lichess profiles, games and players.
            "playerPerf" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let perf = normal("assertPerfType", &arg(args, 1))?;
                self.inner("lichess.playerPerf", vec![name, perf]).await
            }
            "crosstable" => {
                let a = normal("assertUsername", &arg(args, 0))?;
                let b = normal("assertUsername", &arg(args, 1))?;
                self.inner("lichess.crosstable", vec![a, b]).await
            }
            "recentGames" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let rated = arg(args, 1).as_bool() == Some(true);
                self.inner("lichess.recentGames", vec![name, json!(rated)])
                    .await
            }
            "sendMessage" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let username = normal("assertUsername", &arg(args, 1))?;
                let text = normal("assertMessageText", &arg(args, 2))?;
                self.inner("lichess.sendMessage", vec![account, username, text])
                    .await
            }
            "tvChannels" => self.inner("spectate.tvChannels", vec![]).await,
            "watch" => {
                if self.services.online().playing() {
                    return Err(CoreError::new("Finish your game before watching another."));
                }
                let target = normal_args(
                    "assertWatchTarget",
                    &[arg(args, 0), json!(kchess_domain::misc::TV_CHANNEL_KEYS)],
                )?;
                self.inner("spectate.watch", vec![target]).await
            }
            "watchBroadcast" => {
                if self.services.online().playing() {
                    return Err(CoreError::new("Finish your game before watching another."));
                }
                let round = normal("assertLichessId", &arg(args, 0))?;
                self.inner("spectate.watchRound", vec![round]).await
            }
            "stopWatching" => {
                self.services.spectator().stop();
                Ok(Value::Null)
            }
            "broadcasts" => {
                let query = normal("assertBroadcastQuery", &arg(args, 0))?;
                self.inner("spectate.broadcasts", vec![query]).await
            }
            "broadcastTour" => {
                let id = normal("assertLichessId", &arg(args, 0))?;
                self.inner("spectate.broadcastTour", vec![id]).await
            }
            "tournaments" => {
                let account = normal("assertOptionalAccount", &arg(args, 0))?;
                self.inner("tournaments.list", vec![account]).await
            }
            "tournament" => {
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let account = normal("assertOptionalAccount", &arg(args, 2))?;
                let page = match arg(args, 3).as_i64() {
                    Some(page) if (1..=200).contains(&page) => page,
                    _ => 1,
                };
                let detail = self
                    .inner(
                        "tournaments.get",
                        vec![system.clone(), id.clone(), account, json!(page)],
                    )
                    .await?;
                // An arena says whether you are in it; forget it when it is over or you left.
                let withdrew = detail["me"]["withdraw"].as_bool() == Some(true);
                if system.as_str() == Some("arena")
                    && (detail["status"].as_str() == Some("finished")
                        || detail["me"].is_null()
                        || withdrew)
                {
                    self.inner("store.library.forgetTournament", vec![system, id])
                        .await?;
                }
                Ok(detail)
            }
            "joinTournament" => {
                let name = normal("assertUsername", &arg(args, 2))?;
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let password = normal("assertTournamentPassword", &arg(args, 3))?;
                let result = self
                    .inner(
                        "tournaments.join",
                        vec![system.clone(), id.clone(), name.clone(), password],
                    )
                    .await?;
                if result != json!(true) {
                    return Ok(result);
                }
                // Pairings arrive on the event stream: keep it open while in the tournament.
                self.inner("online.stayConnected", vec![name.clone()])
                    .await?;
                let now = now_ms();
                // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
                let mut entry = json!({
                    "system": system,
                    "id": id,
                    "account": name,
                    "name": id,
                    "until": now + 86_400_000i64,
                });
                match self
                    .inner(
                        "tournaments.get",
                        vec![system.clone(), id.clone(), name.clone(), json!(1)],
                    )
                    .await
                {
                    Ok(detail) => {
                        entry["name"] = detail["name"].clone();
                        entry["until"] = if detail["finishesAt"].is_null() {
                            entry["until"].clone()
                        } else {
                            detail["finishesAt"].clone()
                        };
                    }
                    Err(cause) => self.host.log(
                        Level::Warn,
                        "tournaments",
                        &format!(
                            "Joined tournament details are unavailable: id={} {}",
                            id.as_str().unwrap_or_default(),
                            cause.message
                        ),
                    ),
                }
                self.database
                    .call("store.library.rememberTournament", &[entry, json!(now)])?;
                Ok(result)
            }
            "leaveTournament" => {
                let system = normal("assertTournamentSystem", &arg(args, 0))?;
                let id = normal("assertTournamentId", &arg(args, 1))?;
                let account = normal("assertUsername", &arg(args, 2))?;
                let result = self
                    .inner(
                        "tournaments.leave",
                        vec![system.clone(), id.clone(), account],
                    )
                    .await?;
                if result == json!(true) {
                    self.inner("store.library.forgetTournament", vec![system, id])
                        .await?;
                }
                Ok(result)
            }
            "createTournament" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let arena = normal("assertNewArena", &arg(args, 1))?;
                self.inner("tournaments.create", vec![account, arena]).await
            }

            // Puzzles, the puzzle database and training runs.
            "puzzleNext" => {
                let request = normal("assertPuzzleRequest", &arg(args, 0))?;
                self.inner("lichess.puzzleNext", vec![request]).await
            }
            "puzzleSolve" => {
                let request = normal("assertPuzzleSolve", &arg(args, 0))?;
                self.inner("lichess.puzzleSolve", vec![request]).await
            }
            "puzzleDaily" => self.inner("lichess.puzzleDaily", vec![]).await,
            "puzzleDashboard" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let days = normal("assertDays", &arg(args, 1))?;
                self.inner(
                    "lichess.puzzleDashboard",
                    vec![json!({ "account": account, "days": days })],
                )
                .await
            }
            "puzzleActivity" => {
                let account = normal("assertUsername", &arg(args, 0))?;
                let max = normal("assertActivityMax", &arg(args, 1))?;
                self.inner("lichess.puzzleActivity", vec![account, max])
                    .await
            }
            "stormDashboard" => {
                let name = normal("assertUsername", &arg(args, 0))?;
                let days = normal("assertDays", &arg(args, 1))?;
                self.inner("lichess.stormDashboard", vec![name, days]).await
            }
            "puzzleDbStatus" => self.puzzle_operation("puzzles.status", vec![]).await,
            "puzzleDbInstall" => {
                self.puzzle_operation("puzzles.install", vec![json!(PUZZLE_DB_URL)])
                    .await
            }
            "puzzleDbCancel" => {
                if let Err(cause) = self.dispatch("puzzles.cancel", vec![]).await {
                    self.host.log(
                        Level::Warn,
                        "puzzles",
                        &format!("Puzzle download cancel failed: {}", cause.message),
                    );
                }
                Ok(Value::Null)
            }
            "puzzleDbDelete" => self.puzzle_operation("puzzles.delete", vec![]).await,
            "localPuzzles" => {
                let query = normal("assertLocalQuery", &arg(args, 0))?;
                self.puzzle_operation("puzzles.query", vec![query]).await
            }
            "localLadder" => {
                let query = normal("assertLadderQuery", &arg(args, 0))?;
                self.puzzle_operation("puzzles.ladder", vec![query]).await
            }
            "saveRun" => {
                let run = normal("assertRunInput", &arg(args, 0))?;
                self.database.call("store.runs.saveRun", &[run])
            }
            "runSummary" => {
                let kind = normal("assertRunKind", &arg(args, 0))?;
                self.database.call("store.runs.runSummary", &[kind])
            }
            "clearRuns" => {
                let kind = match args.first() {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertRunKind", value)?,
                };
                self.database.call("store.runs.clearRuns", &[kind])
            }
            "saveVoiceAttempt" => {
                let attempt = normal("assertVoiceAttempt", &arg(args, 0))?;
                self.database
                    .call("store.voiceLog.saveVoiceAttempt", &[attempt])
            }
            "updateVoiceAttempt" => {
                let id = normal("assertVoiceId", &arg(args, 0))?;
                let update = normal("assertVoiceUpdate", &arg(args, 1))?;
                self.database
                    .call("store.voiceLog.updateVoiceAttempt", &[id, update])
            }
            "voiceHistory" => {
                let limit = normal("assertVoiceLimit", &arg(args, 0))?;
                self.database.call("store.voiceLog.voiceHistory", &[limit])
            }
            "clearVoiceHistory" => self.database.call("store.voiceLog.clearVoiceHistory", &[]),

            // The library: studies, played games, drills, sessions and repertoire notes.
            "library" => self
                .database
                .call("store.library.library", &[json!(now_ms())]),
            "importLibrary" => {
                let documents = normal("assertLegacyDocuments", &arg(args, 0))?;
                self.database
                    .call("store.library.importLibrary", &[documents])
            }
            "studyCommand" => {
                let shape = normal("assertStudyCommandShape", &arg(args, 0))?;
                let command = study_command(&shape)?;
                self.database
                    .call("store.library.studyCommand", &[command, json!(now_ms())])
            }
            "saveArchivedGame" => {
                let shape = normal("assertArchivedGameShape", &arg(args, 0))?;
                let game = archived_game(&shape)?;
                self.database
                    .call("store.library.saveArchivedGame", &[game])
            }
            "removeArchivedGame" => {
                let id = normal("assertLibraryId", &arg(args, 0))?;
                self.database
                    .call("store.library.removeArchivedGame", &[id])
            }
            "addMistakes" => {
                let key = normal("assertReviewKey", &arg(args, 0))?;
                let color = match args.get(1) {
                    None | Some(Value::Null) => Value::Null,
                    Some(value) => normal("assertSide", value)?,
                };
                self.database
                    .call("store.library.addMistakes", &[key, color, json!(now_ms())])
            }
            "answerMistake" => {
                let id = normal("assertLibraryId", &arg(args, 0))?;
                let solved = normal("assertBoolean", &arg(args, 1))?;
                self.database.call(
                    "store.library.answerMistake",
                    &[id, solved, json!(now_ms())],
                )
            }
            "saveSession" => {
                let kind = normal("assertSessionKind", &arg(args, 0))?;
                let value = normal_args("assertSession", &[kind.clone(), arg(args, 1)])?;
                self.database
                    .call("store.library.saveSession", &[kind, value])
            }
            "joinedTournaments" => self
                .database
                .call("store.library.joinedTournaments", &[json!(now_ms())]),
            "recordRepertoireMiss" => {
                let key = normal("assertRepertoireKey", &arg(args, 0))?;
                let fen = normal(
                    "assertAnalysisRequest",
                    &json!({ "fen": arg(args, 1), "lines": 1 }),
                )?;
                self.database.call(
                    "store.library.recordRepertoireMiss",
                    &[key, fen["fen"].clone()],
                )
            }
            "clearRepertoireMisses" => {
                let key = normal("assertRepertoireKey", &arg(args, 0))?;
                self.database
                    .call("store.library.clearRepertoireMisses", &[key])
            }

            // Usage and the lifecycle.
            "usage" => self.usage.report(),
            "resetUsage" => self.usage.reset(),
            "trustEnginePath" | "voiceHistoryDocument" => self.call_sync(method, args.to_vec()),
            "suspend" => {
                if let Err(cause) = self.dispatch("puzzles.cancel", vec![]).await {
                    self.host.log(
                        Level::Warn,
                        "puzzles",
                        &format!("Puzzle download cancel failed: {}", cause.message),
                    );
                }
                self.services.spectator().stop();
                self.services.online().close();
                self.search.stop_engine(true);
                self.analysis.stop_analysis(true);
                Ok(Value::Null)
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }

    /// Logs a puzzle operation's failure, as the puzzle service's wrapper did.
    async fn puzzle_operation(&self, method: &str, args: Vec<Value>) -> Result<Value> {
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
    async fn logout_accounts(&self, username: Option<&str>) -> Result<Value> {
        self.ensure_migrated().await?;
        self.database.call(
            "store.games.logoutAccounts",
            &[username.map_or(Value::Null, |name| json!(name))],
        )
    }

    /// A request id for a managed engine install or delete (the host cancels it by id).
    fn next_request(&self) -> u64 {
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
        self.usage.flush();
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
        self.database.close();
        self.claim.release();
    }
}

/// `assertStudyCommand`: a restored study is decoded by the rules; other commands pass through.
fn study_command(shape: &Value) -> Result<Value> {
    if shape["op"].as_str() != Some("restore") {
        return Ok(shape.clone());
    }
    let study = kchess_domain::library::decode_study(&shape["study"])
        .ok_or_else(|| CoreError::new("That study cannot be restored."))?;
    Ok(json!({ "op": "restore", "study": study }))
}

/// `assertArchivedGame`: the archived game as the rules decode it.
fn archived_game(shape: &Value) -> Result<Value> {
    kchess_domain::library::decode_archived_game(shape)
        .ok_or_else(|| CoreError::new("This game cannot be saved."))
}
