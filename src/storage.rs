use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::Rgba;
use rusqlite::{Connection, OptionalExtension, params};

pub const LICHESS_LIGHT_SQUARE_HEX: &str = "#f0d9b5";
pub const LICHESS_DARK_SQUARE_HEX: &str = "#b58863";
pub const CHESS_COM_LIGHT_SQUARE_HEX: &str = "#eeeed2";
pub const CHESS_COM_DARK_SQUARE_HEX: &str = "#769656";
pub const DEFAULT_LIGHT_SQUARE_HEX: &str = LICHESS_LIGHT_SQUARE_HEX;
pub const DEFAULT_DARK_SQUARE_HEX: &str = LICHESS_DARK_SQUARE_HEX;
pub const THEME_PRESET_LICHESS: &str = "lichess";
pub const THEME_PRESET_CHESS_COM: &str = "chess.com";
pub const THEME_PRESET_CUSTOM: &str = "custom";
const THEME_PRESET_DEFAULT_LEGACY: &str = "default";

const DB_REL_PATH: &str = ".kchess/kchess.db";
const SCHEMA_VERSION: i64 = 3;

const KEY_LIGHT_SQUARE_HEX: &str = "board.light_square_hex";
const KEY_DARK_SQUARE_HEX: &str = "board.dark_square_hex";
const KEY_THEME_PRESET: &str = "board.theme_preset";

const INITIAL_GAME_SYNC_PAGE_SIZE: usize = 100;
const INITIAL_GAME_SYNC_MAX_PAGES: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardLooksSettings {
    pub light_square_hex: String,
    pub dark_square_hex: String,
    pub theme_preset: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LichessAccount {
    pub username: String,
    pub added_at: i64,
    pub connected_at: Option<i64>,
    pub auth_kind: String,
    pub last_synced_at: Option<i64>,
    pub latest_game_id: Option<String>,
    pub latest_game_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LichessGame {
    pub game_id: String,
    pub account_username: String,
    pub played_at: i64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub variant: String,
    pub status: String,
    pub winner: Option<String>,
    pub color: String,
    pub opponent_name: String,
    pub opponent_rating: Option<i64>,
    pub player_rating: Option<i64>,
    pub rating_diff: Option<i64>,
    pub opening_name: Option<String>,
    pub moves: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LichessSyncCursor {
    pub latest_game_id: Option<String>,
    pub latest_game_at: Option<i64>,
}

pub const LICHESS_AUTH_KIND_TRACKED: &str = "tracked";
pub const LICHESS_AUTH_KIND_OAUTH: &str = "oauth";

impl LichessSyncCursor {
    pub fn empty() -> Self {
        Self {
            latest_game_id: None,
            latest_game_at: None,
        }
    }
}

impl Default for BoardLooksSettings {
    fn default() -> Self {
        Self {
            light_square_hex: DEFAULT_LIGHT_SQUARE_HEX.to_string(),
            dark_square_hex: DEFAULT_DARK_SQUARE_HEX.to_string(),
            theme_preset: THEME_PRESET_LICHESS.to_string(),
        }
    }
}

impl BoardLooksSettings {
    pub fn new(light_square_hex: String, dark_square_hex: String, theme_preset: String) -> Self {
        Self {
            light_square_hex,
            dark_square_hex,
            theme_preset,
        }
    }
}

#[derive(Debug)]
pub enum StorageError {
    HomeDirectoryUnavailable,
    Io(io::Error),
    Sql(rusqlite::Error),
    InvalidHexColor(String),
    InvalidLichessUsername(String),
    DuplicateLichessAccount(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HomeDirectoryUnavailable => write!(f, "home directory unavailable"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Sql(err) => write!(f, "sqlite error: {err}"),
            Self::InvalidHexColor(value) => write!(f, "invalid strict hex color: {value}"),
            Self::InvalidLichessUsername(value) => write!(f, "invalid lichess username: {value}"),
            Self::DuplicateLichessAccount(value) => {
                write!(f, "lichess account already added: {value}")
            }
        }
    }
}

impl std::error::Error for StorageError {}

impl From<io::Error> for StorageError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
    let (r, g, b, a) = parse_hex_components(hex)?;
    Some(rgba_from_components(r, g, b, a))
}

pub fn format_hex_color(color: Rgba) -> String {
    let (r, g, b, a) = rgba_components(color);
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn is_strict_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    let valid_len = bytes.len() == 7 || bytes.len() == 9;
    valid_len
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
}

pub fn normalize_hex(value: &str) -> Option<String> {
    parse_hex_color(value).map(format_hex_color)
}

pub fn resolve_theme_preset(light_hex: &str, dark_hex: &str) -> String {
    let Some(light_hex) = normalize_hex(light_hex) else {
        return THEME_PRESET_CUSTOM.to_string();
    };
    let Some(dark_hex) = normalize_hex(dark_hex) else {
        return THEME_PRESET_CUSTOM.to_string();
    };

    if light_hex.len() != 7 || dark_hex.len() != 7 {
        return THEME_PRESET_CUSTOM.to_string();
    }

    if light_hex.eq_ignore_ascii_case(LICHESS_LIGHT_SQUARE_HEX)
        && dark_hex.eq_ignore_ascii_case(LICHESS_DARK_SQUARE_HEX)
    {
        return THEME_PRESET_LICHESS.to_string();
    }

    if light_hex.eq_ignore_ascii_case(CHESS_COM_LIGHT_SQUARE_HEX)
        && dark_hex.eq_ignore_ascii_case(CHESS_COM_DARK_SQUARE_HEX)
    {
        return THEME_PRESET_CHESS_COM.to_string();
    }

    THEME_PRESET_CUSTOM.to_string()
}

pub fn preset_colors(preset: &str) -> Option<(&'static str, &'static str)> {
    match preset {
        THEME_PRESET_LICHESS | THEME_PRESET_DEFAULT_LEGACY => {
            Some((LICHESS_LIGHT_SQUARE_HEX, LICHESS_DARK_SQUARE_HEX))
        }
        THEME_PRESET_CHESS_COM => Some((CHESS_COM_LIGHT_SQUARE_HEX, CHESS_COM_DARK_SQUARE_HEX)),
        _ => None,
    }
}

pub fn load_board_looks_settings() -> BoardLooksSettings {
    let default_settings = BoardLooksSettings::default();
    let Some(path) = storage_path() else {
        return default_settings;
    };

    match load_board_looks_settings_with_path(&path) {
        Ok(settings) => settings,
        Err(err) => {
            eprintln!("Failed loading board looks settings, using defaults: {err}");
            default_settings
        }
    }
}

pub fn save_board_looks_settings(settings: &BoardLooksSettings) -> Result<(), StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    save_board_looks_settings_with_path(&path, settings)
}

pub fn load_lichess_accounts() -> Result<Vec<LichessAccount>, StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    load_lichess_accounts_with_path(&path)
}

