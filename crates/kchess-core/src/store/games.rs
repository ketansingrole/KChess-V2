//! `core/src/services/store.ts`: settings, accounts, the game library, the Lichess API cache and the
//! account lifecycle, as `store.games.<name>` methods over `kchess.db`.
//!
//! Each method takes the TypeScript arguments as a JSON array and returns the TypeScript result as
//! JSON (`undefined` is `null` or an omitted key, as `JSON.stringify` writes it). Not mirrored yet:
//! `saveLogin` and `getToken` (the host's credential encryption is not a core capability yet), the
//! Lichess reviews that `saveGames`/`saveGamesPage` write (`reviewStore.ts`, `store/reviews.rs`),
//! the one-time JSON and legacy imports that run before the first read (they need the data
//! directory and the legacy path, which the dispatch does not carry), and the in-memory caches
//! (`snapshot`, `pageTotals`): they only save recounting, so results are the same.
//! A callback argument (`stillCurrent`) is passed as its boolean result, read just before the call.

use std::collections::{BTreeMap, HashMap};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::error::{CoreError, Result};
use kchess_domain::api;

/// Games kept per account (`MAX_GAMES` in `store.ts`).
pub const MAX_GAMES: i64 = 5000;

/// Must match the indexed expression in `migrations.rs` verbatim, or SQLite falls back to a scan.
pub const RESULT_SQL: &str =
    "CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END";

/// Every persisted setting, in column order (`SETTINGS_KEYS`).
const SETTINGS_KEYS: [&str; 40] = [
    "appearance",
    "boardTheme",
    "lightTheme",
    "darkTheme",
    "pieceSet",
    "pieceAnimation",
    "coordinates",
    "soundEnabled",
    "soundVolume",
    "enginePath",
    "premove",
    "promotion",
    "showLegalMoves",
    "notificationsEnabled",
    "notifyActive",
    "notifyBackground",
    "notifyOpponentMove",
    "notifyLowTime",
    "notifyGameEvents",
    "notifyComputerMove",
    "notifySound",
    "voicePushToTalk",
    "voiceConfirmMoves",
    "voiceHistory",
    "updateAutoCheck",
    "updateAutoDownload",
    "updateInstallOnQuit",
    "engineLevels",
    "reviewAuto",
    "reviewOnBattery",
    "receiveChallenges",
    "notifyChallenges",
    "onlineChat",
    "correspondencePoll",
    "zenMode",
    "blindfold",
    "cloudEval",
    "showOpeningName",
    "swipeNavigation",
    "swipeIndicator",
];

/// Every persisted game field, in column order (`GAME_KEYS`).
const GAME_KEYS: [&str; 17] = [
    "account",
    "id",
    "createdAt",
    "lastMoveAt",
    "rated",
    "speed",
    "perf",
    "status",
    "winner",
    "color",
    "opponent",
    "opponentRating",
    "playerRating",
    "ratingDiff",
    "opening",
    "moves",
    "pgn",
];

/// The list view never shows the PGN; it is fetched per game when reviewed.
const LIST_COLUMNS: &str = "account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color, opponent, opponentRating, playerRating, ratingDiff, opening, moves";

const PIECE_ANIMATIONS: [&str; 4] = ["none", "fast", "normal", "slow"];
const REVIEW_AUTO: [&str; 3] = ["off", "recent", "all"];

/// `DEFAULT_SETTINGS` (`core/src/contracts/defaultSettings.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub appearance: String,
    pub board_theme: String,
    pub light_theme: String,
    pub dark_theme: String,
    pub piece_set: String,
    pub piece_animation: String,
    pub coordinates: String,
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub engine_path: String,
    pub premove: bool,
    pub promotion: String,
    pub show_legal_moves: bool,
    pub notifications_enabled: bool,
    pub notify_active: bool,
    pub notify_background: bool,
    pub notify_opponent_move: bool,
    pub notify_low_time: bool,
    pub notify_game_events: bool,
    pub notify_computer_move: bool,
    pub notify_sound: bool,
    pub voice_push_to_talk: bool,
    pub voice_confirm_moves: bool,
    pub voice_history: bool,
    pub update_auto_check: bool,
    pub update_auto_download: bool,
    pub update_install_on_quit: bool,
    pub engine_levels: Vec<String>,
    pub review_auto: String,
    pub review_on_battery: bool,
    pub receive_challenges: bool,
    pub notify_challenges: bool,
    pub online_chat: bool,
    pub correspondence_poll: f64,
    pub zen_mode: bool,
    pub blindfold: bool,
    pub cloud_eval: bool,
    pub show_opening_name: bool,
    pub swipe_navigation: bool,
    pub swipe_indicator: bool,
}

