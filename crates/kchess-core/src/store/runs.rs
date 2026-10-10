//! Local scores for Storm, Streak, Rush and the practice drills (`core/src/services/runs.ts`).
//! Neither is ever sent anywhere.

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::StoreContext;
use super::library::{db_err, js_json, now_ms, num, object, opt_str, str_arg};
use crate::error::Result;
use crate::host::{Host, Level};

/// The most recent runs a summary lists.
const RECENT: i64 = 20;

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let host = ctx.host;
    let result = match method {
        "store.runs.runSummary" => {
            str_arg(args, 0, "kind").and_then(|kind| run_summary(host, db, kind))
        }
        "store.runs.saveRun" => save_run(host, db, args.first().unwrap_or(&Value::Null)),
        "store.runs.clearRuns" => clear_runs(db, opt_str(args, 0)).map(|()| Value::Null),
        _ => return None,
    };
    Some(result)
}

/// `runSummary(kind)`: the best score per variant, the recent runs and the total.
fn run_summary(host: &dyn Host, db: &Connection, kind: &str) -> Result<Value> {
    let mut best = Map::new();
    let mut statement = db
        .prepare(
            "SELECT variant, MAX(score) AS score,
               (SELECT r2.playedAt FROM runs r2 WHERE r2.kind = r.kind AND r2.variant = r.variant
                 ORDER BY r2.score DESC, r2.playedAt ASC LIMIT 1) AS playedAt
             FROM runs r WHERE kind = ?1 GROUP BY variant",
        )
        .map_err(db_err)?;
    let rows = statement
        .query_map(params![kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(db_err)?;
    for row in rows {
        let (variant, score, played_at) = row.map_err(db_err)?;
        best.insert(
            variant,
            json!({ "score": num(score), "playedAt": played_at }),
        );
    }

    let mut recent = Vec::new();
    let mut statement = db
        .prepare("SELECT id, kind, variant, score, detail, playedAt FROM runs WHERE kind = ?1 ORDER BY playedAt DESC, id DESC LIMIT ?2")
        .map_err(db_err)?;
    let rows = statement
        .query_map(params![kind, RECENT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(db_err)?;
    for row in rows {
        let (id, kind, variant, score, detail, played_at) = row.map_err(db_err)?;
        // A detail that is not JSON reads as an empty object.
        let detail = match serde_json::from_str::<Value>(&detail) {
            Ok(detail) => detail,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "runs",
                    &format!("Run row has invalid detail JSON: {cause}"),
                );
                json!({})
            }
        };
        recent.push(json!({
            "id": id,
            "kind": kind,
            "variant": variant,
            "score": num(score),
            "detail": detail,
            "playedAt": played_at,
        }));
    }

    let total: i64 = db
        .query_row(
            "SELECT COUNT(*) AS n FROM runs WHERE kind = ?1",
            params![kind],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    Ok(json!({
        "kind": kind,
        "best": Value::Object(best),
        "recent": recent,
        "total": total,
    }))
}

/// `saveRun(run)`: record a run; `isBest` when it beats every earlier score of its variant.
fn save_run(host: &dyn Host, db: &Connection, run: &Value) -> Result<Value> {
    let run = object(run, "run")?;
    let kind = run.get("kind").and_then(Value::as_str).unwrap_or_default();
    let variant = run
        .get("variant")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let score = run.get("score").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let detail = js_json(run.get("detail").unwrap_or(&Value::Null));
    let previous: Option<f64> = db
        .query_row(
            "SELECT MAX(score) AS score FROM runs WHERE kind = ?1 AND variant = ?2",
            params![kind, variant],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    db.execute(
        "INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![kind, variant, score, detail, now_ms()? as i64],
    )
    .map_err(db_err)?;
    let is_best = score > 0.0 && previous.is_none_or(|best| score > best);
    Ok(json!({ "summary": run_summary(host, db, kind)?, "isBest": is_best }))
}

/// `clearRuns(kind?)`: forget the local scores of one kind, or of all of them.
fn clear_runs(db: &Connection, kind: Option<&str>) -> Result<()> {
    match kind.filter(|k| !k.is_empty()) {
        Some(kind) => db.execute("DELETE FROM runs WHERE kind = ?1", params![kind]),
        None => db.execute("DELETE FROM runs", []),
    }
    .map(|_| ())
    .map_err(db_err)
}