pub fn load_lichess_games(per_account_limit: usize) -> Result<Vec<LichessGame>, StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    load_lichess_games_with_path(&path, per_account_limit)
}

pub fn add_lichess_account(username: &str) -> Result<LichessAccount, StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    add_lichess_account_with_path(&path, username)
}

pub fn upsert_connected_lichess_account(username: &str) -> Result<LichessAccount, StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    upsert_connected_lichess_account_with_path(&path, username)
}

pub fn remove_lichess_account(username: &str) -> Result<(), StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    remove_lichess_account_with_path(&path, username)
}

pub fn save_lichess_sync(
    username: &str,
    cursor: &LichessSyncCursor,
    games: &[LichessGame],
) -> Result<usize, StorageError> {
    let Some(path) = storage_path() else {
        return Err(StorageError::HomeDirectoryUnavailable);
    };
    save_lichess_sync_with_path(&path, username, cursor, games)
}

pub fn normalize_lichess_username(username: &str) -> Result<String, StorageError> {
    let normalized = username.trim();
    let invalid = normalized.is_empty()
        || normalized.contains(char::is_whitespace)
        || normalized.contains('/')
        || normalized.contains('?')
        || normalized.contains('#');

    if invalid {
        Err(StorageError::InvalidLichessUsername(username.to_string()))
    } else {
        Ok(normalized.to_string())
    }
}

pub fn initial_sync_page_size() -> usize {
    INITIAL_GAME_SYNC_PAGE_SIZE
}

pub fn initial_sync_max_pages() -> usize {
    INITIAL_GAME_SYNC_MAX_PAGES
}

fn storage_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(Path::new(&home).join(DB_REL_PATH))
}

fn load_board_looks_settings_with_path(path: &Path) -> Result<BoardLooksSettings, StorageError> {
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let mut values = HashMap::new();
    let mut statement = connection.prepare("SELECT key, value FROM app_settings")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (key, value) = row?;
        values.insert(key, value);
    }

    let mut settings = BoardLooksSettings::default();

    if let Some(light) = values
        .get(KEY_LIGHT_SQUARE_HEX)
        .and_then(|v| normalize_hex(v))
    {
        settings.light_square_hex = light;
    }

    if let Some(dark) = values
        .get(KEY_DARK_SQUARE_HEX)
        .and_then(|v| normalize_hex(v))
    {
        settings.dark_square_hex = dark;
    }

    settings.theme_preset = values
        .get(KEY_THEME_PRESET)
        .map(String::as_str)
        .map(|preset| {
            normalize_theme_preset(
                Some(preset),
                &settings.light_square_hex,
                &settings.dark_square_hex,
            )
        })
        .unwrap_or_else(|| {
            resolve_theme_preset(&settings.light_square_hex, &settings.dark_square_hex)
        });

    Ok(settings)
}

