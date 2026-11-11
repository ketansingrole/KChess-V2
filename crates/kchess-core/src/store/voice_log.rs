//! What voice input heard and what came of it, kept to see which
//! words it mishears. Oldest entries beyond `MAX_ENTRIES` are dropped.

use kchess_contracts::types::{VoiceAttempt, VoiceAttemptInput, VoiceUpdate, VoiceWord};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::library::js_json;
use super::sql::Sql as _;
use crate::host::{Host, Level};
use crate::time::now_ms;
use kchess_contracts::error::{CoreError, Result};

/// Oldest entries beyond this are dropped; enough to study, small enough to never matter on disk.
const MAX_ENTRIES: i64 = 5000;

/// The new entry's id.
pub fn save_voice_attempt(db: &Connection, attempt: &VoiceAttemptInput) -> Result<i64> {
    let words =
        serde_json::to_value(&attempt.words).map_err(|cause| CoreError::new(cause.to_string()))?;
    db.execute(
        "INSERT INTO voice_log (at, source, heard, confidence, words, outcome, parsed, expected, fen, retryOf)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            now_ms(),
            attempt.source.as_str(),
            attempt.heard,
            attempt.confidence,
            js_json(&words),
            attempt.outcome.as_str(),
            attempt.parsed,
            attempt.expected,
            attempt.fen,
            attempt.retry_of,
        ],
    )
    .sql()?;
    let id = db.last_insert_rowid();
    db.execute(
        "DELETE FROM voice_log WHERE id NOT IN (SELECT id FROM voice_log ORDER BY id DESC LIMIT ?1)",
        params![MAX_ENTRIES],
    )
    .sql()?;
    Ok(id)
}

/// A new outcome and expected move, when given.
pub fn update_voice_attempt(db: &Connection, id: i64, update: &VoiceUpdate) -> Result<()> {
    if let Some(outcome) = update.outcome {
        db.execute(
            "UPDATE voice_log SET outcome = ?1 WHERE id = ?2",
            params![outcome.as_str(), id],
        )
        .sql()?;
    }
    if let Some(expected) = update.expected.as_deref().filter(|s| !s.is_empty()) {
        db.execute(
            "UPDATE voice_log SET expected = ?1 WHERE id = ?2",
            params![expected, id],
        )
        .sql()?;
    }
    Ok(())
}

/// The newest entries first.
pub fn voice_history(host: &dyn Host, db: &Connection, limit: u32) -> Result<Vec<VoiceAttempt>> {
    let mut statement = db
        .prepare("SELECT * FROM voice_log ORDER BY id DESC LIMIT ?1")
        .sql()?;
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
        .sql()?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, at, source, heard, confidence, words, outcome, parsed, expected, fen, retry_of) =
            row.sql()?;
        // Words that are not JSON read as none.
        let words = match serde_json::from_str::<Vec<VoiceWord>>(&words) {
            Ok(words) => words,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "voice",
                    &format!("Voice log row has invalid words JSON: {cause}"),
                );
                Vec::new()
            }
        };
        // A source or outcome no build of KChess writes leaves the row out.
        let named = serde_json::from_value(json!(source))
            .and_then(|source| Ok((source, serde_json::from_value(json!(outcome))?)));
        let (source, outcome) = match named {
            Ok(named) => named,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "voice",
                    &format!("Voice log row {id} is not readable: {cause}"),
                );
                continue;
            }
        };
        entries.push(VoiceAttempt {
            id,
            at,
            input: VoiceAttemptInput {
                source,
                heard,
                confidence,
                words,
                outcome,
                parsed,
                expected,
                fen,
                retry_of,
            },
        });
    }
    Ok(entries)
}

/// The whole log as an indented JSON document for export.
pub fn clear_voice_history(db: &Connection) -> Result<()> {
    db.execute("DELETE FROM voice_log", []).sql()?;
    Ok(())
}

pub fn voice_history_document(host: &dyn Host, db: &Connection) -> Result<String> {
    let mut entries = Vec::new();
    for attempt in voice_history(host, db, MAX_ENTRIES as u32)? {
        let time = iso_time(attempt.at as f64)?;
        let mut entry =
            serde_json::to_value(attempt).map_err(|cause| CoreError::new(cause.to_string()))?;
        if let Value::Object(map) = &mut entry {
            map.insert("time".into(), json!(time));
        }
        entries.push(entry);
    }
    let document = json!({ "exportedAt": now_ms(), "entries": entries });
    let mut out = String::new();
    write_pretty(&document, 0, &mut out);
    out.push('\n');
    Ok(out)
}

