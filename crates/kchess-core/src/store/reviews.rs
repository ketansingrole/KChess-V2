//! Game reviews on disk. A review is the scores of every
//! position; the summary beside it (accuracy and counts per side) is what the game list shows.

use rusqlite::{Connection, OptionalExtension, params, types::Value as Sql};
use serde_json::{Map, Value, json};

use super::library::js_json;
use super::sql::{Sql as _, transaction};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use crate::time::now_ms;
use kchess_domain::js::truthy;

/// `parse` for a stored review or summary: a failure is logged.
fn parse_logged(host: &dyn Host, text: &str) -> Option<Value> {
    match serde_json::from_str(text) {
        Ok(value) => Some(value),
        Err(cause) => {
            host.log(
                Level::Debug,
                "review-store",
                &format!("Review parse failed: {cause}"),
            );
            None
        }
    }
}

/// A stored review's row: the review, and its summary when one is stored.
struct Stored {
    review: Value,
    summary: Option<Value>,
}

fn read_stored(host: &dyn Host, db: &Connection, key: &str) -> Result<Option<Stored>> {
    let row = db
        .query_row(
            "SELECT key, data, summary FROM reviews WHERE key = ?1",
            params![key],
            |row| Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .sql()?;
    let Some((data, summary)) = row else {
        return Ok(None);
    };
    // A review that is not JSON, or is a falsy value, reads as no review (`!review`).
    let Some(review) = parse_logged(host, &data).filter(|review| truthy(Some(review))) else {
        return Ok(None);
    };
    let summary = parse_logged(host, &summary).filter(|s| !s.is_null());
    Ok(Some(Stored { review, summary }))
}

/// `readReview(key)`: the stored review, or None.
pub fn read_review(host: &dyn Host, db: &Connection, key: &str) -> Result<Option<Value>> {
    Ok(read_stored(host, db, key)?.map(|stored| stored.review))
}

/// Which review to keep: Lichess's own beats ours, a finished one beats one in progress.
fn rank(review: &Value) -> i64 {
    let complete = if truthy(review.get("complete")) { 2 } else { 0 };
    let lichess = i64::from(review.get("source").and_then(Value::as_str) == Some("lichess"));
    complete + lichess
}

/// `a ?? b` for an optional JSON value: null and absent fall through to `b`.
fn coalesce(a: Option<&Value>, b: Option<&Value>) -> Option<Value> {
    match a {
        Some(value) if !value.is_null() => Some(value.clone()),
        _ => b.cloned(),
    }
}

/// `{ ...review, gameId }`: an absent game id is left out, as `JSON.stringify` leaves out undefined.
fn with_game_id(review: &Value, game_id: Option<Value>) -> Result<Map<String, Value>> {
    let mut merged = review
        .as_object()
        .cloned()
        .ok_or_else(|| CoreError::new("review must be an object"))?;
    match game_id {
        Some(value) => {
            merged.insert("gameId".into(), value);
        }
        None => {
            merged.shift_remove("gameId");
        }
    }
    Ok(merged)
}

/// The scores a review keeps for the game list (`summarize`), which fails for a review the
/// core could not have written.
pub fn summarize_stored(review: &Value) -> Result<Value> {
    let input = review_input(review)?;
    kchess_domain::review::summarize(&input).map_err(cannot_analyse)
}

/// `analyseReview`: labels, accuracy and chances for every move of a stored review.
pub fn analyse_stored(review: &Value) -> Result<Value> {
    let input = review_input(review)?;
    kchess_domain::review::analyse_review(&input).map_err(cannot_analyse)
}

fn cannot_analyse(reason: String) -> CoreError {
    CoreError::new(format!("This review cannot be analysed: {reason}."))
}

/// `reviewInput`: the fields the review analysis reads (scores without engine lines).
fn review_input(review: &Value) -> Result<Value> {
    let mut out = Map::new();
    // `variant`: a variant game's moves replay by its own rules.
    for key in ["key", "source", "complete", "fen", "moves", "variant"] {
        if let Some(value) = review.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    let evals = review
        .get("evals")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::new("Cannot read properties of undefined (reading 'map')"))?;
    let scores: Vec<Value> = evals
        .iter()
        .map(|e| {
            if let Some(object) = e.as_object() {
                let mut score = Map::new();
                for key in ["cp", "mate"] {
                    if let Some(value) = object.get(key) {
                        score.insert(key.into(), value.clone());
                    }
                }
                Value::Object(score)
            } else {
                e.clone()
            }
        })
        .collect();
    out.insert("evals".into(), Value::Array(scores));
    for key in ["judgments", "accuracy"] {
        if let Some(value) = review.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    Ok(Value::Object(out))
}

/// `writeReview(review)`: save `review` unless a better one is stored; returns what is stored.
pub fn write_review(host: &dyn Host, db: &Connection, input: &Value) -> Result<Value> {
    let key = input
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::new("key must be a string"))?
        .to_string();
    let input_game = input.get("gameId").filter(|v| truthy(Some(v))).cloned();
    let link = |db: &Connection| -> Result<()> {
        if let Some(game) = input_game.as_ref().and_then(Value::as_str) {
            db.execute(
                "INSERT OR IGNORE INTO game_reviews (gameId, reviewKey) VALUES (?1, ?2)",
                params![game, key],
            )
            .sql()?;
        }
        Ok(())
    };
    let existing = read_stored(host, db, &key)?;
    if let Some(existing) = existing.as_ref().filter(|e| rank(&e.review) > rank(input)) {
        link(db)?;
        let game_id = coalesce(input.get("gameId"), existing.review.get("gameId"));
        let review = with_game_id(&existing.review, game_id)?;
        let summary = match &existing.summary {
            Some(summary) => summary.clone(),
            None => summarize_stored(&existing.review)?,
        };
        return Ok(json!({ "review": review, "summary": summary }));
    }
    let existing_game = existing
        .as_ref()
        .and_then(|e| e.review.get("gameId"))
        .cloned();
    let game_id = coalesce(input.get("gameId"), existing_game.as_ref());
    let mut merged = with_game_id(input, game_id.clone())?;
    merged.insert("updatedAt".into(), json!(now_ms()));
    let merged = Value::Object(merged);
    let summary = summarize_stored(&merged)?;
    let data = js_json(&merged);
    let summary_text = js_json(&summary);
    db.execute(
        "INSERT INTO reviews (key, gameId, source, complete, depth, data, summary, updatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(key) DO UPDATE SET gameId=excluded.gameId, source=excluded.source,
           complete=excluded.complete, depth=excluded.depth, data=excluded.data,
           summary=excluded.summary, updatedAt=excluded.updatedAt",
        params![
            key,
            game_id.as_ref().and_then(Value::as_str),
            merged
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            i64::from(truthy(merged.get("complete"))),
            sql_number(merged.get("depth")),
            data,
            summary_text,
            merged
                .get("updatedAt")
                .and_then(Value::as_f64)
                .unwrap_or_default() as i64,
        ],
    )
    .sql()?;
    link(db)?;
    Ok(json!({ "review": merged, "summary": summary }))
}

/// `writeReviews(reviews)`: save a batch atomically; a failure or cancellation
/// keeps the previous batch. Callers that need per-review cancellability keep
/// using `writeReview`.
pub fn write_reviews(host: &dyn Host, db: &Connection, reviews: &Value) -> Result<Value> {
    let list = reviews.as_array().cloned().unwrap_or_default();
    transaction(db, || {
        let mut stored = Vec::with_capacity(list.len());
        for review in &list {
            let written = write_review(host, db, review)?;
            stored.push(written.get("review").cloned().unwrap_or(Value::Null));
        }
        Ok(Value::Array(stored))
    })
}

/// A JSON number as a SQLite value (integers stay integers).
fn sql_number(value: Option<&Value>) -> Sql {
    match value.and_then(Value::as_f64) {
        Some(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Sql::Integer(n as i64),
        Some(n) => Sql::Real(n),
        None => Sql::Null,
    }
}

/// `reviewSummaries(ids)`: the summaries of these Lichess games' reviews, by game id.
pub fn review_summaries(host: &dyn Host, db: &Connection, ids: &[String]) -> Result<Value> {
    if ids.is_empty() {
        return Ok(json!({}));
    }
    let marks = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "SELECT gr.gameId, r.summary FROM game_reviews gr JOIN reviews r ON r.key=gr.reviewKey
         WHERE gr.gameId IN ({marks}) ORDER BY r.complete, r.updatedAt"
    );
    let mut statement = db.prepare(&sql).sql()?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .sql()?;
    let mut result = Map::new();
    for row in rows {
        let (game, summary) = row.sql()?;
        if let Some(summary) = parse_logged(host, &summary).filter(|s| truthy(Some(s))) {
            result.insert(game, summary);
        }
    }
    Ok(Value::Object(result))
}

/// Whether this account is still on this device.
pub fn has_account(db: &Connection, username: &str) -> Result<bool> {
    let row = db
        .query_row(
            "SELECT 1 FROM accounts WHERE username = ?1 COLLATE NOCASE",
            params![username],
            |_| Ok(()),
        )
        .optional()
        .sql()?;
    Ok(row.is_some())
}

/// Remember that Lichess was asked for these games' analysis.
pub fn mark_checked(db: &Connection, ids: &[String], at: f64) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    transaction(db, || {
        for id in ids {
            db.execute(
                "INSERT OR REPLACE INTO lichess_review_checks (id, checkedAt) VALUES (?1, ?2)",
                params![id, sql_number(Some(&json!(at)))],
            )
            .sql()?;
        }
        Ok(())
    })
}