fn save_board_looks_settings_with_path(
    path: &Path,
    settings: &BoardLooksSettings,
) -> Result<(), StorageError> {
    let Some(light_hex) = normalize_hex(&settings.light_square_hex) else {
        return Err(StorageError::InvalidHexColor(
            settings.light_square_hex.clone(),
        ));
    };

    let Some(dark_hex) = normalize_hex(&settings.dark_square_hex) else {
        return Err(StorageError::InvalidHexColor(
            settings.dark_square_hex.clone(),
        ));
    };

    let preset = normalize_theme_preset(Some(&settings.theme_preset), &light_hex, &dark_hex);

    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let tx = connection.unchecked_transaction()?;
    upsert_setting(&tx, KEY_LIGHT_SQUARE_HEX, &light_hex)?;
    upsert_setting(&tx, KEY_DARK_SQUARE_HEX, &dark_hex)?;
    upsert_setting(&tx, KEY_THEME_PRESET, &preset)?;
    tx.commit()?;

    Ok(())
}

fn load_lichess_accounts_with_path(path: &Path) -> Result<Vec<LichessAccount>, StorageError> {
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let mut statement = connection.prepare(
        "SELECT username, added_at, connected_at, auth_kind, last_synced_at, latest_game_id, latest_game_at
         FROM lichess_accounts
         ORDER BY added_at ASC, username COLLATE NOCASE ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(LichessAccount {
            username: row.get(0)?,
            added_at: row.get(1)?,
            connected_at: row.get(2)?,
            auth_kind: row.get(3)?,
            last_synced_at: row.get(4)?,
            latest_game_id: row.get(5)?,
            latest_game_at: row.get(6)?,
        })
    })?;

    let mut accounts = Vec::new();
    for row in rows {
        accounts.push(row?);
    }
    Ok(accounts)
}

fn load_lichess_games_with_path(
    path: &Path,
    per_account_limit: usize,
) -> Result<Vec<LichessGame>, StorageError> {
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let mut statement = connection.prepare(
        "SELECT game_id, account_username, played_at, rated, speed, perf, variant, status, winner,
                color, opponent_name, opponent_rating, player_rating, rating_diff, opening_name, moves
         FROM (
             SELECT game_id, account_username, played_at, rated, speed, perf, variant, status, winner,
                    color, opponent_name, opponent_rating, player_rating, rating_diff, opening_name, moves,
                    ROW_NUMBER() OVER (
                        PARTITION BY account_username COLLATE NOCASE
                        ORDER BY played_at DESC, game_id DESC
                    ) AS row_num
             FROM lichess_games
         ) ranked
         WHERE row_num <= ?1
         ORDER BY played_at DESC, game_id DESC",
    )?;
    let rows = statement.query_map(params![per_account_limit as i64], |row| {
        Ok(LichessGame {
            game_id: row.get(0)?,
            account_username: row.get(1)?,
            played_at: row.get(2)?,
            rated: row.get::<_, i64>(3)? != 0,
            speed: row.get(4)?,
            perf: row.get(5)?,
            variant: row.get(6)?,
            status: row.get(7)?,
            winner: row.get(8)?,
            color: row.get(9)?,
            opponent_name: row.get(10)?,
            opponent_rating: row.get(11)?,
            player_rating: row.get(12)?,
            rating_diff: row.get(13)?,
            opening_name: row.get(14)?,
            moves: row.get(15)?,
        })
    })?;

    let mut games = Vec::new();
    for row in rows {
        games.push(row?);
    }
    Ok(games)
}

fn add_lichess_account_with_path(
    path: &Path,
    username: &str,
) -> Result<LichessAccount, StorageError> {
    let normalized = normalize_lichess_username(username)?;
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let exists = connection
        .query_row(
            "SELECT 1 FROM lichess_accounts WHERE username = ?1 COLLATE NOCASE",
            params![normalized],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Err(StorageError::DuplicateLichessAccount(normalized));
    }

    let added_at = unix_seconds_now();
    connection.execute(
        "INSERT INTO lichess_accounts (
            username, added_at, connected_at, auth_kind, last_synced_at, latest_game_id, latest_game_at
         ) VALUES (?1, ?2, NULL, ?3, NULL, NULL, NULL)",
        params![normalized, added_at, LICHESS_AUTH_KIND_TRACKED],
    )?;

    Ok(LichessAccount {
        username: normalized,
        added_at,
        connected_at: None,
        auth_kind: LICHESS_AUTH_KIND_TRACKED.to_string(),
        last_synced_at: None,
        latest_game_id: None,
        latest_game_at: None,
    })
}

