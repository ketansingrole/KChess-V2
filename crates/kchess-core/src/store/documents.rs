//! One bounded JSON document per key (`readDocument`/`writeDocument` in the TypeScript
//! library): raw text access, parsing, decoding, and the damaged-study guard.
//! Split from the library store; bodies are unchanged.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::json_codec::{js_json, utf16_len};
use super::library::{GAMES, STUDIES, db_err, now_ms};
use super::studies::id_of;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use kchess_domain::misc::MAX_DOCUMENT;

/* ── Documents: one bounded JSON value per key ── */

pub(crate) fn read_document_text(db: &Connection, key: &str) -> Result<Option<String>> {
    db.query_row(
        "SELECT body FROM documents WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(db_err)
}

/// The parsed document; None when it is missing or not JSON (`readDocument` in the TypeScript).
pub fn read_document(host: &dyn Host, db: &Connection, key: &str) -> Result<Option<Value>> {
    let Some(body) = read_document_text(db, key)? else {
        return Ok(None);
    };
    match serde_json::from_str(&body) {
        Ok(value) => Ok(Some(value)),
        Err(cause) => {
            host.log(
                Level::Warn,
                "library",
                &format!("Stored document is not valid JSON: key={key} {cause}"),
            );
            Ok(None)
        }
    }
}

/// `writeDocument`: a document within its size limit, stored with its update time.
pub(crate) fn write_document(
    db: &Connection,
    key: &str,
    value: &Value,
    too_large: &str,
) -> Result<Value> {
    let body = js_json(value);
    if utf16_len(&body) > MAX_DOCUMENT {
        return Err(CoreError::new(too_large));
    }
    let now = now_ms()?;
    db.execute(
        "INSERT INTO documents (key, body, updatedAt) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET body = excluded.body, updatedAt = excluded.updatedAt",
        params![key, body, now as i64],
    )
    .map_err(db_err)?;
    Ok(Value::Null)
}

/// Whether the library already has this document (or, for games, any game).
pub(crate) fn stored(host: &dyn Host, db: &Connection, key: &str) -> Result<bool> {
    if key == GAMES {
        let any = db
            .query_row("SELECT 1 FROM archived_games LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(db_err)?;
        return Ok(any.is_some());
    }
    if key == STUDIES {
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM studies", [], |row| row.get(0))
            .map_err(db_err)?;
        return Ok(count > 0 || read_document_text(db, key)?.is_some());
    }
    Ok(read_document(host, db, key)?.is_some())
}

pub(crate) fn parse_text(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

pub(crate) fn decode_studies_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_studies(&raw))
}

pub(crate) fn decode_mistakes_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_mistakes(&raw))
}

pub(crate) fn decode_archive_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_archive(&raw))
}

pub(crate) fn decode_archived_game_text(text: &str) -> Option<Value> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_archived_game(&raw))
}

pub(crate) const DAMAGED_STUDIES: &str = "The saved study library is damaged. Its data has been preserved; restore a backup before editing studies.";

/// Stored documents must not silently discard invalid or excess studies on the next mutation.
pub(crate) fn stored_studies(text: &str) -> Result<Vec<Value>> {
    if utf16_len(text) > MAX_DOCUMENT {
        return Err(CoreError::new(DAMAGED_STUDIES));
    }
    let raw = parse_text(text).ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
    let decoded = kchess_domain::library::decode_studies(&raw)
        .ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
    let raw_items = raw
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
    let mut ids = std::collections::HashSet::new();
    if decoded.len() != raw_items.len() || decoded.iter().any(|study| !ids.insert(id_of(study))) {
        return Err(CoreError::new(DAMAGED_STUDIES));
    }
    Ok(decoded)
}
