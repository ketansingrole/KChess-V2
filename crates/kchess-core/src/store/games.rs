//! Settings, accounts, the game library, the Lichess API cache and the account lifecycle in
//! `kchess.db`. Logins arrive already encrypted: the caller seals them with the operating system's
//! credential store.

use crate::time::now_ms;
use std::collections::{BTreeMap, HashMap};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::StoreContext;
use super::sql::{Sql as _, transaction};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use kchess_domain::calendar::civil_from_days;
use kchess_domain::misc::{validate, value::J};

/// Games kept per account.
pub const MAX_GAMES: i64 = 5000;

/// Must match the indexed expression in `migrations.rs` verbatim, or SQLite falls back to a scan.
pub const RESULT_SQL: &str =
    "CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END";

/// Every persisted setting, in column order (`SETTINGS_KEYS`).
const SETTINGS_KEYS: [&str; 43] = [
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
    "puzzleAutoNext",
    "confirmCorrespondence",
    "figurineNotation",
];

/// Every persisted game field, in column order (`GAME_KEYS`).
pub(super) const GAME_KEYS: [&str; 17] = [
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

/// `DEFAULT_SETTINGS`.
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
    pub puzzle_auto_next: bool,
    pub confirm_correspondence: bool,
    pub figurine_notation: bool,
}

pub(super) fn default_settings() -> Settings {
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
        puzzle_auto_next: false,
        confirm_correspondence: true,
        figurine_notation: false,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_play_time: Option<i64>,
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
    search: Option<String>,
    // Validated whole numbers; the validator hands them back as JSON numbers, which may be floats.
    offset: f64,
    limit: f64,
}

pub fn as_json<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string()))
}

/// A legacy token that could not be sealed again: the token goes, the account and games stay.
pub fn disconnect_account(db: &Connection, username: &str) -> Result<()> {
    transaction(db, || {
        db.execute(
            "UPDATE accounts SET connected = 0 WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        db.execute(
            "DELETE FROM tokens WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        Ok(())
    })
}

/// Stores a sealed token from the one-time import, without touching its account.
pub fn import_token_ciphertext(
    db: &Connection,
    username: Option<&str>,
    encrypted: &str,
) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)",
        params![username, encrypted],
    )
    .sql()?;
    Ok(())
}

// ---- Values: SQLite and JavaScript semantics ------------------------------------------------