fn default_settings() -> Settings {
    Settings {
        appearance: "system".into(),
        board_theme: "brown".into(),
        light_theme: "kchess".into(),
        dark_theme: "kchess".into(),
        piece_set: "cburnett".into(),
        piece_animation: "normal".into(),
        coordinates: "inside".into(),
        sound_enabled: true,
        sound_volume: 0.7,
        engine_path: String::new(),
        premove: true,
        promotion: "ask".into(),
        show_legal_moves: true,
        notifications_enabled: true,
        notify_active: false,
        notify_background: true,
        notify_opponent_move: true,
        notify_low_time: true,
        notify_game_events: true,
        notify_computer_move: true,
        notify_sound: false,
        voice_push_to_talk: false,
        voice_confirm_moves: true,
        voice_history: true,
        update_auto_check: true,
        update_auto_download: true,
        update_install_on_quit: true,
        engine_levels: ["beginner", "club", "expert", "fm", "im", "gm", "max"]
            .map(String::from)
            .to_vec(),
        review_auto: "recent".into(),
        review_on_battery: false,
        receive_challenges: true,
        notify_challenges: true,
        online_chat: true,
        correspondence_poll: 5.0,
        zen_mode: false,
        blindfold: false,
        cloud_eval: false,
        show_opening_name: true,
        swipe_navigation: true,
        swipe_indicator: true,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppData {
    pub settings: Settings,
    pub accounts: Vec<Account>,
    pub game_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub username: String,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LichessGame {
    pub id: String,
    pub account: String,
    pub created_at: i64,
    pub last_move_at: i64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<String>,
    pub color: String,
    pub opponent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opponent_rating: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_rating: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating_diff: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    pub moves: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgn: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePage {
    pub games: Vec<LichessGame>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct GameRecord {
    pub total: i64,
    pub win: i64,
    pub loss: Option<i64>,
    pub draw: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLibraryOverview {
    pub by_account: BTreeMap<String, GameRecord>,
    pub versus: BTreeMap<String, GameRecord>,
}

#[derive(Debug, Serialize)]
pub struct RatingSeries {
    pub name: String,
    pub points: Vec<[i64; 4]>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GamePageQuery {
    account: Option<String>,
    result: Option<String>,
    rated: Option<bool>,
    // Validated whole numbers; the validator hands them back as JSON numbers, which may be floats.
    offset: f64,
    limit: f64,
}

/// Lets the `?` operator carry SQLite failures as the core's errors.
trait StorageResult<T> {
    fn sql(self) -> Result<T>;
}

impl<T> StorageResult<T> for rusqlite::Result<T> {
    fn sql(self) -> Result<T> {
        self.map_err(|cause| CoreError::new(cause.to_string()))
    }
}

/// This module's storage methods; None when the method is not one of them.
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let name = method.strip_prefix("store.games.")?;
    Some(match name {
        "getSettings" => get_settings(db),
        "loadData" => load_app_data(db).and_then(as_json),
        "gamePage" => arg(args, 0, "query").and_then(|query| game_page(db, query)),
        "gameLibraryOverview" => game_library_overview(db).and_then(as_json),
        "gameRatingHistory" => arg(args, 0, "account")
            .and_then(|account: String| rating_history(db, &account))
            .and_then(as_json),
        "pendingGameIds" => arg(args, 0, "account")
            .and_then(|account: String| pending_game_ids(db, &account))
            .and_then(as_json),
        "gamePgn" => arg(args, 0, "account").and_then(|account: String| {
            arg(args, 1, "id").and_then(|id: String| game_pgn(db, &account, &id))
        }),
        "readApiCache" => arg(args, 0, "key").and_then(|key: String| read_api_cache(db, &key)),
        "writeApiCache" => arg(args, 0, "key").and_then(|key: String| {
            write_api_cache(
                db,
                &key,
                args.get(1).unwrap_or(&Value::Null),
                current(args, 2),
            )
        }),
        "saveSettings" => arg(args, 0, "settings").and_then(|settings: Settings| {
            write_settings(db, &settings)?;
            as_json(settings)
        }),
        "addAccount" => arg(args, 0, "username").and_then(|username: String| {
            let connected = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            add_account(db, &username, connected)
        }),
        "addFriends" => arg(args, 0, "usernames")
            .and_then(|usernames: Vec<Value>| add_friends(db, &usernames))
            .and_then(as_json),
        "dismissedFriends" => dismissed_friends(db).and_then(as_json),
        "logoutAccounts" => optional_arg::<String>(args, 0, "username")
            .and_then(|username| logout_accounts(db, username.as_deref()))
            .and_then(as_json),
        "removeAccount" => arg(args, 0, "username")
            .and_then(|username: String| remove_account(db, &username))
            .and_then(as_json),
        "clearAccountData" => arg(args, 0, "username")
            .and_then(|username: String| clear_account_data(db, &username))
            .and_then(as_json),
        "saveGamesPage" => arg(args, 0, "username").and_then(|username: String| {
            let games: Vec<Value> = arg(args, 1, "games")?;
            let reviews: Vec<Value> = optional_arg(args, 2, "reviews")?.unwrap_or_default();
            save_games_page(db, &username, &games, &reviews, current(args, 3))
        }),
        "saveGames" => arg(args, 0, "username").and_then(|username: String| {
            let games: Vec<Value> = arg(args, 1, "games")?;
            let synced_at: Option<i64> = optional_arg(args, 2, "syncedAt")?;
            save_games(db, &username, &games, synced_at, current(args, 3))
        }),
        // Cache resets only drop in-memory state, which this port does not keep.
        "resetStore" => Ok(Value::Null),
        _ => return None,
    })
}

fn as_json<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string()))
}

fn arg<T: DeserializeOwned>(args: &[Value], index: usize, name: &str) -> Result<T> {
    serde_json::from_value(args.get(index).cloned().unwrap_or(Value::Null))
        .map_err(|cause| CoreError::new(format!("Invalid {name}: {cause}")))
}

/// An argument that may be absent or `null`.
fn optional_arg<T: DeserializeOwned>(
    args: &[Value],
    index: usize,
    name: &str,
) -> Result<Option<T>> {
    match args.get(index) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|cause| CoreError::new(format!("Invalid {name}: {cause}"))),
    }
}

/// `stillCurrent`, as the boolean the caller evaluated: absent means still current.
fn current(args: &[Value], index: usize) -> bool {
    !matches!(args.get(index), Some(Value::Bool(false)))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// A validator or domain rule from `kchess-domain`, by its TypeScript name.
fn domain(method: &str, args: Vec<Value>) -> Result<Value> {
    let text = serde_json::to_string(&args).map_err(|cause| CoreError::new(cause.to_string()))?;
    let output = api::call(method, &text).map_err(CoreError::new)?;
    serde_json::from_str(&output).map_err(|cause| CoreError::new(cause.to_string()))
}

/// `rulesLossless(name, value)` with no special values to carry.
fn validate(method: &str, value: &Value) -> Result<Value> {
    domain(method, vec![json!([]), value.clone()])
}

/// Runs `work` inside `BEGIN IMMEDIATE`, committing on success and rolling back on any failure.
fn transaction<T>(db: &Connection, work: impl FnOnce() -> Result<T>) -> Result<T> {
    db.execute_batch("BEGIN IMMEDIATE").sql()?;
    match work().and_then(|value| db.execute_batch("COMMIT").sql().map(|()| value)) {
        Ok(value) => Ok(value),
        Err(cause) => {
            let _ = db.execute_batch("ROLLBACK");
            Err(cause)
        }
    }
}

// ---- Values: SQLite and JavaScript semantics ------------------------------------------------

/// A JSON value bound as SQL, as `toSql` binds it: null and undefined as NULL, booleans as 1/0,
/// lists joined with commas, numbers as doubles.
fn to_sql(value: Option<&Value>) -> Result<SqlValue> {
    Ok(match value {
        None | Some(Value::Null) => SqlValue::Null,
        Some(Value::Bool(flag)) => SqlValue::Integer(i64::from(*flag)),
        Some(Value::Number(number)) => SqlValue::Real(number.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(text)) => SqlValue::Text(text.clone()),
        Some(Value::Array(items)) => SqlValue::Text(
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
        ),
        Some(other) => return Err(CoreError::new(format!("Unsupported value {other}."))),
    })
}

/// `String(v)` for a stored value (`null` gives an empty string where the caller defaults it).
fn text_of(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(text) => text.clone(),
        SqlValue::Integer(number) => number.to_string(),
        SqlValue::Real(number) if number.fract() == 0.0 && number.abs() < 1e21 => {
            format!("{number:.0}")
        }
        SqlValue::Real(number) => number.to_string(),
        SqlValue::Null | SqlValue::Blob(_) => String::new(),
    }
}

/// `Number(v)` for a stored value that is not null; NaN where JavaScript gives NaN.
fn number_of(value: &SqlValue) -> Option<f64> {
    match value {
        SqlValue::Integer(number) => Some(*number as f64),
        SqlValue::Real(number) => Some(*number),
        SqlValue::Text(text) => {
            let trimmed = text.trim();
            Some(if trimmed.is_empty() {
                0.0
            } else {
                trimmed.parse::<f64>().unwrap_or(f64::NAN)
            })
        }
        SqlValue::Null => None,
        SqlValue::Blob(_) => Some(f64::NAN),
    }
}

/// A stored number, and only a number (JavaScript's strict equality never matches text).
fn strict_number(value: &SqlValue) -> Option<f64> {
    match value {
        SqlValue::Integer(number) => Some(*number as f64),
        SqlValue::Real(number) => Some(*number),
        _ => None,
    }
}

/// `row.x === 1`.
fn is_one(value: &SqlValue) -> bool {
    strict_number(value) == Some(1.0)
}

/// `row.x !== 0`.
fn is_not_zero(value: &SqlValue) -> bool {
    strict_number(value) != Some(0.0)
}

// ---- Settings -------------------------------------------------------------------------------

fn settings_select() -> String {
    format!(
        "SELECT {} FROM settings WHERE id = 1",
        SETTINGS_KEYS.join(", ")
    )
}

/// `rowToSettings` followed by `normalizeSettings`.
fn settings_from_values(values: &[SqlValue]) -> Result<Settings> {
    let by_key: HashMap<&str, &SqlValue> =
        SETTINGS_KEYS.iter().copied().zip(values.iter()).collect();
    let column = |key: &str| by_key.get(key).copied().cloned().unwrap_or(SqlValue::Null);
    let text = |key: &str| text_of(&column(key));
    let poll = match column("correspondencePoll") {
        SqlValue::Null => 5.0,
        value => number_of(&value).unwrap_or(f64::NAN),
    };
    // `Number(x) || 0` turns NaN into 0, then the value is clamped to 0..=120.
    let poll = if poll.is_nan() { 0.0 } else { poll };
    let levels: Vec<String> = text("engineLevels").split(',').map(str::to_owned).collect();
    let engine_levels: Vec<String> =
        serde_json::from_value(domain("normalizeEngineLevels", vec![json!(levels)])?)
            .map_err(|cause| CoreError::new(cause.to_string()))?;
    let coordinates = text("coordinates");
    let promotion = text("promotion");
    let piece_animation = text("pieceAnimation");
    let review_auto = text("reviewAuto");
    let mut settings = Settings {
        appearance: text("appearance"),
        board_theme: text("boardTheme"),
        light_theme: text("lightTheme"),
        dark_theme: text("darkTheme"),
        piece_set: text("pieceSet"),
        piece_animation: if PIECE_ANIMATIONS.contains(&piece_animation.as_str()) {
            piece_animation
        } else {
            "normal".into()
        },
        coordinates: if matches!(coordinates.as_str(), "none" | "outside") {
            coordinates
        } else {
            "inside".into()
        },
        sound_enabled: is_one(&column("soundEnabled")),
        sound_volume: number_of(&column("soundVolume")).unwrap_or(f64::NAN),
        engine_path: text("enginePath"),
        premove: is_one(&column("premove")),
        promotion: if matches!(promotion.as_str(), "queen" | "premove") {
            promotion
        } else {
            "ask".into()
        },
        show_legal_moves: is_one(&column("showLegalMoves")),
        notifications_enabled: is_one(&column("notificationsEnabled")),
        notify_active: is_one(&column("notifyActive")),
        notify_background: is_one(&column("notifyBackground")),
        notify_opponent_move: is_one(&column("notifyOpponentMove")),
        notify_low_time: is_one(&column("notifyLowTime")),
        notify_game_events: is_one(&column("notifyGameEvents")),
        notify_computer_move: is_one(&column("notifyComputerMove")),
        notify_sound: is_one(&column("notifySound")),
        voice_push_to_talk: is_one(&column("voicePushToTalk")),
        voice_confirm_moves: is_not_zero(&column("voiceConfirmMoves")),
        voice_history: is_not_zero(&column("voiceHistory")),
        update_auto_check: is_not_zero(&column("updateAutoCheck")),
        update_auto_download: is_not_zero(&column("updateAutoDownload")),
        update_install_on_quit: is_not_zero(&column("updateInstallOnQuit")),
        engine_levels,
        review_auto: if REVIEW_AUTO.contains(&review_auto.as_str()) {
            review_auto
        } else {
            "recent".into()
        },
        review_on_battery: is_one(&column("reviewOnBattery")),
        receive_challenges: is_not_zero(&column("receiveChallenges")),
        notify_challenges: is_not_zero(&column("notifyChallenges")),
        online_chat: is_not_zero(&column("onlineChat")),
        correspondence_poll: poll.clamp(0.0, 120.0),
        zen_mode: is_one(&column("zenMode")),
        blindfold: is_one(&column("blindfold")),
        cloud_eval: is_one(&column("cloudEval")),
        show_opening_name: is_not_zero(&column("showOpeningName")),
        swipe_navigation: is_not_zero(&column("swipeNavigation")),
        swipe_indicator: is_not_zero(&column("swipeIndicator")),
    };
    if settings.sound_volume > 1.0 {
        settings.sound_volume = (settings.sound_volume / 100.0).min(1.0);
    }
    Ok(settings)
}

fn read_settings(db: &Connection) -> Result<Option<Settings>> {
    let values: Option<Vec<SqlValue>> = db
        .query_row(&settings_select(), [], |row| {
            (0..SETTINGS_KEYS.len())
                .map(|index| row.get::<_, SqlValue>(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .optional()
        .sql()?;
    values
        .map(|values| settings_from_values(&values))
        .transpose()
}

fn write_settings(db: &Connection, settings: &Settings) -> Result<()> {
    let object =
        serde_json::to_value(settings).map_err(|cause| CoreError::new(cause.to_string()))?;
    let params = SETTINGS_KEYS
        .iter()
        .map(|key| to_sql(object.get(*key)))
        .collect::<Result<Vec<_>>>()?;
    let placeholders = vec!["?"; SETTINGS_KEYS.len()].join(", ");
    let sql = format!(
        "INSERT OR REPLACE INTO settings (id, {}) VALUES (1, {placeholders})",
        SETTINGS_KEYS.join(", ")
    );
    db.execute(&sql, params_from_iter(params)).sql()?;
    Ok(())
}

fn get_settings(db: &Connection) -> Result<Value> {
    as_json(read_settings(db)?.unwrap_or_else(default_settings))
}

fn load_app_data(db: &Connection) -> Result<AppData> {
    let mut statement = db
        .prepare("SELECT username, connected, lastSyncedAt FROM accounts ORDER BY rowid")
        .sql()?;
    let accounts = statement
        .query_map([], |row| {
            Ok(Account {
                username: row.get(0)?,
                connected: row.get::<_, i64>(1)? == 1,
                last_synced_at: row.get(2)?,
            })
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    let game_count = db
        .query_row("SELECT COUNT(*) AS total FROM games", [], |row| row.get(0))
        .sql()?;
    Ok(AppData {
        settings: read_settings(db)?.unwrap_or_else(default_settings),
        accounts,
        game_count,
    })
}

// ---- Games ----------------------------------------------------------------------------------

fn game_from_row(row: &Row<'_>) -> rusqlite::Result<LichessGame> {
    Ok(LichessGame {
        id: row.get("id")?,
        account: row.get("account")?,
        created_at: row.get("createdAt")?,
        last_move_at: row.get("lastMoveAt")?,
        rated: row.get::<_, i64>("rated")? == 1,
        speed: row.get("speed")?,
        perf: row.get("perf")?,
        status: row.get("status")?,
        winner: row
            .get::<_, Option<String>>("winner")?
            .filter(|winner| winner == "white" || winner == "black"),
        color: if row.get::<_, String>("color")? == "black" {
            "black".into()
        } else {
            "white".into()
        },
        opponent: row.get("opponent")?,
        opponent_rating: row.get("opponentRating")?,
        player_rating: row.get("playerRating")?,
        rating_diff: row.get("ratingDiff")?,
        opening: row.get("opening")?,
        moves: row.get("moves")?,
        pgn: None,
    })
}

/// `gamePage`: a bounded page of list rows, with the total for the same filters.
fn game_page(db: &Connection, input: Value) -> Result<Value> {
    let query: GamePageQuery =
        serde_json::from_value(domain("assertGamePageQuery", vec![json!([]), input])?)
            .map_err(|cause| CoreError::new(cause.to_string()))?;
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    if let Some(account) = query
        .account
        .as_deref()
        .filter(|account| !account.is_empty())
    {
        clauses.push("account = ? COLLATE NOCASE".into());
        params.push(SqlValue::Text(account.to_owned()));
    }
    if let Some(result) = query.result.as_deref().filter(|result| !result.is_empty()) {
        clauses.push(format!("({RESULT_SQL}) = ?"));
        params.push(SqlValue::Text(result.to_owned()));
    }
    if let Some(rated) = query.rated {
        clauses.push("rated = ?".into());
        params.push(SqlValue::Integer(i64::from(rated)));
    }
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let total: i64 = db
        .query_row(
            &format!("SELECT COUNT(*) AS total FROM games {filter}"),
            params_from_iter(params.iter().cloned()),
            |row| row.get(0),
        )
        .sql()?;
    let mut page_params = params;
    page_params.push(SqlValue::Integer(query.limit as i64));
    page_params.push(SqlValue::Integer(query.offset as i64));
    let mut statement = db
        .prepare(&format!(
            "SELECT {LIST_COLUMNS} FROM games {filter} ORDER BY createdAt DESC, account, id LIMIT ? OFFSET ?"
        ))
        .sql()?;
    let games = statement
        .query_map(params_from_iter(page_params), game_from_row)
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    as_json(GamePage { games, total })
}

fn game_record(row: &Row<'_>) -> rusqlite::Result<(String, GameRecord)> {
    Ok((
        row.get("name")?,
        GameRecord {
            total: row.get("total")?,
            win: row.get::<_, Option<i64>>("win")?.unwrap_or(0),
            loss: row.get("loss")?,
            draw: row.get("draw")?,
        },
    ))
}

fn game_records(db: &Connection, sql: &str) -> Result<BTreeMap<String, GameRecord>> {
    let mut statement = db.prepare(sql).sql()?;
    let records = statement
        .query_map([], game_record)
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    Ok(records.into_iter().collect())
}

fn game_library_overview(db: &Connection) -> Result<GameLibraryOverview> {
    const AGGREGATES: &str = "COUNT(*) AS total,
    SUM(winner = color) AS win,
    SUM(winner IS NOT NULL AND winner != color) AS loss,
    SUM(winner IS NULL) AS draw";
    Ok(GameLibraryOverview {
        by_account: game_records(
            db,
            &format!(
                "SELECT lower(account) AS name, {AGGREGATES} FROM games GROUP BY lower(account)"
            ),
        )?,
        // Only tracked opponents need a card; never transfer the entire opponent list.
        versus: game_records(
            db,
            &format!(
                "SELECT lower(opponent) AS name, {AGGREGATES} FROM games
      WHERE account IN (SELECT username FROM accounts WHERE connected = 1)
        AND opponent COLLATE NOCASE IN (SELECT username FROM accounts)
      GROUP BY lower(opponent)"
            ),
        )?,
    })
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month 1-12, day), as `Date` reads them in UTC.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `gameRatingHistory`: the last rated rating each day, per variant.
fn rating_history(db: &Connection, account: &str) -> Result<Vec<RatingSeries>> {
    let mut statement = db
        .prepare(
            "SELECT perf, day, rating FROM (
    SELECT perf, CAST(createdAt / 86400000 AS INTEGER) * 86400000 AS day,
      playerRating + ratingDiff AS rating,
      ROW_NUMBER() OVER (PARTITION BY perf, CAST(createdAt / 86400000 AS INTEGER)
        ORDER BY createdAt DESC, id DESC) AS rank
    FROM games WHERE account = ? COLLATE NOCASE AND rated = 1
      AND playerRating IS NOT NULL AND ratingDiff IS NOT NULL
  ) WHERE rank = 1 ORDER BY day, perf",
        )
        .sql()?;
    let rows = statement
        .query_map([account], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    let mut series: Vec<RatingSeries> = Vec::new();
    for (perf, day, rating) in rows {
        let (year, month, date) = civil_from_days(day.div_euclid(86_400_000));
        let point = [year, month - 1, date, rating];
        match series.iter().position(|entry| entry.name == perf) {
            Some(index) => series[index].points.push(point),
            None => series.push(RatingSeries {
                name: perf,
                points: vec![point],
            }),
        }
    }
    Ok(series)
}

fn pending_game_ids(db: &Connection, account: &str) -> Result<Vec<String>> {
    let mut statement = db
        .prepare("SELECT id FROM pending_game_sync WHERE account = ? COLLATE NOCASE ORDER BY id")
        .sql()?;
    statement
        .query_map([account], |row| row.get(0))
        .sql()?
        .collect::<rusqlite::Result<Vec<String>>>()
        .sql()
}

fn game_pgn(db: &Connection, account: &str, id: &str) -> Result<Value> {
    let pgn = db
        .query_row(
            "SELECT pgn FROM games WHERE account = ? COLLATE NOCASE AND id = ?",
            params![account, id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .sql()?;
    Ok(json!(pgn.flatten()))
}

/// `readApiCache`: a cached document, or null when absent or unreadable.
fn read_api_cache(db: &Connection, key: &str) -> Result<Value> {
    let row = db
        .query_row(
            "SELECT value, fetchedAt FROM api_cache WHERE key = ?",
            [key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .sql()?;
    Ok(match row {
        Some((text, fetched_at)) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => json!({ "value": value, "fetchedAt": fetched_at }),
            Err(_) => Value::Null,
        },
        None => Value::Null,
    })
}

fn write_api_cache(
    db: &Connection,
    key: &str,
    value: &Value,
    still_current: bool,
) -> Result<Value> {
    if !still_current {
        return Ok(Value::Null);
    }
    db.execute(
        "INSERT OR REPLACE INTO api_cache (key, value, fetchedAt) VALUES (?, ?, ?)",
        params![key, value.to_string(), now_ms()],
    )
    .sql()?;
    Ok(Value::Null)
}

fn add_account(db: &Connection, username: &str, connected: bool) -> Result<Value> {
    let normalized: String = serde_json::from_value(validate("assertUsername", &json!(username))?)
        .map_err(|cause| CoreError::new(cause.to_string()))?;
    let existing = db
        .query_row(
            "SELECT username, connected FROM accounts WHERE username = ? COLLATE NOCASE",
            [&normalized],
            |row| row.get::<_, i64>(1),
        )
        .optional()
        .sql()?;
    match existing {
        Some(was_connected) => {
            if connected && was_connected != 1 {
                db.execute(
                    "UPDATE accounts SET connected = 1 WHERE username = ? COLLATE NOCASE",
                    [&normalized],
                )
                .sql()?;
            }
        }
        None => {
            db.execute(
                "INSERT INTO accounts (username, connected) VALUES (?, ?)",
                params![normalized, i64::from(connected)],
            )
            .sql()?;
        }
    }
    // Adding someone on purpose overrides an earlier removal.
    db.execute(
        "DELETE FROM dismissed_friends WHERE username = ? COLLATE NOCASE",
        [&normalized],
    )
    .sql()?;
    as_json(load_app_data(db)?)
}

/// `addFriends`: follow many players at once; validation failure rolls the whole batch back.
fn add_friends(db: &Connection, usernames: &[Value]) -> Result<AppData> {
    transaction(db, || {
        for raw in usernames {
            let name: String = serde_json::from_value(validate("assertUsername", raw)?)
                .map_err(|cause| CoreError::new(cause.to_string()))?;
            let exists = db
                .query_row(
                    "SELECT 1 FROM accounts WHERE username = ? COLLATE NOCASE",
                    [&name],
                    |_| Ok(()),
                )
                .optional()
                .sql()?;
            if exists.is_none() {
                db.execute(
                    "INSERT INTO accounts (username, connected) VALUES (?, 0)",
                    [&name],
                )
                .sql()?;
            }
            db.execute(
                "DELETE FROM dismissed_friends WHERE username = ? COLLATE NOCASE",
                [&name],
            )
            .sql()?;
        }
        Ok(())
    })?;
    load_app_data(db)
}

fn dismissed_friends(db: &Connection) -> Result<Vec<String>> {
    let mut statement = db.prepare("SELECT username FROM dismissed_friends").sql()?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    Ok(names.iter().map(|name| name.to_lowercase()).collect())
}

/// Everything stored for one account's games, inside the caller's transaction: games, pending
/// syncs, cached profile data, its own explorer lookups, and any review or check left without a
/// game. A game another tracked account also played keeps its reviews.
fn purge_account_data(db: &Connection, account: &str, usage: bool) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM games WHERE account = ? COLLATE NOCASE",
        [account],
    )?;
    db.execute(
        "DELETE FROM pending_game_sync WHERE account = ? COLLATE NOCASE",
        [account],
    )?;
    // `length` is JavaScript's UTF-16 length, as the TypeScript passes it.
    let prefix_length = i64::try_from(account.encode_utf16().count() + 1).unwrap_or(i64::MAX);
    db.execute(
        "DELETE FROM api_cache WHERE substr(key, 1, ?) = ? COLLATE NOCASE",
        params![prefix_length, format!("{account}:")],
    )?;
    // Explorer keys hold the options as JSON; ASCII LIKE ignores case, `_` in names is escaped.
    let mut escaped = String::new();
    for character in account.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    db.execute(
        r"DELETE FROM position_lookups WHERE key LIKE ? ESCAPE '\'",
        [format!("player:%\"player\":\"{escaped}\"%")],
    )?;
    if usage {
        db.execute(
            "DELETE FROM usage WHERE account = ? COLLATE NOCASE",
            [account],
        )?;
    }
    db.execute(
        "DELETE FROM lichess_review_checks WHERE NOT EXISTS (SELECT 1 FROM games WHERE games.id = lichess_review_checks.id)",
        [],
    )?;
    db.execute(
        "DELETE FROM game_reviews WHERE NOT EXISTS (SELECT 1 FROM games WHERE games.id = game_reviews.gameId)",
        [],
    )?;
    db.execute(
        "DELETE FROM reviews WHERE NOT EXISTS (SELECT 1 FROM game_reviews WHERE game_reviews.reviewKey = reviews.key)",
        [],
    )?;
    Ok(())
}

fn logout_accounts(db: &Connection, username: Option<&str>) -> Result<AppData> {
    transaction(db, || {
        let filter = username.map_or(SqlValue::Null, |name| SqlValue::Text(name.to_owned()));
        let accounts = {
            let mut statement = db
                .prepare(
                    "SELECT username FROM accounts WHERE connected = 1 AND (? IS NULL OR username = ? COLLATE NOCASE)",
                )
                .sql()?;
            statement
                .query_map(params_from_iter([filter.clone(), filter]), |row| {
                    row.get::<_, String>(0)
                })
                .sql()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .sql()?
        };
        for account in &accounts {
            purge_account_data(db, account, true).sql()?;
            db.execute(
                "DELETE FROM tokens WHERE username = ? COLLATE NOCASE",
                [account],
            )
            .sql()?;
            db.execute(
                "DELETE FROM accounts WHERE username = ? COLLATE NOCASE",
                [account],
            )
            .sql()?;
        }
        if username.is_none() {
            db.execute("DELETE FROM tokens", []).sql()?;
        }
        Ok(())
    })?;
    load_app_data(db)
}

fn remove_account(db: &Connection, username: &str) -> Result<AppData> {
    transaction(db, || {
        // Only friends are remembered as removed; disconnecting one's own account is not a "no thanks".
        let connected = db
            .query_row(
                "SELECT connected FROM accounts WHERE username = ? COLLATE NOCASE",
                [username],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .sql()?;
        if connected == Some(0) {
            db.execute(
                "INSERT OR REPLACE INTO dismissed_friends (username, dismissedAt) VALUES (?, ?)",
                params![username, now_ms()],
            )
            .sql()?;
        }
        purge_account_data(db, username, true).sql()?;
        db.execute(
            "DELETE FROM tokens WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        db.execute(
            "DELETE FROM accounts WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        Ok(())
    })?;
    load_app_data(db)
}

/// Deletes what was downloaded for one account but keeps the account; the cursor resets so the next
/// sync fetches everything again.
fn clear_account_data(db: &Connection, username: &str) -> Result<AppData> {
    transaction(db, || {
        // The account and its data-usage history stay; only what was downloaded goes.
        purge_account_data(db, username, false).sql()?;
        db.execute(
            "UPDATE accounts SET lastSyncedAt = NULL WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        Ok(())
    })?;
    load_app_data(db)
}

/// Whether a game is still live, as `isGameInProgress` decides it.
fn in_progress(game: &Value) -> Result<bool> {
    domain(
        "isGameInProgress",
        vec![game.get("status").cloned().unwrap_or(Value::Null)],
    )?
    .as_bool()
    .ok_or_else(|| CoreError::new("isGameInProgress did not return a boolean."))
}

fn upsert_sql() -> String {
    let updates = GAME_KEYS
        .iter()
        .filter(|key| **key != "account" && **key != "id")
        .map(|key| format!("{key} = excluded.{key}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO games ({}) VALUES ({}) ON CONFLICT(account, id) DO UPDATE SET {updates}",
        GAME_KEYS.join(", "),
        vec!["?"; GAME_KEYS.len()].join(", ")
    )
}

/// `upsertGames`: one batch of games, and the account's sync marker, in a single transaction.
fn upsert_games(
    db: &Connection,
    username: &str,
    games: &[Value],
    synced_at: Option<i64>,
    reviews: &[Value],
) -> Result<()> {
    transaction(db, || {
        let exists = db
            .query_row(
                "SELECT 1 FROM accounts WHERE username = ? COLLATE NOCASE",
                [username],
                |_| Ok(()),
            )
            .optional()
            .sql()?;
        if exists.is_none() {
            return Err(CoreError::new("This account was removed during sync."));
        }
        let mut upsert = db.prepare(&upsert_sql()).sql()?;
        for game in games {
            if in_progress(game)? {
                db.execute(
                    "INSERT OR IGNORE INTO pending_game_sync (account, id) VALUES (?, ?)",
                    params_from_iter([
                        SqlValue::Text(username.to_owned()),
                        to_sql(game.get("id"))?,
                    ]),
                )
                .sql()?;
            } else {
                let params = GAME_KEYS
                    .iter()
                    .map(|key| to_sql(game.get(*key)))
                    .collect::<Result<Vec<_>>>()?;
                upsert.execute(params_from_iter(params)).sql()?;
                db.execute(
                    "DELETE FROM pending_game_sync WHERE account = ? COLLATE NOCASE AND id = ?",
                    params_from_iter([
                        SqlValue::Text(username.to_owned()),
                        to_sql(game.get("id"))?,
                    ]),
                )
                .sql()?;
            }
        }
        if !reviews.is_empty() {
            return Err(CoreError::new(
                "Lichess reviews cannot be saved by the Rust store yet.",
            ));
        }
        // Trim once per completed sync rather than after every page.
        if let Some(synced_at) = synced_at {
            db.execute(
                &format!(
                    "DELETE FROM games WHERE account = ? AND rowid NOT IN (SELECT rowid FROM games WHERE account = ? ORDER BY createdAt DESC LIMIT {MAX_GAMES})"
                ),
                [username, username],
            )
            .sql()?;
            db.execute(
                "UPDATE accounts SET lastSyncedAt = ? WHERE username = ? COLLATE NOCASE",
                params![synced_at, username],
            )
            .sql()?;
        }
        Ok(())
    })
}

fn save_games_page(
    db: &Connection,
    username: &str,
    games: &[Value],
    reviews: &[Value],
    still_current: bool,
) -> Result<Value> {
    if !still_current {
        return Err(CoreError::new("Sync was cancelled."));
    }
    upsert_games(db, username, games, None, reviews)?;
    Ok(Value::Null)
}

fn save_games(
    db: &Connection,
    username: &str,
    games: &[Value],
    synced_at: Option<i64>,
    still_current: bool,
) -> Result<Value> {
    if !still_current {
        return Err(CoreError::new("Sync was cancelled."));
    }
    upsert_games(
        db,
        username,
        games,
        Some(synced_at.unwrap_or_else(now_ms)),
        &[],
    )?;
    as_json(load_app_data(db)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        db
    }

    /// `game-library.test.ts`: result filters are served from the result indexes.
    #[test]
    fn result_filters_use_the_result_indexes() {
        let db = database();
        let plan = |sql: &str, params: Vec<&str>| -> String {
            let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            statement
                .query_map(params_from_iter(params), |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join(" | ")
        };
        assert!(
            plan(
                &format!("SELECT COUNT(*) FROM games WHERE ({RESULT_SQL}) = ?"),
                vec!["loss"]
            )
            .contains("idx_games_result_page")
        );
        assert!(
            plan(
                &format!(
                    "SELECT COUNT(*) FROM games WHERE account = ? COLLATE NOCASE AND ({RESULT_SQL}) = ?"
                ),
                vec!["alice", "loss"]
            )
            .contains("idx_games_account_result_page")
        );
    }

    #[test]
    fn civil_dates_match_utc_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2023-10-01T00:00:00Z
        assert_eq!(
            civil_from_days(1_696_118_400_000 / 86_400_000),
            (2023, 10, 1)
        );
        // 2000-02-29 (a leap day) and 1969-12-31.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn a_fresh_database_reads_the_default_settings() {
        let db = database();
        assert_eq!(read_settings(&db).unwrap(), None);
        let defaults = default_settings();
        assert_eq!(
            serde_json::to_value(&defaults).unwrap()["engineLevels"],
            json!(["beginner", "club", "expert", "fm", "im", "gm", "max"])
        );
        assert_eq!(get_settings(&db).unwrap(), json!(defaults));
    }

    #[test]
    fn stored_settings_are_normalized_as_the_row_mapper_did() {
        let db = database();
        db.execute_batch(
            "INSERT INTO settings (id, appearance, boardTheme, coordinates, soundEnabled, soundVolume, enginePath, promotion, pieceAnimation, reviewAuto, engineLevels, correspondencePoll)
               VALUES (1, 'dark', 'brown', 'bogus', 1, 80, '/x', 'rook', 'warp', 'sometimes', 'club,nonsense', 900);",
        )
        .unwrap();
        let settings = read_settings(&db).unwrap().unwrap();
        assert_eq!(settings.coordinates, "inside");
        assert_eq!(settings.promotion, "ask");
        assert_eq!(settings.piece_animation, "normal");
        assert_eq!(settings.review_auto, "recent");
        assert_eq!(settings.sound_volume, 0.8);
        assert_eq!(settings.correspondence_poll, 120.0);
        assert_eq!(settings.engine_levels, ["club"]);
    }
}
