//! Game reviews on disk. A review is the scores of every
//! position; the summary beside it (accuracy and counts per side) is what the game list shows.

use rusqlite::{Connection, OptionalExtension, params, types::Value as Sql};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::library::js_json;
use super::sql::{Sql as _, transaction};
use crate::host::{Host, Level};
use crate::time::now_ms;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{Keyed, ReviewSource, ReviewSummary, ReviewUpdate, StoredReview};
use kchess_domain::js::truthy;
use kchess_domain::review::GameAnalysis;

/// `parse` for a stored review or summary: a failure is logged.
fn parse_logged<T: DeserializeOwned>(host: &dyn Host, text: &str) -> Option<T> {
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
    review: StoredReview,
    summary: Option<ReviewSummary>,
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
    // A row that is not a review reads as none: a review written over it replaces it.
    let Some(review) = parse_logged(host, &data) else {
        return Ok(None);
    };
    // A summary that cannot be read is worked out again.
    let summary = parse_logged::<Option<ReviewSummary>>(host, &summary).flatten();
    Ok(Some(Stored { review, summary }))
}

/// The stored review, or None.
pub fn read_review(host: &dyn Host, db: &Connection, key: &str) -> Result<Option<StoredReview>> {
    Ok(read_stored(host, db, key)?.map(|stored| stored.review))
}

/// Which review to keep: Lichess's own beats ours, a finished one beats one in progress.
fn rank(review: &StoredReview) -> u8 {
    2 * u8::from(review.complete) + u8::from(review.source == ReviewSource::Lichess)
}

/// The scores a review keeps for the game list, which fails for a review the core could not
/// have written.
pub fn summarize_stored(review: &StoredReview) -> Result<ReviewSummary> {
    kchess_domain::review::summarize(review).map_err(cannot_analyse)
}

/// Labels, accuracy and chances for every move of a stored review.
pub fn analyse_stored(review: &StoredReview) -> Result<GameAnalysis> {
    kchess_domain::review::analyse(review).map_err(cannot_analyse)
}

fn cannot_analyse(reason: String) -> CoreError {
    CoreError::new(format!("This review cannot be analysed: {reason}."))
}

fn to_text(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_value(value)
        .map(|value| js_json(&value))
        .map_err(|cause| CoreError::new(cause.to_string()))
}

/// Save `review` unless a better one is stored; returns what is stored.
pub fn write_review(
    host: &dyn Host,
    db: &Connection,
    review: StoredReview,
) -> Result<ReviewUpdate> {
    let key = review.key.clone();
    let input_game = review.game_id.clone().filter(|game| !game.is_empty());
    let link = |db: &Connection| -> Result<()> {
        if let Some(game) = &input_game {
            db.execute(
                "INSERT OR IGNORE INTO game_reviews (gameId, reviewKey) VALUES (?1, ?2)",
                params![game, key],
            )
            .sql()?;
        }
        Ok(())
    };
    let existing = read_stored(host, db, &key)?;
    if let Some(existing) = existing
        .as_ref()
        .filter(|e| rank(&e.review) > rank(&review))
    {
        link(db)?;
        let mut kept = existing.review.clone();
        kept.game_id = review.game_id.or(kept.game_id);
        let summary = match &existing.summary {
            Some(summary) => summary.clone(),
            None => summarize_stored(&existing.review)?,
        };
        return Ok(ReviewUpdate {
            review: kept,
            summary,
        });
    }
    let mut merged = review;
    merged.game_id = merged
        .game_id
        .take()
        .or_else(|| existing.and_then(|e| e.review.game_id));
    merged.updated_at = now_ms();
    let summary = summarize_stored(&merged)?;
    db.execute(
        "INSERT INTO reviews (key, gameId, source, complete, depth, data, summary, updatedAt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(key) DO UPDATE SET gameId=excluded.gameId, source=excluded.source,
           complete=excluded.complete, depth=excluded.depth, data=excluded.data,
           summary=excluded.summary, updatedAt=excluded.updatedAt",
        params![
            key,
            merged.game_id.as_deref(),
            merged.source.as_str(),
            i64::from(merged.complete),
            merged.depth,
            to_text(&merged)?,
            to_text(&summary)?,
            merged.updated_at,
        ],
    )
    .sql()?;
    link(db)?;
    Ok(ReviewUpdate {
        review: merged,
        summary,
    })
}

/// Save a batch atomically; a failure or cancellation keeps the previous batch.
pub fn write_reviews(
    host: &dyn Host,
    db: &Connection,
    reviews: &[StoredReview],
) -> Result<Vec<StoredReview>> {
    transaction(db, || {
        reviews
            .iter()
            .map(|review| Ok(write_review(host, db, review.clone())?.review))
            .collect()
    })
}

/// A JSON number as a SQLite value (integers stay integers).
use super::sql::number as sql_number;

/// The summaries of these Lichess games' reviews, by game id.
pub fn review_summaries(
    host: &dyn Host,
    db: &Connection,
    ids: &[String],
) -> Result<Keyed<ReviewSummary>> {
    let mut result = Keyed::default();
    if ids.is_empty() {
        return Ok(result);
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
    for row in rows {
        let (game, summary) = row.sql()?;
        let Some(summary) = parse_logged::<Value>(host, &summary).filter(|s| truthy(Some(s)))
        else {
            continue;
        };
        match serde_json::from_value(summary) {
            Ok(summary) => result.insert(game, summary),
            Err(cause) => host.log(
                Level::Warn,
                "reviews",
                &format!("Unreadable review summary of {game}: {cause}"),
            ),
        }
    }
    Ok(result)
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
