//! Local scores for Storm, Streak, Rush and the practice drills.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Config;
    use crate::store::StoreCache;

    struct Logged(std::sync::Mutex<Vec<String>>);
    impl Host for Logged {
        fn log(&self, _: Level, scope: &str, message: &str) {
            self.0.lock().unwrap().push(format!("{scope}: {message}"));
        }
        fn emit(&self, _: &str, _: Value) {}
    }

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        db
    }

    fn run(db: &Connection, host: &Logged, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let config = Config::new(std::path::PathBuf::new());
        let cache = StoreCache::default();
        let ctx = StoreContext {
            db,
            host,
            config: &config,
            cache: &cache,
        };
        call(&ctx, method, args)
    }

    fn save(db: &Connection, host: &Logged, run_value: Value) -> Value {
        run(db, host, "store.runs.saveRun", &[run_value])
            .unwrap()
            .unwrap()
    }

    #[test]
    fn an_empty_kind_summarises_to_nothing() {
        let db = database();
        let host = Logged(Default::default());
        assert_eq!(
            run(&db, &host, "store.runs.runSummary", &[json!("storm")])
                .unwrap()
                .unwrap(),
            json!({ "kind": "storm", "best": {}, "recent": [], "total": 0 })
        );
    }

    #[test]
    fn saving_reports_a_new_best_only_when_it_beats_the_variant() {
        let db = database();
        let host = Logged(Default::default());
        let first = save(
            &db,
            &host,
            json!({ "kind": "storm", "variant": "3m", "score": 20, "detail": { "errors": 2 } }),
        );
        assert_eq!(first["isBest"], json!(true));
        let lower = save(
            &db,
            &host,
            json!({ "kind": "storm", "variant": "3m", "score": 12 }),
        );
        assert_eq!(lower["isBest"], json!(false));
        let tie = save(
            &db,
            &host,
            json!({ "kind": "storm", "variant": "3m", "score": 20 }),
        );
        assert_eq!(tie["isBest"], json!(false));
        // Another variant keeps its own best, and a fractional score keeps its fraction.
        let other = save(
            &db,
            &host,
            json!({ "kind": "storm", "variant": "5m", "score": 7.5 }),
        );
        assert_eq!(other["isBest"], json!(true));
        // A zero score is never a best, even as a variant's first run.
        let zero = save(&db, &host, json!({ "kind": "streak", "score": 0 }));
        assert_eq!(zero["isBest"], json!(false));

        let summary = &other["summary"];
        assert_eq!(summary["total"], json!(4));
        assert_eq!(summary["best"]["3m"]["score"], json!(20));
        assert_eq!(summary["best"]["5m"]["score"], json!(7.5));
        let recent = summary["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 4);
        // Newest first; ties on playedAt fall back to the later id.
        assert_eq!(recent[0]["variant"], json!("5m"));
        assert_eq!(recent[3]["detail"], json!({ "errors": 2 }));
        // The earliest run reaching the best score is the one dated.
        assert_eq!(summary["best"]["3m"]["playedAt"], recent[3]["playedAt"]);
    }

    #[test]
    fn the_summary_lists_only_the_most_recent_runs() {
        let db = database();
        let host = Logged(Default::default());
        for score in 1..=(RECENT + 5) {
            save(
                &db,
                &host,
                json!({ "kind": "rush", "variant": "5m", "score": score }),
            );
        }
        let summary = run(&db, &host, "store.runs.runSummary", &[json!("rush")])
            .unwrap()
            .unwrap();
        assert_eq!(summary["total"], json!(RECENT + 5));
        assert_eq!(summary["recent"].as_array().unwrap().len(), RECENT as usize);
        assert_eq!(summary["recent"][0]["score"], json!(RECENT + 5));
        assert_eq!(summary["best"]["5m"]["score"], json!(RECENT + 5));
    }

    #[test]
    fn a_detail_that_is_not_json_reads_as_empty_and_is_logged() {
        let db = database();
        let host = Logged(Default::default());
        db.execute(
            "INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES ('storm', '3m', 5, 'not json', 1)",
            [],
        )
        .unwrap();
        let summary = run(&db, &host, "store.runs.runSummary", &[json!("storm")])
            .unwrap()
            .unwrap();
        assert_eq!(summary["recent"][0]["detail"], json!({}));
        let logged = host.0.lock().unwrap();
        assert_eq!(logged.len(), 1);
        assert!(logged[0].starts_with("runs: Run row has invalid detail JSON"));
    }

    #[test]
    fn clearing_forgets_one_kind_or_every_kind() {
        let db = database();
        let host = Logged(Default::default());
        for kind in ["storm", "streak", "rush"] {
            save(&db, &host, json!({ "kind": kind, "score": 3 }));
        }
        let total = |kind: &str| {
            run(&db, &host, "store.runs.runSummary", &[json!(kind)])
                .unwrap()
                .unwrap()["total"]
                .clone()
        };
        run(&db, &host, "store.runs.clearRuns", &[json!("storm")])
            .unwrap()
            .unwrap();
        assert_eq!(total("storm"), json!(0));
        assert_eq!(total("streak"), json!(1));
        // An empty kind clears everything, as no kind does.
        run(&db, &host, "store.runs.clearRuns", &[json!("")])
            .unwrap()
            .unwrap();
        assert_eq!(total("streak"), json!(0));
        save(&db, &host, json!({ "kind": "rush", "score": 3 }));
        run(&db, &host, "store.runs.clearRuns", &[])
            .unwrap()
            .unwrap();
        assert_eq!(total("rush"), json!(0));
    }

    #[test]
    fn bad_arguments_are_refused_and_other_methods_are_not_this_modules() {
        let db = database();
        let host = Logged(Default::default());
        assert!(
            run(&db, &host, "store.runs.runSummary", &[json!(3)])
                .unwrap()
                .is_err()
        );
        assert!(run(&db, &host, "store.runs.saveRun", &[]).unwrap().is_err());
        assert!(
            run(&db, &host, "store.runs.saveRun", &[json!("storm")])
                .unwrap()
                .is_err()
        );
        assert!(run(&db, &host, "store.games.loadData", &[]).is_none());
    }
}
