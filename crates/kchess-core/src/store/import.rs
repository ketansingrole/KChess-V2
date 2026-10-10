//! The one-time imports an empty profile takes from earlier releases (`ensureMigrated`,
//! `runMigration`, `migrateFromJson` and `migrateLegacy` in `crates/kchess-node/js/store.ts`): the JSON
//! backup `kchess-data.json` with its `lichess-tokens.json`, or else the earlier main database.
//!
//! Tokens: the JSON backup already holds encrypted tokens, which are stored as they are. The earlier
//! database holds plaintext tokens; they are returned to the caller, which encrypts them with the
//! OS credential store (Rust never sees it) and stores them with `store.games.importTokenCiphertext`.

use std::fs;
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params_from_iter};
use serde_json::{Map, Value, json};

use super::StoreContext;
use super::games::{
    GAME_KEYS, MAX_GAMES, default_settings, in_progress, to_sql, write_settings_json,
};
use crate::error::{CoreError, Result};
use crate::host::Level;
use kchess_domain::{api, js};

const PIECE_ANIMATIONS: [&str; 4] = ["none", "fast", "normal", "slow"];
const REVIEW_AUTO: [&str; 3] = ["off", "recent", "all"];

/// A plaintext token from an earlier release, for the caller to encrypt and store.
pub struct PlainToken {
    /// NULL stays NULL, as the earlier database stored it.
    pub account: Option<String>,
    pub token: String,
}

fn storage(cause: rusqlite::Error) -> CoreError {
    CoreError::new(cause.to_string())
}

fn log(ctx: &StoreContext, level: Level, message: &str) {
    ctx.host.log(level, "store", message);
}