fn upsert_connected_lichess_account_with_path(
    path: &Path,
    username: &str,
) -> Result<LichessAccount, StorageError> {
    let normalized = normalize_lichess_username(username)?;
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let now = unix_seconds_now();
    connection.execute(
        "INSERT INTO lichess_accounts (
            username, added_at, connected_at, auth_kind, last_synced_at, latest_game_id, latest_game_at
         ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL)
         ON CONFLICT(username) DO UPDATE SET
            connected_at = excluded.connected_at,
            auth_kind = excluded.auth_kind",
        params![normalized, now, now, LICHESS_AUTH_KIND_OAUTH],
    )?;

    load_lichess_accounts_with_path(path)?
        .into_iter()
        .find(|account| account.username.eq_ignore_ascii_case(&normalized))
        .ok_or_else(|| StorageError::InvalidLichessUsername(normalized))
}

fn save_lichess_sync_with_path(
    path: &Path,
    username: &str,
    cursor: &LichessSyncCursor,
    games: &[LichessGame],
) -> Result<usize, StorageError> {
    let normalized = normalize_lichess_username(username)?;
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let tx = connection.unchecked_transaction()?;
    let now = unix_seconds_now();
    let mut inserted = 0;

    tx.execute(
        "INSERT INTO lichess_accounts (
            username, added_at, connected_at, auth_kind, last_synced_at, latest_game_id, latest_game_at
         ) VALUES (?1, ?2, NULL, ?3, NULL, NULL, NULL)
         ON CONFLICT(username) DO NOTHING",
        params![normalized, now, LICHESS_AUTH_KIND_TRACKED],
    )?;

    for game in games {
        inserted += tx.execute(
            "INSERT OR IGNORE INTO lichess_games (
                game_id, account_username, played_at, rated, speed, perf, variant, status, winner,
                color, opponent_name, opponent_rating, player_rating, rating_diff, opening_name, moves, inserted_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
            )",
            params![
                game.game_id,
                normalized,
                game.played_at,
                if game.rated { 1 } else { 0 },
                game.speed,
                game.perf,
                game.variant,
                game.status,
                game.winner,
                game.color,
                game.opponent_name,
                game.opponent_rating,
                game.player_rating,
                game.rating_diff,
                game.opening_name,
                game.moves,
                now,
            ],
        )?;
    }

    match (&cursor.latest_game_id, cursor.latest_game_at) {
        (Some(game_id), Some(game_at)) => {
            tx.execute(
                "UPDATE lichess_accounts
                 SET latest_game_id = ?2, latest_game_at = ?3, last_synced_at = ?4
                 WHERE username = ?1 COLLATE NOCASE",
                params![normalized, game_id, game_at, now],
            )?;
        }
        _ => {
            tx.execute(
                "UPDATE lichess_accounts
                 SET last_synced_at = ?2
                 WHERE username = ?1 COLLATE NOCASE",
                params![normalized, now],
            )?;
        }
    }

    tx.commit()?;
    Ok(inserted)
}

fn remove_lichess_account_with_path(path: &Path, username: &str) -> Result<(), StorageError> {
    let normalized = normalize_lichess_username(username)?;
    let connection = open_connection(path)?;
    run_migrations(&connection)?;

    let tx = connection.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM lichess_games WHERE account_username = ?1 COLLATE NOCASE",
        params![normalized],
    )?;
    tx.execute(
        "DELETE FROM lichess_accounts WHERE username = ?1 COLLATE NOCASE",
        params![normalized],
    )?;
    tx.commit()?;

    Ok(())
}

fn open_connection(path: &Path) -> Result<Connection, StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(Connection::open(path)?)
}

