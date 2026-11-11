//! Local scores for Storm, Streak, Rush and the practice drills.
//! Neither is ever sent anywhere.

use kchess_contracts::types::{Keyed, RunBest, RunInput, RunKind, RunRecord, RunSaved, RunSummary};
use rusqlite::{Connection, params};
use serde_json::{Map, Value};

use super::library::js_json;
use super::sql::Sql as _;
use crate::error::Result;
use crate::host::{Host, Level};
use crate::time::now_ms;

/// The most recent runs a summary lists.
const RECENT: i64 = 20;

/// The best score per variant, the recent runs and the total.
pub fn run_summary(host: &dyn Host, db: &Connection, kind: RunKind) -> Result<RunSummary> {
    let mut best = Keyed::default();
    let mut statement = db
        .prepare(
            "SELECT variant, MAX(score) AS score,
               (SELECT r2.playedAt FROM runs r2 WHERE r2.kind = r.kind AND r2.variant = r.variant
                 ORDER BY r2.score DESC, r2.playedAt ASC LIMIT 1) AS playedAt
             FROM runs r WHERE kind = ?1 GROUP BY variant",
        )
        .sql()?;
    let rows = statement
        .query_map(params![kind.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .sql()?;
    for row in rows {
        let (variant, score, played_at) = row.sql()?;
        best.0.push((variant, RunBest { score, played_at }));
    }

    let mut recent = Vec::new();
    let mut statement = db
        .prepare("SELECT id, kind, variant, score, detail, playedAt FROM runs WHERE kind = ?1 ORDER BY playedAt DESC, id DESC LIMIT ?2")
        .sql()?;
    let rows = statement
        .query_map(params![kind.as_str(), RECENT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .sql()?;
    for row in rows {
        let (id, variant, score, detail, played_at) = row.sql()?;
        // A detail that is not a JSON object reads as an empty one.
        let detail = match serde_json::from_str::<Map<String, Value>>(&detail) {
            Ok(detail) => detail,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "runs",
                    &format!("Run row has invalid detail JSON: {cause}"),
                );
                Map::new()
            }
        };
        recent.push(RunRecord {
            id,
            kind,
            variant,
            score,
            detail,
            played_at,
        });
    }

    let total: i64 = db
        .query_row(
            "SELECT COUNT(*) AS n FROM runs WHERE kind = ?1",
            params![kind.as_str()],
            |row| row.get(0),
        )
        .sql()?;
    Ok(RunSummary {
        kind,
        best,
        recent,
        total,
    })
}

/// Record a run; `isBest` when it beats every earlier score of its variant.
pub fn save_run(host: &dyn Host, db: &Connection, run: &RunInput) -> Result<RunSaved> {
    let (kind, variant, score) = (run.kind.as_str(), run.variant.as_str(), run.score);
    let detail = js_json(&Value::Object(run.detail.clone()));
    let previous: Option<f64> = db
        .query_row(
            "SELECT MAX(score) AS score FROM runs WHERE kind = ?1 AND variant = ?2",
            params![kind, variant],
            |row| row.get(0),
        )
        .sql()?;
    db.execute(
        "INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![kind, variant, score, detail, now_ms()],
    )
    .sql()?;
    let is_best = score > 0.0 && previous.is_none_or(|best| score > best);
    Ok(RunSaved {
        summary: run_summary(host, db, run.kind)?,
        is_best,
    })
}

/// Forget the local scores of one kind, or of all of them.
pub fn clear_runs(db: &Connection, kind: Option<RunKind>) -> Result<()> {
    match kind {
        Some(kind) => db.execute("DELETE FROM runs WHERE kind = ?1", params![kind.as_str()]),
        None => db.execute("DELETE FROM runs", []),
    }
    .map(|_| ())
    .sql()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_host::TestHost;
    use serde_json::json;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        db
    }

    /// A run as the game sends it; the variant and detail may be left out here.
    fn save(db: &Connection, host: &TestHost, mut sent: Value) -> Value {
        sent.as_object_mut()
            .unwrap()
            .entry("variant")
            .or_insert(json!(""));
        sent.as_object_mut()
            .unwrap()
            .entry("detail")
            .or_insert(json!({}));
        let run: RunInput = serde_json::from_value(sent).unwrap();
        serde_json::to_value(save_run(host, db, &run).unwrap()).unwrap()
    }

    fn summary(db: &Connection, host: &TestHost, kind: RunKind) -> Value {
        serde_json::to_value(run_summary(host, db, kind).unwrap()).unwrap()
    }

    #[test]
    fn an_empty_kind_summarises_to_nothing() {
        let db = database();
        let host = TestHost::default();
        assert_eq!(
            summary(&db, &host, RunKind::Storm),
            json!({ "kind": "storm", "best": {}, "recent": [], "total": 0 })
        );
    }

    #[test]
    fn saving_reports_a_new_best_only_when_it_beats_the_variant() {
        let db = database();
        let host = TestHost::default();
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
        let host = TestHost::default();
        for score in 1..=(RECENT + 5) {
            save(
                &db,
                &host,
                json!({ "kind": "rush", "variant": "5m", "score": score }),
            );
        }
        let summary = summary(&db, &host, RunKind::Rush);
        assert_eq!(summary["total"], json!(RECENT + 5));
        assert_eq!(summary["recent"].as_array().unwrap().len(), RECENT as usize);
        assert_eq!(summary["recent"][0]["score"], json!(RECENT + 5));
        assert_eq!(summary["best"]["5m"]["score"], json!(RECENT + 5));
    }

    #[test]
    fn a_detail_that_is_not_json_reads_as_empty_and_is_logged() {
        let db = database();
        let host = TestHost::default();
        db.execute(
            "INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES ('storm', '3m', 5, 'not json', 1)",
            [],
        )
        .unwrap();
        let summary = summary(&db, &host, RunKind::Storm);
        assert_eq!(summary["recent"][0]["detail"], json!({}));
        let logged = host.logs();
        assert_eq!(logged.len(), 1);
        assert!(logged[0].contains("[runs] Run row has invalid detail JSON"));
    }

    #[test]
    fn clearing_forgets_one_kind_or_every_kind() {
        let db = database();
        let host = TestHost::default();
        for kind in ["storm", "streak", "rush"] {
            save(&db, &host, json!({ "kind": kind, "score": 3 }));
        }
        let total = |kind| summary(&db, &host, kind)["total"].clone();
        clear_runs(&db, Some(RunKind::Storm)).unwrap();
        assert_eq!(total(RunKind::Storm), json!(0));
        assert_eq!(total(RunKind::Streak), json!(1));
        clear_runs(&db, None).unwrap();
        assert_eq!(total(RunKind::Streak), json!(0));
        assert_eq!(total(RunKind::Rush), json!(0));
    }

    /// A run's row as earlier builds wrote it: the detail in the order sent, whole numbers
    /// without a fraction.
    #[test]
    fn a_saved_run_is_stored_as_before() {
        let db = database();
        let host = TestHost::default();
        save(
            &db,
            &host,
            json!({ "kind": "storm", "variant": "3min", "score": 21,
                    "detail": { "moves": 30, "accuracy": 87.5, "combo": 4, "hard": true } }),
        );
        let row: (String, String, f64, String) = db
            .query_row("SELECT kind, variant, score, detail FROM runs", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        assert_eq!(
            row,
            (
                "storm".into(),
                "3min".into(),
                21.0,
                r#"{"moves":30,"accuracy":87.5,"combo":4,"hard":true}"#.into()
            )
        );
        assert_eq!(
            summary(&db, &host, RunKind::Storm)["recent"][0]["detail"],
            json!({ "moves": 30, "accuracy": 87.5, "combo": 4, "hard": true })
        );
    }
}