/// The accounts' finished games with no finished review, newest first.
pub fn games_to_review(
    db: &Connection,
    accounts: &[String],
    since: f64,
    limit: f64,
) -> Result<Value> {
    if accounts.is_empty() {
        return Ok(json!([]));
    }
    let marks = vec!["?"; accounts.len()].join(", ");
    let sql = format!(
        // A Chess.com game (`cc-`) has no Lichess analysis to look up: it counts as checked.
        "SELECT g.id, g.account, g.moves, g.pgn, g.perf, g.createdAt,
           (c.id IS NOT NULL OR substr(g.id, 1, 3) = 'cc-') AS checked
         FROM games g
         LEFT JOIN lichess_review_checks c ON c.id = g.id
         WHERE g.account COLLATE NOCASE IN ({marks}) AND g.createdAt >= ? AND NOT EXISTS (
           SELECT 1 FROM game_reviews gr JOIN reviews r ON r.key=gr.reviewKey
           WHERE gr.gameId=g.id AND r.complete=1)
           AND g.moves != '' AND g.status NOT IN ('created', 'started')
         GROUP BY g.id
         ORDER BY g.createdAt DESC
         LIMIT ?"
    );
    let mut params_list: Vec<Sql> = accounts.iter().map(|a| Sql::Text(a.clone())).collect();
    params_list.push(sql_number(Some(&json!(since))));
    params_list.push(sql_number(Some(&json!(limit))));
    let mut statement = db.prepare(&sql).sql()?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params_list.iter()), |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "account": row.get::<_, String>(1)?,
                "moves": row.get::<_, String>(2)?,
                "pgn": row.get::<_, Option<String>>(3)?,
                "perf": row.get::<_, String>(4)?,
                // The queue parses this row as `GameToReview` (`created_at: i64`); a floating
                // point reading fails the whole pass with "expected i64".
                "createdAt": row.get::<_, i64>(5)?,
                "checked": row.get::<_, i64>(6)? == 1,
            }))
        })
        .sql()?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.sql()?);
    }
    Ok(Value::Array(out))
}
