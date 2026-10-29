//! Test-only storage methods (`store.debug.*`). Nothing in the application calls them: parity and
//! unit tests read the whole database in one normalised value, and seed rows that an earlier release
//! would have written.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, params_from_iter};
use serde_json::{Map, Value, json};

use crate::error::{CoreError, Result};

/// The debug methods run only when `KCHESS_STORE_DEBUG=1` is set in the process (tests set it);
/// otherwise they are unknown methods, like any other name the core does not have.
///
/// Intentionally runtime-gated, not `cfg(test/debug_assertions)`: tests and end-to-end runs
/// drive a *release build*, where `debug_assertions` is off, yet still need to seed historical
/// rows and read dumps.
/// Packaged builds never set the variable, and `core.rs` refuses the test-only
/// Lichess/voice overrides unless it is set.
pub fn enabled() -> bool {
    std::env::var("KCHESS_STORE_DEBUG").as_deref() == Ok("1")
}

fn storage(cause: rusqlite::Error) -> CoreError {
    CoreError::new(cause.to_string())
}

/// `store.debug.tables([orders])`: every schema object, `user_version`, and every table's rows as
/// objects keyed by column. Rows are ordered by `orders[table]` (SQL, as the caller gives it) or by
/// every column. Shaped as the parity tests' dump of the database.
pub fn tables(db: &Connection, args: &[Value]) -> Result<Value> {
    let orders = args.first().and_then(Value::as_object);
    let schema: Vec<(String, String, String, Option<String>)> = db
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .map_err(storage)?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(storage)?
        .collect::<std::result::Result<_, _>>()
        .map_err(storage)?;
    let user_version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(storage)?;
    let mut tables = Map::new();
    for (kind, name, _, _) in &schema {
        if kind != "table" || name.starts_with("sqlite_") {
            continue;
        }
        let columns: Vec<String> = db
            .prepare(&format!("PRAGMA table_info(\"{name}\")"))
            .map_err(storage)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(storage)?
            .collect::<std::result::Result<_, _>>()
            .map_err(storage)?;
        let order = orders
            .and_then(|map| map.get(name.as_str()))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                columns
                    .iter()
                    .map(|column| format!("\"{column}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        let mut statement = db
            .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY {order}"))
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                let mut object = Map::new();
                for (index, column) in columns.iter().enumerate() {
                    object.insert(column.clone(), cell(row.get_ref(index)?));
                }
                Ok(Value::Object(object))
            })
            .map_err(storage)?
            .collect::<std::result::Result<Vec<Value>, _>>()
            .map_err(storage)?;
        tables.insert(name.clone(), Value::Array(rows));
    }
    let schema: Vec<Value> = schema
        .into_iter()
        .map(|(kind, name, table, sql)| {
            json!({ "type": kind, "name": name, "tbl_name": table, "sql": sql })
        })
        .collect();
    Ok(json!({ "schema": schema, "userVersion": user_version, "tables": tables }))
}

/// `store.debug.query(sql, params)`: the rows of one statement, as objects keyed by column name.
pub fn query(db: &Connection, args: &[Value]) -> Result<Value> {
    let sql = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::new("sql must be a string"))?;
    let params: Vec<SqlValue> = args
        .get(1)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(bind)
        .collect();
    let mut statement = db.prepare(sql).map_err(storage)?;
    let columns: Vec<String> = statement.column_names().map(str::to_owned).collect();
    let mut rows = statement.query(params_from_iter(params)).map_err(storage)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(storage)? {
        let mut object = Map::new();
        for (index, column) in columns.iter().enumerate() {
            object.insert(column.clone(), cell(row.get_ref(index).map_err(storage)?));
        }
        out.push(Value::Object(object));
    }
    Ok(Value::Array(out))
}

/// `store.debug.exec(sql, params)`: one statement with bound parameters, to seed rows that earlier
/// releases wrote. Returns the number of rows changed.
pub fn exec(db: &Connection, args: &[Value]) -> Result<Value> {
    let sql = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::new("sql must be a string"))?;
    let params = args
        .get(1)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(bind)
        .collect::<Vec<_>>();
    if params.is_empty() {
        db.execute_batch(sql).map_err(storage)?;
        return Ok(Value::Null);
    }
    let changed = db.execute(sql, params_from_iter(params)).map_err(storage)?;
    Ok(json!(changed))
}

/// `store.debug.historical(steps, rows)`: a database as the release that applied the first `steps`
/// migrations left it, with rows an earlier release would have written. `rows` is a list of
/// `[table, object]`; a key that the table lacks at that version is left out. Written to `path`
/// before the store opens it, so the next open migrates it to the current version.
pub fn historical(path: &std::path::Path, args: &[Value]) -> Result<Value> {
    let steps = args
        .first()
        .and_then(Value::as_u64)
        .ok_or_else(|| CoreError::new("steps must be a number"))?;
    let db = Connection::open(path).map_err(storage)?;
    db.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(storage)?;
    super::migrations::migrate_to(&db, usize::try_from(steps).unwrap_or(usize::MAX))?;
    for entry in args
        .get(1)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let table = entry
            .get(0)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(row) = entry.get(1).and_then(Value::as_object) else {
            continue;
        };
        let columns: Vec<String> = db
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(storage)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(storage)?
            .collect::<std::result::Result<_, _>>()
            .map_err(storage)?;
        let names: Vec<(&String, &Value)> = row
            .iter()
            .filter(|(name, _)| columns.contains(name))
            .collect();
        if names.is_empty() {
            continue;
        }
        let quoted = names
            .iter()
            .map(|(name, _)| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let marks = vec!["?"; names.len()].join(", ");
        db.execute(
            &format!("INSERT INTO \"{table}\" ({quoted}) VALUES ({marks})"),
            params_from_iter(names.iter().map(|(_, value)| bind(value))),
        )
        .map_err(storage)?;
    }
    Ok(Value::Null)
}

