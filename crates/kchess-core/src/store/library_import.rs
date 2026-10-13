//! The library snapshot (`library`), its readers, and the one-time legacy import
//! (`importLibrary`) with the studies-table migration.
//! Split from the library store; bodies are unchanged. Named `library_import` because
//! `store::import` already owns the JSON-backup/legacy-database profile migration.

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::documents::{
    DAMAGED_STUDIES, decode_archive_text, decode_archived_game_text, decode_mistakes_text,
    decode_studies_text, parse_text, read_document, read_document_text, stored, stored_studies,
    write_document,
};
use super::json_codec::{js_json, utf16_len};
use super::library::{
    GAMES, IMPORTED, JOINED, LEGACY_TARGETS, MISSES, MISTAKES, SESSION_KINDS, STUDIES, db_err,
    misc, now_ms, transaction, truthy, write_game,
};
use super::studies::id_of;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use kchess_domain::library::MAX_STUDIES;
use kchess_domain::misc::MAX_DOCUMENT;

/// Migrate an earlier release's document atomically, retaining it unchanged on any failure.
pub(crate) fn migrate_studies(db: &Connection) -> Result<()> {
    let Some(body) = read_document_text(db, STUDIES)? else {
        return Ok(());
    };
    // A migrated marker is any document whose `version` is 3; extra keys,
    // whitespace and key order must not un-migrate a healthy library.
    let migrated = parse_text(&body)
        .and_then(|doc| doc.get("version").cloned())
        .is_some_and(|version| match &version {
            Value::Number(n) => {
                n.as_u64() == Some(3) || n.as_i64() == Some(3) || n.as_f64() == Some(3.0)
            }
            _ => false,
        });
    if migrated {
        return Ok(());
    }
    let items = stored_studies(&body)?;
    let work = || {
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM studies", [], |row| row.get(0))
            .map_err(db_err)?;
        if count != 0 {
            return Err(CoreError::new(DAMAGED_STUDIES));
        }
        for (position, study) in items.iter().enumerate() {
            let body = js_json(study);
            let units = utf16_len(&body) as i64;
            db.execute(
                "INSERT INTO studies (id, body, position, textUnits) VALUES (?1, ?2, ?3, ?4)",
                params![id_of(study), body, position as i64, units],
            )
            .map_err(db_err)?;
        }
        write_document(db, STUDIES, &json!({ "version": 3 }), "")?;
        Ok(())
    };
    if db.is_autocommit() {
        transaction(db, work)
    } else {
        work()
    }
}

pub(crate) fn read_studies(db: &Connection) -> Result<Vec<Value>> {
    migrate_studies(db)?;
    let mut statement = db
        .prepare("SELECT id, body FROM studies ORDER BY position")
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    let mut items = Vec::new();
    for row in rows {
        let (id, body) = row.map_err(db_err)?;
        let raw = parse_text(&body).ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
        let study = kchess_domain::library::decode_study(&raw)
            .ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
        if id_of(&study) != Some(id.as_str()) {
            return Err(CoreError::new(DAMAGED_STUDIES));
        }
        items.push(study);
    }
    if items.len() > MAX_STUDIES {
        return Err(CoreError::new(DAMAGED_STUDIES));
    }
    Ok(items)
}

pub(crate) fn read_mistakes(db: &Connection) -> Result<Vec<Value>> {
    Ok(read_document_text(db, MISTAKES)?
        .and_then(|body| decode_mistakes_text(&body))
        .unwrap_or_default())
}

pub(crate) fn read_misses(host: &dyn Host, db: &Connection) -> Result<Value> {
    let raw = read_document(host, db, MISSES)?.unwrap_or(Value::Null);
    misc("decodeRepertoireMisses", vec![raw])
}

pub(crate) fn read_joined(host: &dyn Host, db: &Connection) -> Result<Vec<Value>> {
    let raw = read_document(host, db, JOINED)?.unwrap_or(Value::Null);
    match misc("decodeJoinedTournaments", vec![raw])? {
        Value::Array(entries) => Ok(entries),
        _ => Ok(Vec::new()),
    }
}

/// `readSession(kind)`: the decoded session, or null.
pub(crate) fn read_session(host: &dyn Host, db: &Connection, kind: &str) -> Result<Value> {
    let raw = read_document(host, db, &format!("session:{kind}"))?.unwrap_or(Value::Null);
    misc("decodeStoredSession", vec![json!(kind), raw])
}

