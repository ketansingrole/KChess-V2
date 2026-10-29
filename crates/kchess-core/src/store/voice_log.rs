//! What voice input heard and what came of it, kept to see which
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

/// `new Date(ms).toISOString()`; a time outside the calendar's range is an error.
fn iso_time(ms: f64) -> Result<String> {
    Some(ms)
        .filter(|ms| ms.is_finite())
        .and_then(|ms| kchess_domain::calendar::iso_millis(ms.trunc() as i64))
        .ok_or_else(|| CoreError::new("Invalid time value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_javascript_reads_them() {
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

    struct Logged(std::sync::Mutex<Vec<String>>);
    impl Host for Logged {
        fn log(&self, _: Level, scope: &str, message: &str) {
            self.0
                .lock()
                .expect("log")
                .push(format!("{scope}: {message}"));
        }
        fn emit(&self, _: &str, _: Value) {}
    }

    fn database() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        crate::store::migrations::migrate(&db).expect("migrate");
        db
    }

    fn run(db: &Connection, host: &Logged, method: &str, args: &[Value]) -> Result<Value> {
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = StoreContext {
            db,
            host,
            config: &config,
            cache: &cache,
        };
        call(&ctx, method, args).expect("a voice log method")
    }

    #[test]
    fn attempts_are_saved_corrected_and_listed_newest_first() {
        let db = database();
        let host = Logged(Default::default());
        let first = run(
            &db,
            &host,
            "store.voiceLog.saveVoiceAttempt",
            &[json!({
                "source": "vosk",
                "heard": "night f three",
                "confidence": 0.75,
                "words": [{ "word": "night", "conf": 0.5 }],
                "outcome": "played",
                "parsed": "Nf3",
                "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
                "expected": null,
            })],
        )
        .expect("save");
        let retry = run(
            &db,
            &host,
            "store.voiceLog.saveVoiceAttempt",
            &[json!({ "source": "vosk", "heard": "knight", "confidence": 1, "outcome": "unclear", "retryOf": first })],
        )
        .expect("save");
        assert_eq!(retry.as_i64(), first.as_i64().map(|id| id + 1));
        run(
            &db,
            &host,
            "store.voiceLog.updateVoiceAttempt",
            &[
                first.clone(),
                json!({ "outcome": "wrong", "expected": "Nc3" }),
            ],
        )
        .expect("update");
        // Empty fields leave the entry as it was.
        run(
            &db,
            &host,
            "store.voiceLog.updateVoiceAttempt",
            &[first.clone(), json!({ "outcome": "", "expected": "" })],
        )
        .expect("update");

        let history =
            run(&db, &host, "store.voiceLog.voiceHistory", &[json!(10)]).expect("history");
        let entries = history.as_array().expect("entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["id"], retry);
        assert_eq!(entries[0]["retryOf"], first);
        assert_eq!(entries[0]["words"], json!(null));
        assert!(entries[0].get("parsed").is_none());
        assert_eq!(entries[1]["outcome"], json!("wrong"));
        assert_eq!(entries[1]["expected"], json!("Nc3"));
        assert_eq!(entries[1]["parsed"], json!("Nf3"));
        assert_eq!(entries[1]["confidence"], json!(0.75));
        assert_eq!(
            entries[1]["words"],
            json!([{ "word": "night", "conf": 0.5 }])
        );
        assert!(entries[1].get("retryOf").is_none());

        let limited = run(&db, &host, "store.voiceLog.voiceHistory", &[json!(1)]).expect("history");
        assert_eq!(limited.as_array().map(Vec::len), Some(1));

        run(&db, &host, "store.voiceLog.clearVoiceHistory", &[]).expect("clear");
        assert_eq!(
            run(&db, &host, "store.voiceLog.voiceHistory", &[json!(10)]).expect("history"),
            json!([])
        );
    }

    #[test]
    fn the_log_keeps_only_the_newest_entries() {
        let db = database();
        let host = Logged(Default::default());
        db.execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {MAX_ENTRIES})
             INSERT INTO voice_log (at, source, heard, confidence, words, outcome)
             SELECT i, 'vosk', 'e4', 1, '[]', 'played' FROM n"
        ))
        .expect("seed");
        let id = run(
            &db,
            &host,
            "store.voiceLog.saveVoiceAttempt",
            &[json!({ "heard": "e5", "confidence": 1 })],
        )
        .expect("save");
        let (count, oldest): (i64, i64) = db
            .query_row("SELECT COUNT(*), MIN(id) FROM voice_log", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("count");
        assert_eq!(count, MAX_ENTRIES);
        assert_eq!(oldest, 2);
        assert_eq!(id, json!(MAX_ENTRIES + 1));
    }

    #[test]
    fn the_export_dates_every_entry_and_reads_bad_words_as_none() {
        let db = database();
        let host = Logged(Default::default());
        db.execute(
            "INSERT INTO voice_log (at, source, heard, confidence, words, outcome)
             VALUES (1700000000123, 'vosk', 'e4', 0.5, 'not json', 'played')",
            [],
        )
        .expect("seed");
        let document =
            run(&db, &host, "store.voiceLog.voiceHistoryDocument", &[]).expect("document");
        let text = document.as_str().expect("text");
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"entries\": [\n    {\n"));
        let parsed: Value = serde_json::from_str(text).expect("json");
        assert_eq!(
            parsed["entries"][0]["time"],
            json!("2023-11-14T22:13:20.123Z")
        );
        assert_eq!(parsed["entries"][0]["words"], json!([]));
        assert!(parsed["exportedAt"].as_f64().is_some());
        let logged = host.0.lock().expect("log");
        assert!(logged[0].starts_with("voice: Voice log row has invalid words JSON"));
    }

    #[test]
    fn bad_arguments_are_refused_and_other_methods_are_not_this_modules() {
        let db = database();
        let host = Logged(Default::default());
        assert!(run(&db, &host, "store.voiceLog.saveVoiceAttempt", &[]).is_err());
        assert!(
            run(
                &db,
                &host,
                "store.voiceLog.updateVoiceAttempt",
                &[json!("1")]
            )
            .is_err()
        );
        assert!(run(&db, &host, "store.voiceLog.voiceHistory", &[]).is_err());
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = StoreContext {
            db: &db,
            host: &host,
            config: &config,
            cache: &cache,
        };
        assert!(call(&ctx, "store.runs.runSummary", &[]).is_none());
    }
}