/// A JSON parameter as SQLite binds it.
fn bind(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => SqlValue::Integer(integer),
            None => SqlValue::Real(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(text) => SqlValue::Text(text.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

/// A cell as JSON: blobs as lowercase hex.
fn cell(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => json!(number),
        ValueRef::Real(number) => json!(number),
        ValueRef::Text(text) => json!(String::from_utf8_lossy(text)),
        ValueRef::Blob(bytes) => {
            json!(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, weight REAL, raw BLOB);
             CREATE INDEX idx_notes_body ON notes (body);
             PRAGMA user_version = 7;",
        )
        .unwrap();
        db
    }

    #[test]
    fn exec_binds_every_json_kind_and_query_reads_it_back() {
        let db = database();
        let changed = exec(
            &db,
            &[
                json!("INSERT INTO notes (id, body, weight, raw) VALUES (?, ?, ?, ?)"),
                json!([1, "first", 1.5, null]),
            ],
        )
        .unwrap();
        assert_eq!(changed, json!(1));
        // Booleans bind as integers; arrays and objects as their JSON text.
        exec(
            &db,
            &[
                json!("INSERT INTO notes (id, body, weight) VALUES (?, ?, ?)"),
                json!([2, { "a": [1] }, true]),
            ],
        )
        .unwrap();
        // Without parameters the SQL runs as a batch.
        assert_eq!(
            exec(
                &db,
                &[json!(
                    "INSERT INTO notes (id, raw) VALUES (3, x'00ff'); UPDATE notes SET weight = 0 WHERE id = 3"
                )],
            )
            .unwrap(),
            Value::Null
        );
        let rows = query(
            &db,
            &[
                json!("SELECT * FROM notes WHERE id >= ? ORDER BY id"),
                json!([1]),
            ],
        )
        .unwrap();
        assert_eq!(
            rows,
            json!([
                { "id": 1, "body": "first", "weight": 1.5, "raw": null },
                { "id": 2, "body": "{\"a\":[1]}", "weight": 1.0, "raw": null },
                { "id": 3, "body": null, "weight": 0.0, "raw": "00ff" },
            ])
        );
        assert_eq!(
            query(&db, &[json!("SELECT 1 AS one")]).unwrap(),
            json!([{ "one": 1 }])
        );
    }

    #[test]
    fn statements_need_sql_and_report_sqlite_errors() {
        let db = database();
        assert!(query(&db, &[]).is_err());
        assert!(exec(&db, &[json!(1)]).is_err());
        assert!(query(&db, &[json!("SELECT * FROM missing")]).is_err());
        assert!(exec(&db, &[json!("INSERT INTO missing VALUES (?)"), json!([1])]).is_err());
    }

    #[test]
    fn tables_dumps_schema_version_and_ordered_rows() {
        let db = database();
        exec(
            &db,
            &[json!(
                "INSERT INTO notes (id, body) VALUES (2, 'a'), (1, 'b')"
            )],
        )
        .unwrap();
        let dump = tables(&db, &[]).unwrap();
        assert_eq!(dump["userVersion"], json!(7));
        let names: Vec<&str> = dump["schema"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["idx_notes_body", "notes"]);
        // Every column orders by default; a given order is used as written.
        assert_eq!(dump["tables"]["notes"][0]["id"], json!(1));
        let ordered = tables(&db, &[json!({ "notes": "body" })]).unwrap();
        assert_eq!(ordered["tables"]["notes"][0]["id"], json!(2));
        assert!(tables(&db, &[json!({ "notes": "nonsense(" })]).is_err());
    }

    #[test]
    fn historical_writes_an_old_version_with_the_rows_it_knew() {
        let dir = std::env::temp_dir().join(format!(
            "kchess-debug-historical-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kchess.db");
        let _ = std::fs::remove_file(&path);
        historical(
            &path,
            &[
                json!(1),
                json!([
                    ["accounts", { "username": "Alice", "connected": 1, "notAColumn": 3 }],
                    ["accounts", { "notAColumn": 3 }],
                    ["accounts", "not a row"],
                ]),
            ],
        )
        .unwrap();
        let db = Connection::open(&path).unwrap();
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        assert_eq!(
            query(&db, &[json!("SELECT username, connected FROM accounts")]).unwrap(),
            json!([{ "username": "Alice", "connected": 1 }])
        );
        drop(db);
        assert!(historical(&path, &[json!("one")]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