/// Import from an earlier release if the profile is empty; a profile with anything in it is left
/// alone. Returns the plaintext tokens of an earlier database that the caller must store.
pub fn migrate(ctx: &StoreContext, encryption_available: bool) -> Result<Vec<PlainToken>> {
    let db = ctx.db;
    let count =
        |sql: &str| -> Result<i64> { db.query_row(sql, [], |row| row.get(0)).map_err(storage) };
    let accounts = count("SELECT COUNT(*) AS n FROM accounts")?;
    let games = count("SELECT COUNT(*) AS n FROM games")?;
    let tokens = count("SELECT COUNT(*) AS n FROM tokens")?;
    let settings = db
        .query_row("SELECT id FROM settings WHERE id = 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map_err(storage)?;
    if accounts > 0 || games > 0 || tokens > 0 || settings.is_some() {
        return Ok(Vec::new());
    }
    if from_json(ctx)? {
        return Ok(Vec::new());
    }
    Ok(from_legacy(ctx, encryption_available))
}

/// `migrateFromJson`: true when the JSON backup was imported (and archived).
fn from_json(ctx: &StoreContext) -> Result<bool> {
    let data_path = ctx.config.data_dir.join("kchess-data.json");
    let token_path = ctx.config.data_dir.join("lichess-tokens.json");
    let raw: Value = match fs::read_to_string(&data_path)
        .map_err(|cause| cause.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|cause| cause.to_string()))
    {
        Ok(raw) => raw,
        Err(cause) => {
            log(
                ctx,
                Level::Debug,
                &format!("No JSON backup to migrate: {cause}"),
            );
            return Ok(false);
        }
    };
    if js::falsy(Some(&raw)) {
        return Ok(false);
    }
    let field = |name: &str| raw.get(name).filter(|value| !value.is_null()).cloned();
    let stored_settings: Map<String, Value> = match field("settings") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let settings = normalize_settings(&stored_settings)?;
    let accounts = field("accounts").unwrap_or_else(|| json!([]));
    // `[...(raw.games ?? [])]` throws for a non-iterable, outside the import's own error handling.
    let mut games: Vec<Value> = match field("games") {
        None => Vec::new(),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(CoreError::new("games is not iterable")),
    };
    // `sort((a, b) => b.createdAt - a.createdAt)`: a NaN difference counts as equal.
    games.sort_by(|a, b| {
        let created = |game: &Value| js::to_number(game.get("createdAt"));
        let difference = created(b) - created(a);
        if difference.is_nan() {
            std::cmp::Ordering::Equal
        } else {
            difference
                .partial_cmp(&0.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    games.truncate(MAX_GAMES as usize);

    let tokens: Option<Value> = match fs::read_to_string(&token_path)
        .map_err(|cause| cause.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|cause| cause.to_string()))
    {
        Ok(value) => Some(value),
        Err(cause) => {
            log(
                ctx,
                Level::Debug,
                &format!("No token backup to migrate: {cause}"),
            );
            None
        }
    };

    let imported = (|| -> Result<()> {
        begin(ctx.db)?;
        let outcome = (|| -> Result<()> {
            write_settings_json(ctx.db, &Value::Object(settings.clone()))?;
            let Some(accounts) = accounts.as_array() else {
                return Err(CoreError::new("accounts is not iterable"));
            };
            for account in accounts {
                ctx.db
                    .execute(
                        "INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, ?, ?)",
                        params_from_iter([
                            to_sql(account.get("username"))?,
                            SqlValue::Integer(i64::from(!js::falsy(account.get("connected")))),
                            SqlValue::Null,
                        ]),
                    )
                    .map_err(storage)?;
            }
            for game in &games {
                insert_game(ctx.db, game)?;
            }
            if let Some(tokens) = &tokens {
                let Some(entries) = tokens.as_object() else {
                    return Err(CoreError::new("tokens is not an object"));
                };
                for (username, encrypted) in entries {
                    ctx.db
                        .execute(
                            "INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)",
                            params_from_iter([
                                SqlValue::Text(username.clone()),
                                to_sql(Some(encrypted))?,
                            ]),
                        )
                        .map_err(storage)?;
                }
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => commit(ctx.db),
            Err(cause) => {
                rollback(ctx, &cause);
                Err(cause)
            }
        }
    })();
    if let Err(cause) = imported {
        log(
            ctx,
            Level::Debug,
            &format!("JSON migration failed: {cause}"),
        );
        return Ok(false);
    }

    if let Err(cause) = fs::rename(&data_path, with_bak(&data_path)) {
        log(
            ctx,
            Level::Debug,
            &format!("Could not archive migrated data file: {cause}"),
        );
    }
    if let Err(cause) = fs::rename(&token_path, with_bak(&token_path)) {
        log(
            ctx,
            Level::Debug,
            &format!("Could not archive migrated token file: {cause}"),
        );
    }
    Ok(true)
}

fn with_bak(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    name.into()
}

/// `migrateLegacy`: the earlier main database, read only; returns the tokens to store.
fn from_legacy(ctx: &StoreContext, encryption_available: bool) -> Vec<PlainToken> {
    let Some(legacy_path) = ctx.config.legacy_database_path.as_deref() else {
        return Vec::new();
    };
    let legacy = match Connection::open_with_flags(
        legacy_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(legacy) => legacy,
        Err(cause) => {
            log(
                ctx,
                Level::Debug,
                &format!(
                    "No legacy database to migrate: {} {cause}",
                    legacy_path.display()
                ),
            );
            return Vec::new();
        }
    };
    let rows = |sql: &str, name: &str, width: usize| -> Vec<Vec<SqlValue>> {
        let read = || -> rusqlite::Result<Vec<Vec<SqlValue>>> {
            let mut statement = legacy.prepare(sql)?;
            let mut out = Vec::new();
            let mut cursor = statement.query([])?;
            while let Some(row) = cursor.next()? {
                out.push(
                    (0..width)
                        .map(|index| row.get::<_, SqlValue>(index))
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                );
            }
            Ok(out)
        };
        match read() {
            Ok(found) => found,
            Err(cause) => {
                log(
                    ctx,
                    Level::Debug,
                    &format!("Legacy {name} are unavailable: {cause}"),
                );
                Vec::new()
            }
        }
    };
    let setting_rows = rows("SELECT key, value FROM app_settings", "settings", 2);
    let account_rows = rows(
        "SELECT username, auth_kind, last_synced_at FROM lichess_accounts",
        "accounts",
        3,
    );
    let game_rows = rows(
        "SELECT game_id, account_username, played_at, rated, speed, perf, status, winner, color, opponent_name, opponent_rating, player_rating, rating_diff, opening_name, moves FROM lichess_games ORDER BY played_at DESC LIMIT 5000",
        "games",
        15,
    );
    let token_rows = rows("SELECT username, token FROM lichess_tokens", "tokens", 2);

    let get = |key: &str| -> SqlValue {
        setting_rows
            .iter()
            .find(|row| matches!(&row[0], SqlValue::Text(name) if name == key))
            .map(|row| row[1].clone())
            .unwrap_or(SqlValue::Null)
    };
    // `get(key) ?? fallback`: a missing or NULL value takes the fallback.
    let or_default = |value: SqlValue, fallback: Value| match value {
        SqlValue::Null => fallback,
        other => sql_json(other),
    };
    // A missing setting is `undefined` in the TypeScript: it never equals a string and is falsy.
    let text = |value: SqlValue| match value {
        SqlValue::Null => String::new(),
        other => js::to_string(Some(&sql_json(other))),
    };
    let appearance = text(get("app.appearance_mode"));
    let theme_preset = text(get("board.theme_preset"));
    let sound_enabled = text(get("sound.enabled")) != "false";
    let volume = js::to_number(Some(&or_default(get("sound.volume"), json!(70))));
    let custom = text(get("engine.custom_path"));
    let managed = text(get("engine.managed_path"));
    let engine_path = if !custom.is_empty() {
        custom
    } else if !managed.is_empty() {
        managed
    } else {
        String::new()
    };
    let mut stored = Map::new();
    stored.insert(
        "appearance".into(),
        json!(if appearance == "light" || appearance == "dark" {
            appearance
        } else {
            "system".into()
        }),
    );
    stored.insert(
        "boardTheme".into(),
        json!(if theme_preset == "chess.com" {
            "green"
        } else {
            "brown"
        }),
    );
    stored.insert("soundEnabled".into(), json!(sound_enabled));
    // `Math.min(1, x)`: NaN stays NaN (Rust's `min` would drop it).
    let volume = volume / 100.0;
    stored.insert(
        "soundVolume".into(),
        number_json(if volume.is_nan() {
            volume
        } else {
            volume.min(1.0)
        }),
    );
    stored.insert("enginePath".into(), json!(engine_path));
    let settings = match normalize_settings(&stored) {
        Ok(settings) => settings,
        Err(cause) => {
            log(
                ctx,
                Level::Debug,
                &format!("Legacy migration failed: {cause}"),
            );
            return Vec::new();
        }
    };

    let games: Vec<Value> = game_rows.iter().map(|row| legacy_game(row)).collect();
    // Stored tokens are encrypted by the caller, which must read every row: a NULL token makes
    // the encryption throw, and that rolls the whole import back (only when encryption is there).
    let tokens: Vec<PlainToken> = token_rows
        .iter()
        .filter_map(|row| match &row[1] {
            SqlValue::Null => None,
            other => Some(PlainToken {
                account: match &row[0] {
                    SqlValue::Null => None,
                    name => Some(js::to_string(Some(&sql_json(name.clone())))),
                },
                token: js::to_string(Some(&sql_json(other.clone()))),
            }),
        })
        .collect();
    let null_token = token_rows
        .iter()
        .any(|row| matches!(row[1], SqlValue::Null));

    let imported = (|| -> Result<()> {
        begin(ctx.db)?;
        let outcome = (|| -> Result<()> {
            write_settings_json(ctx.db, &Value::Object(settings.clone()))?;
            for row in &account_rows {
                let auth_oauth = matches!(&row[1], SqlValue::Text(kind) if kind == "oauth");
                ctx.db
                    .execute(
                        "INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, ?, ?)",
                        params_from_iter([
                            row[0].clone(),
                            SqlValue::Integer(i64::from(auth_oauth)),
                            SqlValue::Null,
                        ]),
                    )
                    .map_err(storage)?;
            }
            for game in &games {
                insert_game(ctx.db, game)?;
            }
            if encryption_available && null_token {
                return Err(CoreError::new("Cannot encrypt a NULL token."));
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => commit(ctx.db),
            Err(cause) => {
                rollback(ctx, &cause);
                Err(cause)
            }
        }
    })();
    match imported {
        Ok(()) if encryption_available => tokens,
        Ok(()) => Vec::new(),
        Err(cause) => {
            log(
                ctx,
                Level::Debug,
                &format!("Legacy migration failed: {cause}"),
            );
            Vec::new()
        }
    }
}

/// One legacy game row as the TypeScript mapping built it (`String`, `Number`, `Boolean` coercions).
fn legacy_game(row: &[SqlValue]) -> Value {
    let value = |index: usize| sql_json(row[index].clone());
    let optional_number = |index: usize| match &row[index] {
        SqlValue::Null => Value::Null,
        other => number_json(js::to_number(Some(&sql_json(other.clone())))),
    };
    let optional_text = |index: usize| match &row[index] {
        SqlValue::Null => Value::Null,
        other => json!(js::to_string(Some(&sql_json(other.clone())))),
    };
    let text = |index: usize| json!(js::to_string(Some(&value(index))));
    let raw = |index: usize| match &row[index] {
        SqlValue::Null => Value::Null,
        other => sql_json(other.clone()),
    };
    json!({
        "id": text(0),
        "account": text(1),
        "createdAt": number_json(js::to_number(Some(&value(2)))),
        "lastMoveAt": number_json(js::to_number(Some(&value(2)))),
        "rated": json!(!js::falsy(Some(&value(3)))),
        "speed": text(4),
        "perf": text(5),
        "status": text(6),
        "winner": raw(7),
        "color": raw(8),
        "opponent": text(9),
        "opponentRating": optional_number(10),
        "playerRating": optional_number(11),
        "ratingDiff": optional_number(12),
        "opening": optional_text(13),
        "moves": text(14),
    })
}

/// `runInsertGame`: a live game is kept as pending (only for a tracked account), any other replaces
/// the stored game.
fn insert_game(db: &Connection, game: &Value) -> Result<()> {
    if in_progress(game)? {
        let account = to_sql(game.get("account"))?;
        db.execute(
            "INSERT OR IGNORE INTO pending_game_sync (account, id)
             SELECT ?, ? WHERE EXISTS (SELECT 1 FROM accounts WHERE username = ? COLLATE NOCASE)",
            params_from_iter([account.clone(), to_sql(game.get("id"))?, account]),
        )
        .map_err(storage)?;
        return Ok(());
    }
    let columns = GAME_KEYS.join(", ");
    let placeholders = vec!["?"; GAME_KEYS.len()].join(", ");
    let params = GAME_KEYS
        .iter()
        .map(|key| to_sql(game.get(*key)))
        .collect::<Result<Vec<_>>>()?;
    db.execute(
        &format!("INSERT OR REPLACE INTO games ({columns}) VALUES ({placeholders})"),
        params_from_iter(params),
    )
    .map_err(storage)?;
    Ok(())
}

/// `normalizeSettings`: the stored settings over the defaults, with the older forms converted.
pub fn normalize_settings(stored: &Map<String, Value>) -> Result<Map<String, Value>> {
    let defaults = match serde_json::to_value(default_settings()) {
        Ok(Value::Object(map)) => map,
        _ => return Err(CoreError::new("The default settings are not an object.")),
    };
    let mut settings = defaults.clone();
    for (key, value) in stored {
        if key != "boardPreset" {
            settings.insert(key.clone(), value.clone());
        }
    }
    let present = |key: &str| stored.get(key).filter(|value| !value.is_null());
    let default = |key: &str| defaults.get(key).cloned().unwrap_or(Value::Null);
    // Older builds stored a preset plus two square colors instead of a board theme.
    let board_theme = match present("boardTheme") {
        Some(theme) => theme.clone(),
        None if stored.get("boardPreset").and_then(Value::as_str) == Some("chess.com") => {
            json!("green")
        }
        None => default("boardTheme"),
    };
    settings.insert("boardTheme".into(), board_theme);
    let coordinates = stored.get("coordinates").cloned().unwrap_or(Value::Null);
    settings.insert(
        "coordinates".into(),
        if matches!(coordinates.as_str(), Some("none" | "outside")) {
            coordinates
        } else {
            json!("inside")
        },
    );
    let promotion = stored.get("promotion").cloned().unwrap_or(Value::Null);
    settings.insert(
        "promotion".into(),
        if matches!(promotion.as_str(), Some("queen" | "premove")) {
            promotion
        } else {
            json!("ask")
        },
    );
    let animation = stored.get("pieceAnimation").cloned().unwrap_or(Value::Null);
    settings.insert(
        "pieceAnimation".into(),
        match animation.as_str() {
            Some(name) if PIECE_ANIMATIONS.contains(&name) => animation,
            _ => default("pieceAnimation"),
        },
    );
    let levels = present("engineLevels")
        .cloned()
        .unwrap_or_else(|| default("engineLevels"));
    settings.insert(
        "engineLevels".into(),
        domain("normalizeEngineLevels", vec![levels])?,
    );
    let review = stored.get("reviewAuto").cloned().unwrap_or(Value::Null);
    settings.insert(
        "reviewAuto".into(),
        match review.as_str() {
            Some(name) if REVIEW_AUTO.contains(&name) => review,
            _ => default("reviewAuto"),
        },
    );
    if let Some(volume) = settings.get("soundVolume").and_then(Value::as_f64)
        && volume > 1.0
    {
        settings.insert("soundVolume".into(), number_json((volume / 100.0).min(1.0)));
    }
    Ok(settings)
}

fn domain(method: &str, args: Vec<Value>) -> Result<Value> {
    let text = serde_json::to_string(&args).map_err(|cause| CoreError::new(cause.to_string()))?;
    let output = api::call(method, &text).map_err(CoreError::new)?;
    serde_json::from_str(&output).map_err(|cause| CoreError::new(cause.to_string()))
}

fn begin(db: &Connection) -> Result<()> {
    db.execute_batch("BEGIN IMMEDIATE").map_err(storage)
}

fn commit(db: &Connection) -> Result<()> {
    db.execute_batch("COMMIT").map_err(storage)
}

/// `rollback`: a failed rollback is logged, and the original error is the one reported.
fn rollback(ctx: &StoreContext, _cause: &CoreError) {
    if let Err(cause) = ctx.db.execute_batch("ROLLBACK") {
        log(ctx, Level::Debug, &format!("Rollback failed: {cause}"));
    }
}

/// A SQLite value as the JSON the TypeScript mapping saw.
fn sql_json(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(number) => json!(number),
        SqlValue::Real(number) => number_json(number),
        SqlValue::Text(text) => json!(text),
        SqlValue::Blob(_) => Value::Null,
    }
}

/// A number as JSON; NaN and infinities become null, as `JSON.stringify` writes them.
fn number_json(number: f64) -> Value {
    serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number)
}