/// A JSON value bound as SQL, as `toSql` binds it: null and undefined as NULL, booleans as 1/0,
/// lists joined with commas, numbers as doubles.
pub(super) fn to_sql(value: Option<&Value>) -> Result<SqlValue> {
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
    let engine_levels =
        kchess_domain::training::engine_levels::normalize(&levels).map_err(CoreError::new)?;
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
        puzzle_auto_next: is_not_zero(&column("puzzleAutoNext")),
        confirm_correspondence: is_not_zero(&column("confirmCorrespondence")),
        figurine_notation: is_one(&column("figurineNotation")),
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

/// Stores every setting; the settings as stored.
pub fn save_settings(db: &Connection, settings: &Settings) -> Result<Value> {
    let object = as_json(settings)?;
    write_settings_json(db, &object)?;
    Ok(object)
}

/// `writeSettings`: every persisted setting of a settings object, as `toSql` binds it.
pub(super) fn write_settings_json(db: &Connection, object: &Value) -> Result<()> {
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

pub fn get_settings(db: &Connection) -> Result<Value> {
    as_json(read_settings(db)?.unwrap_or_else(default_settings))
}

pub fn load_app_data(db: &Connection) -> Result<AppData> {
    let mut statement = db
        .prepare(
            "SELECT username, connected, lastSyncedAt, syncedPlayTime FROM accounts ORDER BY rowid",
        )
        .sql()?;
    let accounts = statement
        .query_map([], |row| {
            Ok(Account {
                username: row.get(0)?,
                connected: row.get::<_, i64>(1)? == 1,
                last_synced_at: row.get(2)?,
                synced_play_time: row.get(3)?,
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

/// `saveLogin`'s write: the stored login and its connected account, in one transaction. The token
/// arrives already encrypted; the caller encrypts it with the operating system's credential store.
pub fn save_login(db: &Connection, username: &str, encrypted: &str) -> Result<AppData> {
    transaction(db, || {
        db.execute(
            "INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)",
            params![username, encrypted],
        )
        .sql()?;
        let updated = db
            .execute(
                "UPDATE accounts SET connected = 1 WHERE username = ? COLLATE NOCASE",
                params![username],
            )
            .sql()?;
        if updated == 0 {
            db.execute(
                "INSERT INTO accounts (username, connected) VALUES (?, 1)",
                params![username],
            )
            .sql()?;
        }
        db.execute(
            "DELETE FROM dismissed_friends WHERE username = ? COLLATE NOCASE",
            params![username],
        )
        .sql()?;
        Ok(())
    })?;
    load_app_data(db)
}

/// The sealed token stored for an account.
pub fn token_ciphertext(db: &Connection, username: &str) -> Result<Option<String>> {
    db.query_row(
        "SELECT encrypted FROM tokens WHERE username = ? COLLATE NOCASE",
        [username],
        |row| row.get(0),
    )
    .optional()
    .sql()
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
pub fn game_page(ctx: &StoreContext, input: Value) -> Result<Value> {
    let db = ctx.db;
    let query: GamePageQuery =
        crate::engine::validated(validate::assert_game_page_query(J::from(&input)))?;
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
    if let Some(search) = &query.search {
        // Any part of the opponent's name or the opening's, any case; `%` and `_` are literal.
        let escaped = search
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        clauses.push(
            "(opponent LIKE ? ESCAPE '\\' OR COALESCE(opening, '') LIKE ? ESCAPE '\\')".into(),
        );
        params.push(SqlValue::Text(pattern.clone()));
        params.push(SqlValue::Text(pattern));
    }
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    // Counting the filter walks the whole indexed range; the page itself is a short slice of it.
    // The same filters are re-counted on every page and result toggle, so keep the total until a
    // call that can change the games runs.
    let count_key = format!(
        "{}|{}|{}|{}",
        query
            .account
            .as_deref()
            .filter(|account| !account.is_empty())
            .unwrap_or(""),
        query
            .result
            .as_deref()
            .filter(|result| !result.is_empty())
            .unwrap_or(""),
        query
            .rated
            .map_or("any".to_string(), |rated| rated.to_string()),
        query.search.as_deref().unwrap_or("")
    );
    let total: i64 = match ctx.cache.count(&count_key) {
        Some(total) => total,
        None => {
            let total = db
                .query_row(
                    &format!("SELECT COUNT(*) AS total FROM games {filter}"),
                    params_from_iter(params.iter().cloned()),
                    |row| row.get(0),
                )
                .sql()?;
            ctx.cache.remember_count(count_key, total);
            total
        }
    };
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

pub fn game_library_overview(ctx: &StoreContext) -> Result<Value> {
    if let Some(cached) = ctx.cache.overview() {
        return Ok(cached);
    }
    let db = ctx.db;
    const AGGREGATES: &str = "COUNT(*) AS total,
    SUM(winner = color) AS win,
    SUM(winner IS NOT NULL AND winner != color) AS loss,
    SUM(winner IS NULL) AS draw";
    let overview = GameLibraryOverview {
        by_account: game_records(
            db,
            &format!(
                // Group on the indexed `account` with the same NOCASE collation as the lower()
                // grouping, so SQLite walks idx_games_account_created instead of building a
                // temporary B-tree over every game.
                "SELECT lower(account) AS name, {AGGREGATES} FROM games GROUP BY account COLLATE NOCASE"
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
    };
    let value = as_json(overview)?;
    ctx.cache.remember_overview(&value);
    Ok(value)
}

/// `gameRatingHistory`: the last rated rating each day, per variant. A Chess.com game records the
/// player's rating after it and no change, so its rating stands as it is.
pub fn rating_history(db: &Connection, account: &str) -> Result<Vec<RatingSeries>> {
    let after_game = kchess_domain::chesscom::username_of(account).is_some();
    let mut statement = db
        .prepare(
            "SELECT perf, day, rating FROM (
    SELECT perf, CAST(createdAt / 86400000 AS INTEGER) * 86400000 AS day,
      playerRating + COALESCE(ratingDiff, 0) AS rating,
      ROW_NUMBER() OVER (PARTITION BY perf, CAST(createdAt / 86400000 AS INTEGER)
        ORDER BY createdAt DESC, id DESC) AS rank
    FROM games WHERE account = ?1 COLLATE NOCASE AND rated = 1
      AND playerRating IS NOT NULL AND (ratingDiff IS NOT NULL OR ?2)
  ) WHERE rank = 1 ORDER BY day, perf",
        )
        .sql()?;
    let rows = statement
        .query_map(params![account, after_game], |row| {
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
        let Some((year, month, date)) = civil_from_days(day.div_euclid(86_400_000)) else {
            continue;
        };
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

pub fn pending_game_ids(db: &Connection, account: &str) -> Result<Vec<String>> {
    let mut statement = db
        .prepare("SELECT id FROM pending_game_sync WHERE account = ? COLLATE NOCASE ORDER BY id")
        .sql()?;
    statement
        .query_map([account], |row| row.get(0))
        .sql()?
        .collect::<rusqlite::Result<Vec<String>>>()
        .sql()
}

pub fn game_pgn(db: &Connection, account: &str, id: &str) -> Result<Option<String>> {
    let pgn = db
        .query_row(
            "SELECT pgn FROM games WHERE account = ? COLLATE NOCASE AND id = ?",
            params![account, id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .sql()?;
    Ok(pgn.flatten())
}

/// A cached document as `{ value, fetchedAt }`, or null when absent or unreadable.
pub fn read_api_cache(host: &dyn Host, db: &Connection, key: &str) -> Result<Value> {
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
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "store",
                    &format!("API cache entry is invalid: {key} {cause}"),
                );
                Value::Null
            }
        },
        None => Value::Null,
    })
}

/// The `api_cache` keys of games kept from `exportGame`, so they open without a connection.
pub const EXPORTED_GAME_CACHE: &str = "exportGame:";
/// How many of those are kept, newest first.
const EXPORTED_GAMES_KEPT: i64 = 200;

pub fn write_api_cache(db: &Connection, key: &str, value: &Value) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO api_cache (key, value, fetchedAt) VALUES (?, ?, ?)",
        params![key, value.to_string(), now_ms()],
    )
    .sql()?;
    if key.starts_with(EXPORTED_GAME_CACHE) {
        let pattern = format!("{EXPORTED_GAME_CACHE}%");
        db.execute(
            "DELETE FROM api_cache WHERE key LIKE ?1 AND key NOT IN (
               SELECT key FROM api_cache WHERE key LIKE ?1 ORDER BY fetchedAt DESC, rowid DESC LIMIT ?2)",
            params![pattern, EXPORTED_GAMES_KEPT],
        )
        .sql()?;
    }
    Ok(())
}

pub fn add_account(db: &Connection, username: &str, connected: bool) -> Result<AppData> {
    let normalized =
        validate::assert_username(J::Str(username.to_owned())).map_err(CoreError::new)?;
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
    load_app_data(db)
}

/// Follows many players at once; validation failure rolls the whole batch back.
pub fn add_friends(db: &Connection, usernames: &[Value]) -> Result<AppData> {
    transaction(db, || {
        for raw in usernames {
            let name = validate::assert_username(J::from(raw)).map_err(CoreError::new)?;
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

pub fn dismissed_friends(db: &Connection) -> Result<Vec<String>> {
    let mut statement = db.prepare("SELECT username FROM dismissed_friends").sql()?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .sql()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .sql()?;
    Ok(names.iter().map(|name| name.to_lowercase()).collect())
}

/// Everything stored for one account's games, inside the caller's transaction: games, pending
/// syncs, cached profile data, its own explorer lookups, the reviews of its games, and any review
/// check left without a game. A game another tracked account also played keeps its reviews.
pub(super) fn purge_account_data(
    db: &Connection,
    account: &str,
    usage: bool,
) -> rusqlite::Result<()> {
    // The reviews of this account's games go with them, unless a game that stays still links
    // one. A game goes when only this account has it (another may share it). Reviews of local
    // games and of the analysis board link no synced game: they stay.
    let gone = |column: &str| {
        format!(
            "(EXISTS (SELECT 1 FROM games g WHERE g.id = {column} AND g.account = ?1 COLLATE NOCASE)
              AND NOT EXISTS (SELECT 1 FROM games g WHERE g.id = {column} AND g.account <> ?1 COLLATE NOCASE))"
        )
    };
    db.execute(
        &format!(
            "DELETE FROM reviews WHERE key IN (SELECT reviewKey FROM game_reviews WHERE {})
               AND NOT EXISTS (SELECT 1 FROM game_reviews other
                                WHERE other.reviewKey = reviews.key AND NOT {})",
            gone("game_reviews.gameId"),
            gone("other.gameId")
        ),
        [account],
    )?;
    db.execute(
        &format!(
            "DELETE FROM game_reviews WHERE {}",
            gone("game_reviews.gameId")
        ),
        [account],
    )?;
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
    // `"` and JSON escapes are not escaped: safe for Lichess usernames
    // (`[a-zA-Z0-9_-]`), brittle if the key format ever carries other values.
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
    Ok(())
}

pub fn logout_accounts(db: &Connection, username: Option<&str>) -> Result<AppData> {
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

pub fn remove_account(db: &Connection, username: &str) -> Result<AppData> {
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
pub fn clear_account_data(db: &Connection, username: &str) -> Result<AppData> {
    transaction(db, || {
        // The account and its data-usage history stay; only what was downloaded goes.
        purge_account_data(db, username, false).sql()?;
        db.execute(
            "UPDATE accounts SET lastSyncedAt = NULL, syncedPlayTime = NULL WHERE username = ? COLLATE NOCASE",
            [username],
        )
        .sql()?;
        Ok(())
    })?;
    load_app_data(db)
}

/// Whether a game is still live, by its `status`.
pub(super) fn in_progress(game: &Value) -> bool {
    kchess_domain::records::games::is_game_in_progress(game.get("status"))
}

pub(super) fn upsert_sql() -> String {
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
    host: &dyn Host,
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
            if in_progress(game) {
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
        for review in reviews {
            super::reviews::write_review(host, db, review)?;
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

/// One page of a sync: its games and their Lichess reviews.
pub fn save_games_page(
    host: &dyn Host,
    db: &Connection,
    username: &str,
    games: &[Value],
    reviews: &[Value],
) -> Result<()> {
    upsert_games(host, db, username, games, None, reviews)
}

/// The last page of a sync, which trims the account's games and marks it synced.
pub fn save_games(
    host: &dyn Host,
    db: &Connection,
    username: &str,
    games: &[Value],
    synced_at: Option<i64>,
) -> Result<AppData> {
    upsert_games(
        host,
        db,
        username,
        games,
        Some(synced_at.unwrap_or_else(now_ms)),
        &[],
    )?;
    load_app_data(db)
}

/// The player's Lichess play time as of their last sync, which a later profile is compared with.
pub fn save_synced_play_time(db: &Connection, username: &str, play_time: i64) -> Result<()> {
    db.execute(
        "UPDATE accounts SET syncedPlayTime = ? WHERE username = ? COLLATE NOCASE",
        params![play_time, username],
    )
    .sql()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        db
    }

    /// Clearing an account drops its games' reviews, but keeps the reviews of local games and
    /// of the analysis board, and a review another account's game still links.
    #[test]
    fn clearing_an_account_keeps_reviews_that_are_not_its_games() {
        let db = database();
        let review = |key: &str| {
            format!(
                "INSERT INTO reviews (key, source, complete, depth, data, summary, updatedAt)
                   VALUES ('{key}', 'local', 1, 18, '{{}}', '{{}}', 1);"
            )
        };
        let game = |account: &str, id: &str| {
            format!(
                "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status,
                                    color, opponent, moves)
                   VALUES ('{account}', '{id}', 1, 1, 1, 'blitz', 'blitz', 'mate', 'white', 'X', '');"
            )
        };
        db.execute_batch(
            &[
                review("board"),
                review("archived"),
                review("alice"),
                review("shared"),
                game("Alice", "Alice001"),
                game("Bob", "BobGame1"),
                "INSERT INTO game_reviews VALUES ('Alice001', 'alice'), ('Local001', 'archived'),
                 ('Alice001', 'shared'), ('BobGame1', 'shared');"
                    .to_string(),
            ]
            .concat(),
        )
        .unwrap();
        purge_account_data(&db, "alice", true).unwrap();
        let mut kept: Vec<String> = db
            .prepare("SELECT key FROM reviews ORDER BY key")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        kept.sort();
        assert_eq!(kept, ["archived", "board", "shared"]);
        let links: i64 = db
            .query_row("SELECT count(*) FROM game_reviews", [], |row| row.get(0))
            .unwrap();
        assert_eq!(links, 2, "the archived game's and Bob's links stay");
    }

    /// Result filters are served from the result indexes.
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
    fn a_fresh_database_reads_the_default_settings() {
        let db = database();
        assert_eq!(read_settings(&db).unwrap(), None);
        let defaults = default_settings();
        assert_eq!(
            serde_json::to_value(&defaults).unwrap()["engineLevels"],
            json!(["beginner", "club", "expert", "fm", "im", "gm", "max"])
        );
        assert_eq!(get_settings(&db).unwrap(), json!(defaults));
        // A solved puzzle stays on the board until Next (or Return), as on Lichess.
        assert!(!defaults.puzzle_auto_next);
        // A correspondence move cannot be taken back, so it waits for Confirm, as on Lichess.
        assert!(defaults.confirm_correspondence);
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
