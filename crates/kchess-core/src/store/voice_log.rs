//! What voice input heard and what came of it (`crates/kchess-node/js/voiceLog.ts`), kept to see which
//! words it mishears. Oldest entries beyond `MAX_ENTRIES` are dropped.

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::StoreContext;
use super::library::{db_err, js_json, now_ms, object};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// Oldest entries beyond this are dropped; enough to study, small enough to never matter on disk.
const MAX_ENTRIES: i64 = 5000;

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let host = ctx.host;
    let result = match method {
        "store.voiceLog.saveVoiceAttempt" => {
            save_voice_attempt(db, args.first().unwrap_or(&Value::Null)).map(|id| json!(id))
        }
        "store.voiceLog.updateVoiceAttempt" => {
            let id = args.first().and_then(Value::as_i64);
            match id {
                Some(id) => update_voice_attempt(db, id, args.get(1).unwrap_or(&Value::Null))
                    .map(|()| Value::Null),
                None => Err(CoreError::new("id must be a number")),
            }
        }
        "store.voiceLog.voiceHistory" => match args.first().and_then(Value::as_i64) {
            Some(limit) => voice_history(host, db, limit).map(Value::Array),
            None => Err(CoreError::new("limit must be a number")),
        },
        "store.voiceLog.clearVoiceHistory" => db
            .execute("DELETE FROM voice_log", [])
            .map(|_| Value::Null)
            .map_err(db_err),
        "store.voiceLog.voiceHistoryDocument" => {
            voice_history_document(host, db).map(Value::String)
        }
        _ => return None,
    };
    Some(result)
}

/// `saveVoiceAttempt(attempt)`: the new entry's id.
fn save_voice_attempt(db: &Connection, attempt: &Value) -> Result<i64> {
    let attempt = object(attempt, "voice attempt")?;
    let text = |key: &str| attempt.get(key).cloned().unwrap_or(Value::Null);
    let optional = |key: &str| -> Option<String> {
        attempt
            .get(key)
            .filter(|v| !v.is_null())
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let source = attempt
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let heard = attempt
        .get("heard")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let confidence = attempt
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let outcome = attempt
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default();
    db.execute(
        "INSERT INTO voice_log (at, source, heard, confidence, words, outcome, parsed, expected, fen, retryOf)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            now_ms()? as i64,
            source,
            heard,
            confidence,
            js_json(&text("words")),
            outcome,
            optional("parsed"),
            optional("expected"),
            optional("fen"),
            attempt.get("retryOf").and_then(Value::as_i64),
        ],
    )
    .map_err(db_err)?;
    let id = db.last_insert_rowid();
    db.execute(
        "DELETE FROM voice_log WHERE id NOT IN (SELECT id FROM voice_log ORDER BY id DESC LIMIT ?1)",
        params![MAX_ENTRIES],
    )
    .map_err(db_err)?;
    Ok(id)
}

/// `updateVoiceAttempt(id, update)`: a new outcome and expected move, when given.
fn update_voice_attempt(db: &Connection, id: i64, update: &Value) -> Result<()> {
    if let Some(outcome) = update
        .get("outcome")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        db.execute(
            "UPDATE voice_log SET outcome = ?1 WHERE id = ?2",
            params![outcome, id],
        )
        .map_err(db_err)?;
    }
    if let Some(expected) = update
        .get("expected")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        db.execute(
            "UPDATE voice_log SET expected = ?1 WHERE id = ?2",
            params![expected, id],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

/// `voiceHistory(limit)`: the newest entries first.
fn voice_history(host: &dyn Host, db: &Connection, limit: i64) -> Result<Vec<Value>> {
    let mut statement = db
        .prepare("SELECT * FROM voice_log ORDER BY id DESC LIMIT ?1")
        .map_err(db_err)?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<i64>>(10)?,
            ))
        })
        .map_err(db_err)?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, at, source, heard, confidence, words, outcome, parsed, expected, fen, retry_of) =
            row.map_err(db_err)?;
        // Words that are not JSON read as none.
        let words = match serde_json::from_str::<Value>(&words) {
            Ok(words) => words,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "voice",
                    &format!("Voice log row has invalid words JSON: {cause}"),
                );
                json!([])
            }
        };
        let mut entry = Map::new();
        entry.insert("id".into(), json!(id));
        entry.insert("at".into(), json!(at));
        entry.insert("source".into(), json!(source));
        entry.insert("heard".into(), json!(heard));
        entry.insert("confidence".into(), json!(confidence));
        entry.insert("words".into(), words);
        entry.insert("outcome".into(), json!(outcome));
        for (key, value) in [("parsed", parsed), ("expected", expected), ("fen", fen)] {
            if let Some(value) = value {
                entry.insert(key.into(), json!(value));
            }
        }
        if let Some(retry) = retry_of {
            entry.insert("retryOf".into(), json!(retry));
        }
        entries.push(Value::Object(entry));
    }
    Ok(entries)
}

/// `voiceHistoryDocument()`: the whole log as an indented JSON document for export.
fn voice_history_document(host: &dyn Host, db: &Connection) -> Result<String> {
    let mut entries = Vec::new();
    for mut entry in voice_history(host, db, MAX_ENTRIES)? {
        let at = entry.get("at").and_then(Value::as_f64).unwrap_or(f64::NAN);
        let time = iso_time(at)?;
        if let Value::Object(map) = &mut entry {
            map.insert("time".into(), json!(time));
        }
        entries.push(entry);
    }
    let document = json!({ "exportedAt": now_ms()?, "entries": entries });
    let mut out = String::new();
    write_pretty(&document, 0, &mut out);
    out.push('\n');
    Ok(out)
}

/// `JSON.stringify(value, null, 2)`.
fn write_pretty(value: &Value, depth: usize, out: &mut String) {
    let indent = |out: &mut String, level: usize| out.push_str(&"  ".repeat(level));
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write_pretty(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push_str("{\n");
            for (i, (key, item)) in map.iter().enumerate() {
                indent(out, depth + 1);
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push_str(": ");
                write_pretty(item, depth + 1, out);
                out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push('}');
        }
        other => out.push_str(&js_json(other)),
    }
}

/// `new Date(ms).toISOString()`; a time outside JavaScript's range is an error.
fn iso_time(ms: f64) -> Result<String> {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return Err(CoreError::new("Invalid time value"));
    }
    let ms = ms.trunc() as i64;
    let days = ms.div_euclid(86_400_000);
    let within = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second, millis) = (
        within / 3_600_000,
        within / 60_000 % 60,
        within / 1_000 % 60,
        within % 1_000,
    );
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    Ok(format!(
        "{year}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_javascript_reads_them() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(iso_time(0.0).expect("epoch"), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso_time(1_700_000_000_123.0).expect("date"),
            "2023-11-14T22:13:20.123Z"
        );
        assert_eq!(iso_time(-1.0).expect("date"), "1969-12-31T23:59:59.999Z");
        assert!(iso_time(9.0e15).is_err());
    }

    #[test]
    fn exports_are_indented_as_javascript_indents_them() {
        let value = json!({ "a": [1, {}], "b": [] });
        let mut out = String::new();
        write_pretty(&value, 0, &mut out);
        assert_eq!(out, "{\n  \"a\": [\n    1,\n    {}\n  ],\n  \"b\": []\n}");
    }
}