fn run_migrations(connection: &Connection) -> Result<(), StorageError> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            version INTEGER NOT NULL
        )",
        [],
    )?;

    connection.execute(
        "INSERT INTO schema_migrations (id, version) VALUES (1, 0)
         ON CONFLICT(id) DO NOTHING",
        [],
    )?;

    let current_version: i64 = connection
        .query_row(
            "SELECT version FROM schema_migrations WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);

    if current_version < 1 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;

        connection.execute("UPDATE schema_migrations SET version = 1 WHERE id = 1", [])?;
    }

    if current_version < 2 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS lichess_accounts (
                username TEXT PRIMARY KEY COLLATE NOCASE,
                added_at INTEGER NOT NULL,
                connected_at INTEGER,
                auth_kind TEXT NOT NULL DEFAULT 'tracked',
                last_synced_at INTEGER,
                latest_game_id TEXT,
                latest_game_at INTEGER
            )",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS lichess_games (
                game_id TEXT PRIMARY KEY,
                account_username TEXT NOT NULL,
                played_at INTEGER NOT NULL,
                rated INTEGER NOT NULL,
                speed TEXT NOT NULL,
                perf TEXT NOT NULL,
                variant TEXT NOT NULL,
                status TEXT NOT NULL,
                winner TEXT,
                color TEXT NOT NULL,
                opponent_name TEXT NOT NULL,
                opponent_rating INTEGER,
                player_rating INTEGER,
                rating_diff INTEGER,
                opening_name TEXT,
                moves TEXT NOT NULL,
                inserted_at INTEGER NOT NULL,
                FOREIGN KEY(account_username) REFERENCES lichess_accounts(username)
            )",
            [],
        )?;

        connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_lichess_games_account_played_at
             ON lichess_games (account_username COLLATE NOCASE, played_at DESC)",
            [],
        )?;

        connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_lichess_games_played_at
             ON lichess_games (played_at DESC)",
            [],
        )?;

        connection.execute("UPDATE schema_migrations SET version = 2 WHERE id = 1", [])?;
    }

    if current_version < 3 {
        let _ = connection.execute(
            "ALTER TABLE lichess_accounts ADD COLUMN connected_at INTEGER",
            [],
        );
        let _ = connection.execute(
            "ALTER TABLE lichess_accounts ADD COLUMN auth_kind TEXT NOT NULL DEFAULT 'tracked'",
            [],
        );
        connection.execute(
            "UPDATE lichess_accounts
             SET auth_kind = COALESCE(auth_kind, 'tracked')
             WHERE auth_kind IS NULL OR auth_kind = ''",
            [],
        )?;
        connection.execute(
            "UPDATE schema_migrations SET version = ?1 WHERE id = 1",
            params![SCHEMA_VERSION],
        )?;
    }

    Ok(())
}

fn upsert_setting(connection: &Connection, key: &str, value: &str) -> Result<(), StorageError> {
    let now = unix_seconds_now();
    connection.execute(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )?;

    Ok(())
}

fn unix_seconds_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn parse_hex_components(hex: &str) -> Option<(u8, u8, u8, u8)> {
    if !is_strict_hex_color(hex) {
        return None;
    }

    let text = hex.trim_start_matches('#');
    let rgba = match text.len() {
        6 => {
            let rgb = u32::from_str_radix(text, 16).ok()?;
            (
                ((rgb >> 16) & 0xff) as u8,
                ((rgb >> 8) & 0xff) as u8,
                (rgb & 0xff) as u8,
                255,
            )
        }
        8 => {
            let rgba = u32::from_str_radix(text, 16).ok()?;
            (
                ((rgba >> 24) & 0xff) as u8,
                ((rgba >> 16) & 0xff) as u8,
                ((rgba >> 8) & 0xff) as u8,
                (rgba & 0xff) as u8,
            )
        }
        _ => return None,
    };

    Some(rgba)
}

fn rgba_from_components(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    }
}

fn rgba_components(color: Rgba) -> (u8, u8, u8, u8) {
    (
        float_channel_to_u8(color.r),
        float_channel_to_u8(color.g),
        float_channel_to_u8(color.b),
        float_channel_to_u8(color.a),
    )
}