/// The played games, newest first; a game that no longer replays is skipped.
pub(crate) fn read_games(host: &dyn Host, db: &Connection) -> Result<Vec<Value>> {
    let mut statement = db
        .prepare("SELECT id, body FROM archived_games ORDER BY seq DESC")
        .map_err(db_err)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    let mut games = Vec::new();
    for row in rows {
        let (id, body) = row.map_err(db_err)?;
        match decode_archived_game_text(&body) {
            Some(game) => games.push(game),
            None => host.log(
                Level::Debug,
                "library",
                &format!("Skipped an archived game that does not replay: id={id}"),
            ),
        }
    }
    Ok(games)
}

/// `library(now)`: the whole snapshot.
pub fn library(host: &dyn Host, db: &Connection, now: f64) -> Result<Value> {
    let mut sessions = Map::new();
    for kind in SESSION_KINDS {
        let value = read_session(host, db, kind)?;
        if truthy(Some(&value)) {
            sessions.insert(kind.to_string(), value);
        }
    }
    let joined: Vec<Value> = read_joined(host, db)?
        .into_iter()
        .filter(|entry| until_after(entry, now))
        .collect();
    let (studies, study_error) = match read_studies(db) {
        Ok(studies) => (studies, None),
        Err(error) if error.message == DAMAGED_STUDIES => {
            host.log(Level::Warn, "library", DAMAGED_STUDIES);
            (Vec::new(), Some(error.message))
        }
        Err(error) => return Err(error),
    };
    let mut snapshot = json!({
        "studies": studies,
        "games": read_games(host, db)?,
        "mistakes": read_mistakes(db)?,
        "sessions": Value::Object(sessions),
        "joinedTournaments": joined,
        "repertoireMisses": read_misses(host, db)?,
        "imported": read_document(host, db, IMPORTED)? == Some(Value::Bool(true)),
    });
    if let Some(error) = study_error {
        snapshot["studyError"] = json!(error);
    }
    Ok(snapshot)
}

/// `entry.until > now`
pub(crate) fn until_after(entry: &Value, now: f64) -> bool {
    entry
        .get("until")
        .and_then(Value::as_f64)
        .is_some_and(|until| until > now)
}

/// `importLibrary(documents)`: take over documents an earlier release kept, once.
pub(crate) fn import_library(
    host: &dyn Host,
    db: &Connection,
    documents: Option<&Value>,
) -> Result<Value> {
    if read_document(host, db, IMPORTED)? == Some(Value::Bool(true)) {
        return library(host, db, now_ms()?);
    }
    let documents = documents.and_then(Value::as_object);
    transaction(db, || {
        for (legacy, key) in LEGACY_TARGETS {
            let Some(text) = documents
                .and_then(|map| map.get(legacy))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if utf16_len(text) > MAX_DOCUMENT || stored(host, db, key)? {
                continue;
            }
            let Some(raw) = parse_text(text) else {
                host.log(
                    Level::Warn,
                    "library",
                    &format!("Skipped an unreadable saved document: key={legacy}"),
                );
                continue;
            };
            let Some(value) = legacy_value(key, &raw, text)? else {
                host.log(
                    Level::Warn,
                    "library",
                    &format!("Skipped an invalid saved document: key={legacy}"),
                );
                continue;
            };
            if key == GAMES {
                // Oldest first, so the most recent game ends up most recent again.
                if let Value::Array(games) = &value {
                    for game in games.iter().rev() {
                        write_game(db, game)?;
                    }
                }
                continue;
            }
            write_document(db, key, &value, "A saved document is too large to import.")?;
        }
        write_document(db, IMPORTED, &Value::Bool(true), "")?;
        Ok(())
    })?;
    library(host, db, now_ms()?)
}

/// The value a legacy document becomes in the library, or None when it is invalid.
fn legacy_value(key: &str, raw: &Value, text: &str) -> Result<Option<Value>> {
    Ok(match key {
        STUDIES => decode_studies_text(text).map(|items| json!({ "version": 2, "items": items })),
        GAMES => decode_archive_text(text).map(Value::Array),
        MISTAKES => decode_mistakes_text(text).map(|items| json!({ "version": 1, "items": items })),
        JOINED => Some(misc("decodeJoinedTournaments", vec![raw.clone()])?),
        MISSES => Some(misc("decodeRepertoireMisses", vec![raw.clone()])?),
        _ => {
            let kind = key.strip_prefix("session:").unwrap_or_default();
            let session = misc("decodeStoredSession", vec![json!(kind), raw.clone()])?;
            if truthy(Some(&session)) {
                Some(misc("encodeSession", vec![json!(kind), session])?)
            } else {
                None
            }
        }
    })
}
