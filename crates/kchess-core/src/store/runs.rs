//! Local scores for Storm, Streak, Rush and the practice drills.
//! Neither is ever sent anywhere.

use kchess_contracts::types::{
    Keyed, RunBest, RunCard, RunInput, RunKind, RunRecord, RunSaved, RunSummary,
};
use kchess_domain::training::repetition;
use rusqlite::{Connection, params};
use serde_json::{Map, Value};

use super::library::js_json;
use super::sql::Sql as _;
use crate::host::{Host, Level};
use crate::time::now_ms;
use kchess_contracts::error::Result;

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

/// The drills of `kind` missed at least once (a run that scored nothing), each with the card
/// its runs leave in order, the one due soonest first.
pub fn run_schedule(db: &Connection, kind: RunKind) -> Result<Vec<RunCard>> {
    let mut statement = db
        .prepare(
            "SELECT variant, score, playedAt FROM runs
             WHERE kind = ?1 AND variant IN
               (SELECT variant FROM runs WHERE kind = ?1 AND score <= 0)
             ORDER BY variant, playedAt, id",
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
    let mut answers: Vec<(String, Vec<(bool, f64)>)> = Vec::new();
    for row in rows {
        let (variant, score, played_at) = row.sql()?;
        let answer = (score > 0.0, played_at as f64);
        match answers.last_mut() {
            Some((last, list)) if *last == variant => list.push(answer),
            _ => answers.push((variant, vec![answer])),
        }
    }
    let mut cards: Vec<RunCard> = answers
        .into_iter()
        .filter_map(|(variant, answers)| {
            Some(RunCard {
                variant,
                card: repetition::replay(answers)?,
            })
        })
        .collect();
    cards.sort_by(|a, b| a.card.due.total_cmp(&b.card.due));
    Ok(cards)
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

    /// Drills missed at least once come back by their runs in order: one missed and then solved
    /// is due a day after; one only missed is due at once, first; one never missed has no card;
    /// another kind's misses are its own.
    #[test]
    fn missed_drills_come_back_by_their_runs() {
        let db = database();
        let run = |kind: &str, variant: &str, score: f64, at: i64| {
            db.execute(
                "INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES (?1, ?2, ?3, '{}', ?4)",
                params![kind, variant, score, at],
            )
            .unwrap();
        };
        const DAY: i64 = 86_400_000;
        let now = 1_800_000_000_000_i64;
        run("endgame", "kr-vs-k", 0.0, now);
        run("endgame", "kr-vs-k", 1.0, now + 60_000);
        run(
            "endgame",
            "own:8/8/8/8/8/8/1k6/R3K3 w - - 0 1",
            0.0,
            now + 120_000,
        );
        run("endgame", "kq-vs-k", 1.0, now);
        run("lesson", "opposition-1", 0.0, now);
        let cards = run_schedule(&db, RunKind::Endgame).unwrap();
        let due: Vec<(&str, f64, u32)> = cards
            .iter()
            .map(|card| (card.variant.as_str(), card.card.due, card.card.lapses))
            .collect();
        assert_eq!(
            due,
            [
                (
                    "own:8/8/8/8/8/8/1k6/R3K3 w - - 0 1",
                    (now + 120_000) as f64,
                    1
                ),
                ("kr-vs-k", (now + 60_000 + DAY) as f64, 1),
            ]
        );
        assert!(run_schedule(&db, RunKind::Storm).unwrap().is_empty());
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