fn float_channel_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn normalize_theme_preset(preset: Option<&str>, light_hex: &str, dark_hex: &str) -> String {
    match preset {
        Some(THEME_PRESET_LICHESS) | Some(THEME_PRESET_DEFAULT_LEGACY)
            if light_hex.len() == 7 && dark_hex.len() == 7 =>
        {
            THEME_PRESET_LICHESS.to_string()
        }
        Some(THEME_PRESET_CHESS_COM) if light_hex.len() == 7 && dark_hex.len() == 7 => {
            THEME_PRESET_CHESS_COM.to_string()
        }
        Some(THEME_PRESET_CUSTOM) => THEME_PRESET_CUSTOM.to_string(),
        _ => resolve_theme_preset(light_hex, dark_hex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_db_path(test_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("kchess-{test_name}-{timestamp}.db"))
    }

    #[test]
    fn migration_bootstrap_is_idempotent() {
        let path = unique_temp_db_path("migration");
        let conn = open_connection(&path).expect("open db");
        run_migrations(&conn).expect("first migration");
        run_migrations(&conn).expect("second migration");

        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("schema version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn save_and_load_round_trip_works() {
        let path = unique_temp_db_path("round-trip");
        let input = BoardLooksSettings::new(
            "#123abc".to_string(),
            "#fedcba".to_string(),
            THEME_PRESET_CUSTOM.to_string(),
        );

        save_board_looks_settings_with_path(&path, &input).expect("save board looks");
        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded, input);
    }

    #[test]
    fn translucent_colors_round_trip_as_eight_digit_hex() {
        let path = unique_temp_db_path("translucent");
        let input = BoardLooksSettings::new(
            "#123abc80".to_string(),
            "#fedcba40".to_string(),
            THEME_PRESET_CUSTOM.to_string(),
        );

        save_board_looks_settings_with_path(&path, &input).expect("save board looks");
        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded, input);
    }

    #[test]
    fn opaque_eight_digit_hex_normalizes_to_six_digit_hex() {
        assert_eq!(normalize_hex("#123abcff").as_deref(), Some("#123abc"));
    }

    #[test]
    fn default_preset_round_trip_works() {
        let path = unique_temp_db_path("default");
        let input = BoardLooksSettings::new(
            DEFAULT_LIGHT_SQUARE_HEX.to_string(),
            DEFAULT_DARK_SQUARE_HEX.to_string(),
            THEME_PRESET_LICHESS.to_string(),
        );

        save_board_looks_settings_with_path(&path, &input).expect("save board looks");
        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded, input);
    }

    #[test]
    fn invalid_stored_values_fall_back_to_defaults() {
        let path = unique_temp_db_path("invalid");
        let conn = open_connection(&path).expect("open db");
        run_migrations(&conn).expect("migrate");
        upsert_setting(&conn, KEY_LIGHT_SQUARE_HEX, "#12zz99").expect("write invalid light");
        upsert_setting(&conn, KEY_DARK_SQUARE_HEX, "not-a-color").expect("write invalid dark");
        upsert_setting(&conn, KEY_THEME_PRESET, "weird").expect("write invalid preset");

        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded.light_square_hex, DEFAULT_LIGHT_SQUARE_HEX);
        assert_eq!(loaded.dark_square_hex, DEFAULT_DARK_SQUARE_HEX);
        assert_eq!(loaded.theme_preset, THEME_PRESET_LICHESS);
    }

    #[test]
    fn legacy_default_preset_value_maps_to_lichess() {
        let path = unique_temp_db_path("legacy-default-preset");
        let conn = open_connection(&path).expect("open db");
        run_migrations(&conn).expect("migrate");
        upsert_setting(&conn, KEY_THEME_PRESET, THEME_PRESET_DEFAULT_LEGACY).expect("legacy");

        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded.theme_preset, THEME_PRESET_LICHESS);
    }

    #[test]
    fn chess_com_colors_map_to_chess_com_preset() {
        let path = unique_temp_db_path("chess-com");
        let input = BoardLooksSettings::new(
            CHESS_COM_LIGHT_SQUARE_HEX.to_string(),
            CHESS_COM_DARK_SQUARE_HEX.to_string(),
            THEME_PRESET_CHESS_COM.to_string(),
        );

        save_board_looks_settings_with_path(&path, &input).expect("save board looks");
        let loaded = load_board_looks_settings_with_path(&path).expect("load board looks");
        assert_eq!(loaded, input);
    }

    #[test]
    fn translucent_preset_colors_resolve_to_custom() {
        assert_eq!(
            resolve_theme_preset("#f0d9b580", LICHESS_DARK_SQUARE_HEX),
            THEME_PRESET_CUSTOM
        );
    }

    #[test]
    fn lichess_account_round_trip_works() {
        let path = unique_temp_db_path("lichess-account");

        let created = add_lichess_account_with_path(&path, "TestUser").expect("create account");
        let loaded = load_lichess_accounts_with_path(&path).expect("load accounts");

        assert_eq!(loaded, vec![created]);
    }

    #[test]
    fn duplicate_lichess_account_is_rejected_case_insensitively() {
        let path = unique_temp_db_path("lichess-duplicate");

        add_lichess_account_with_path(&path, "TestUser").expect("create account");
        let error = add_lichess_account_with_path(&path, "testuser").expect_err("duplicate");

        assert!(matches!(error, StorageError::DuplicateLichessAccount(_)));
    }

    #[test]
    fn connected_lichess_account_upgrades_existing_row() {
        let path = unique_temp_db_path("lichess-connected");
        add_lichess_account_with_path(&path, "TestUser").expect("create account");

        let connected =
            upsert_connected_lichess_account_with_path(&path, "TestUser").expect("connect");

        assert_eq!(connected.auth_kind, LICHESS_AUTH_KIND_OAUTH);
        assert!(connected.connected_at.is_some());
    }

    #[test]
    fn removing_lichess_account_also_removes_games() {
        let path = unique_temp_db_path("lichess-remove");
        add_lichess_account_with_path(&path, "TestUser").expect("create account");
        save_lichess_sync_with_path(
            &path,
            "TestUser",
            &LichessSyncCursor {
                latest_game_id: Some("game-1".to_string()),
                latest_game_at: Some(1_000),
            },
            &[LichessGame {
                game_id: "game-1".to_string(),
                account_username: "TestUser".to_string(),
                played_at: 1_000,
                rated: true,
                speed: "blitz".to_string(),
                perf: "blitz".to_string(),
                variant: "standard".to_string(),
                status: "mate".to_string(),
                winner: Some("white".to_string()),
                color: "white".to_string(),
                opponent_name: "Opponent".to_string(),
                opponent_rating: Some(1800),
                player_rating: Some(1820),
                rating_diff: Some(8),
                opening_name: Some("Italian Game".to_string()),
                moves: "e4 e5".to_string(),
            }],
        )
        .expect("save sync");

        remove_lichess_account_with_path(&path, "TestUser").expect("remove account");

        assert!(
            load_lichess_accounts_with_path(&path)
                .expect("load accounts")
                .is_empty()
        );
        assert!(
            load_lichess_games_with_path(&path, 10)
                .expect("load games")
                .is_empty()
        );
    }

    #[test]
    fn lichess_sync_persists_games_and_cursor() {
        let path = unique_temp_db_path("lichess-sync");
        add_lichess_account_with_path(&path, "TestUser").expect("create account");

        let inserted = save_lichess_sync_with_path(
            &path,
            "TestUser",
            &LichessSyncCursor {
                latest_game_id: Some("game-2".to_string()),
                latest_game_at: Some(2_000),
            },
            &[
                LichessGame {
                    game_id: "game-2".to_string(),
                    account_username: "TestUser".to_string(),
                    played_at: 2_000,
                    rated: true,
                    speed: "blitz".to_string(),
                    perf: "blitz".to_string(),
                    variant: "standard".to_string(),
                    status: "mate".to_string(),
                    winner: Some("white".to_string()),
                    color: "white".to_string(),
                    opponent_name: "Opponent".to_string(),
                    opponent_rating: Some(1800),
                    player_rating: Some(1820),
                    rating_diff: Some(8),
                    opening_name: Some("Sicilian Defense".to_string()),
                    moves: "e4 c5".to_string(),
                },
                LichessGame {
                    game_id: "game-1".to_string(),
                    account_username: "TestUser".to_string(),
                    played_at: 1_000,
                    rated: false,
                    speed: "rapid".to_string(),
                    perf: "rapid".to_string(),
                    variant: "standard".to_string(),
                    status: "resign".to_string(),
                    winner: Some("black".to_string()),
                    color: "white".to_string(),
                    opponent_name: "Other".to_string(),
                    opponent_rating: Some(1750),
                    player_rating: Some(1812),
                    rating_diff: Some(-5),
                    opening_name: None,
                    moves: "d4 Nf6".to_string(),
                },
            ],
        )
        .expect("save sync");

        assert_eq!(inserted, 2);

        let accounts = load_lichess_accounts_with_path(&path).expect("load accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].latest_game_id.as_deref(), Some("game-2"));
        assert_eq!(accounts[0].latest_game_at, Some(2_000));
        assert!(accounts[0].last_synced_at.is_some());

        let games = load_lichess_games_with_path(&path, 10).expect("load games");
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].game_id, "game-2");
        assert_eq!(games[1].game_id, "game-1");
    }

    #[test]
    fn lichess_games_limit_is_applied_per_account() {
        let path = unique_temp_db_path("lichess-per-account-limit");
        add_lichess_account_with_path(&path, "Alpha").expect("create alpha");
        add_lichess_account_with_path(&path, "Beta").expect("create beta");

        let alpha_games = vec![
            LichessGame {
                game_id: "alpha-3".to_string(),
                account_username: "Alpha".to_string(),
                played_at: 3_000,
                rated: true,
                speed: "blitz".to_string(),
                perf: "blitz".to_string(),
                variant: "standard".to_string(),
                status: "mate".to_string(),
                winner: Some("white".to_string()),
                color: "white".to_string(),
                opponent_name: "OppA".to_string(),
                opponent_rating: Some(1700),
                player_rating: Some(1710),
                rating_diff: Some(5),
                opening_name: None,
                moves: "e4 e5".to_string(),
            },
            LichessGame {
                game_id: "alpha-2".to_string(),
                account_username: "Alpha".to_string(),
                played_at: 2_000,
                rated: true,
                speed: "rapid".to_string(),
                perf: "rapid".to_string(),
                variant: "standard".to_string(),
                status: "resign".to_string(),
                winner: Some("black".to_string()),
                color: "white".to_string(),
                opponent_name: "OppB".to_string(),
                opponent_rating: Some(1720),
                player_rating: Some(1704),
                rating_diff: Some(-6),
                opening_name: None,
                moves: "d4 Nf6".to_string(),
            },
            LichessGame {
                game_id: "alpha-1".to_string(),
                account_username: "Alpha".to_string(),
                played_at: 1_000,
                rated: false,
                speed: "blitz".to_string(),
                perf: "blitz".to_string(),
                variant: "standard".to_string(),
                status: "mate".to_string(),
                winner: Some("white".to_string()),
                color: "white".to_string(),
                opponent_name: "OppC".to_string(),
                opponent_rating: Some(1690),
                player_rating: Some(1708),
                rating_diff: Some(4),
                opening_name: None,
                moves: "c4 e5".to_string(),
            },
        ];

        let beta_games = vec![
            LichessGame {
                game_id: "beta-3".to_string(),
                account_username: "Beta".to_string(),
                played_at: 3_100,
                rated: true,
                speed: "bullet".to_string(),
                perf: "bullet".to_string(),
                variant: "standard".to_string(),
                status: "mate".to_string(),
                winner: Some("white".to_string()),
                color: "white".to_string(),
                opponent_name: "OppX".to_string(),
                opponent_rating: Some(1680),
                player_rating: Some(1690),
                rating_diff: Some(7),
                opening_name: None,
                moves: "Nf3 d5".to_string(),
            },
            LichessGame {
                game_id: "beta-2".to_string(),
                account_username: "Beta".to_string(),
                played_at: 2_100,
                rated: true,
                speed: "blitz".to_string(),
                perf: "blitz".to_string(),
                variant: "standard".to_string(),
                status: "resign".to_string(),
                winner: Some("black".to_string()),
                color: "white".to_string(),
                opponent_name: "OppY".to_string(),
                opponent_rating: Some(1675),
                player_rating: Some(1684),
                rating_diff: Some(-6),
                opening_name: None,
                moves: "e4 c5".to_string(),
            },
            LichessGame {
                game_id: "beta-1".to_string(),
                account_username: "Beta".to_string(),
                played_at: 1_100,
                rated: false,
                speed: "rapid".to_string(),
                perf: "rapid".to_string(),
                variant: "standard".to_string(),
                status: "mate".to_string(),
                winner: Some("white".to_string()),
                color: "white".to_string(),
                opponent_name: "OppZ".to_string(),
                opponent_rating: Some(1660),
                player_rating: Some(1694),
                rating_diff: Some(3),
                opening_name: None,
                moves: "d4 d5".to_string(),
            },
        ];

        save_lichess_sync_with_path(
            &path,
            "Alpha",
            &LichessSyncCursor {
                latest_game_id: Some("alpha-3".to_string()),
                latest_game_at: Some(3_000),
            },
            &alpha_games,
        )
        .expect("save alpha");

        save_lichess_sync_with_path(
            &path,
            "Beta",
            &LichessSyncCursor {
                latest_game_id: Some("beta-3".to_string()),
                latest_game_at: Some(3_100),
            },
            &beta_games,
        )
        .expect("save beta");

        let games = load_lichess_games_with_path(&path, 2).expect("load per-account limited games");
        assert_eq!(games.len(), 4);
        assert_eq!(
            games
                .iter()
                .filter(|game| game.account_username.eq_ignore_ascii_case("Alpha"))
                .count(),
            2
        );
        assert_eq!(
            games
                .iter()
                .filter(|game| game.account_username.eq_ignore_ascii_case("Beta"))
                .count(),
            2
        );

        let ids = games
            .iter()
            .map(|game| game.game_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"alpha-3"));
        assert!(ids.contains(&"alpha-2"));
        assert!(ids.contains(&"beta-3"));
        assert!(ids.contains(&"beta-2"));
        assert!(!ids.contains(&"alpha-1"));
        assert!(!ids.contains(&"beta-1"));
    }
}
