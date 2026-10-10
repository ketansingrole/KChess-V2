//! Test-only storage methods (`store.debug.*`). Nothing in the application calls them: parity and
//! unit tests read the whole database in one normalised value, and seed rows that an earlier release
//! would have written.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, params_from_iter};
use serde_json::{Map, Value, json};

use crate::error::{CoreError, Result};

/// The debug methods run only when `KCHESS_STORE_DEBUG=1` is set in the process (tests set it);
/// otherwise they are unknown methods, like any other name the core does not have.
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
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
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