/// JSON indented by two spaces.
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

    use crate::test_host::TestHost;

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

    fn database() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        crate::store::migrations::migrate(&db).expect("migrate");
        db
    }

    fn save(db: &Connection, sent: Value) -> i64 {
        let attempt: VoiceAttemptInput = serde_json::from_value(sent).expect("attempt");
        save_voice_attempt(db, &attempt).expect("save")
    }

    fn update(db: &Connection, id: i64, sent: Value) {
        let update: VoiceUpdate = serde_json::from_value(sent).expect("update");
        update_voice_attempt(db, id, &update).expect("update");
    }

    fn history(host: &TestHost, db: &Connection, limit: u32) -> Value {
        json!(voice_history(host, db, limit).expect("history"))
    }

    #[test]
    fn attempts_are_saved_corrected_and_listed_newest_first() {
        let db = database();
        let host = TestHost::default();
        let first = save(
            &db,
            json!({
                "source": "computer",
                "heard": "night f three",
                "confidence": 0.75,
                "words": [{ "word": "night", "conf": 0.5 }],
                "outcome": "played",
                "parsed": "Nf3",
                "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
                "expected": null,
            }),
        );
        let retry = save(
            &db,
            json!({ "source": "computer", "heard": "knight", "confidence": 1, "words": [], "outcome": "unclear", "retryOf": first }),
        );
        assert_eq!(retry, first + 1);
        update(&db, first, json!({ "outcome": "wrong", "expected": "Nc3" }));
        // An empty expected move, or no outcome, leaves the entry as it was.
        update(&db, first, json!({ "expected": "" }));

        let listed = history(&host, &db, 10);
        let entries = listed.as_array().expect("entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["id"], retry);
        assert_eq!(entries[0]["retryOf"], first);
        assert_eq!(entries[0]["confidence"], json!(1.0));
        assert_eq!(entries[0]["words"], json!([]));
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

        let limited = voice_history(&host, &db, 1).expect("history");
        assert_eq!(limited.len(), 1);

        clear_voice_history(&db).expect("clear");
        assert_eq!(history(&host, &db, 10), json!([]));
    }

    #[test]
    fn the_log_keeps_only_the_newest_entries() {
        let db = database();
        db.execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {MAX_ENTRIES})
             INSERT INTO voice_log (at, source, heard, confidence, words, outcome)
             SELECT i, 'vosk', 'e4', 1, '[]', 'played' FROM n"
        ))
        .expect("seed");
        let id = save(
            &db,
            json!({ "source": "local", "heard": "e5", "confidence": 1, "words": [], "outcome": "played" }),
        );
        let (count, oldest): (i64, i64) = db
            .query_row("SELECT COUNT(*), MIN(id) FROM voice_log", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("count");
        assert_eq!(count, MAX_ENTRIES);
        assert_eq!(oldest, 2);
        assert_eq!(id, MAX_ENTRIES + 1);
    }

    #[test]
    fn the_export_dates_every_entry_and_reads_bad_words_as_none() {
        let db = database();
        let host = TestHost::default();
        db.execute(
            "INSERT INTO voice_log (at, source, heard, confidence, words, outcome)
             VALUES (1700000000123, 'local', 'e4', 0.5, 'not json', 'played'),
                    (1700000000124, 'vosk', 'e5', 0.5, '[]', 'played')",
            [],
        )
        .expect("seed");
        let document = voice_history_document(&host, &db).expect("document");
        let text = document.as_str();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"entries\": [\n    {\n"));
        let parsed: Value = serde_json::from_str(text).expect("json");
        assert_eq!(
            parsed["entries"][0]["time"],
            json!("2023-11-14T22:13:20.123Z")
        );
        assert_eq!(parsed["entries"][0]["words"], json!([]));
        assert!(parsed["exportedAt"].as_f64().is_some());
        assert_eq!(parsed["entries"].as_array().map(Vec::len), Some(1));
        let logged = host.logs();
        assert!(logged[0].contains("[voice] Voice log row 2 is not readable"));
        assert!(logged[1].contains("[voice] Voice log row has invalid words JSON"));
    }

    /// The row a saved phrase leaves, as earlier builds wrote it: whole confidences without a
    /// fraction in the words, absent fields as NULL.
    #[test]
    fn a_saved_phrase_is_stored_as_before() {
        let db = database();
        save(
            &db,
            json!({ "source": "computer", "heard": "e four", "confidence": 1, "outcome": "played",
                    "words": [{ "word": "e", "conf": 1 }, { "word": "four", "conf": 0.5 }] }),
        );
        let row: (String, f64, String, Option<String>, Option<i64>) = db
            .query_row(
                "SELECT source, confidence, words, parsed, retryOf FROM voice_log",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("row");
        assert_eq!(
            row,
            (
                "computer".into(),
                1.0,
                r#"[{"word":"e","conf":1},{"word":"four","conf":0.5}]"#.into(),
                None,
                None
            )
        );
    }
}
