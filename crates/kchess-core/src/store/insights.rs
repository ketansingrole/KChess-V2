//! Patterns in the games synced to this computer (`core/src/services/insights.ts`): results by
//! colour, speed, opening, time of day, opponent strength and game length, and accuracy where
//! games were reviewed. Everything is read locally; nothing is sent to Lichess.

use rusqlite::{Connection, params_from_iter, types::Value as Sql};
use serde_json::{Map, Value, json};

use super::library::{db_err, now_ms, num, truthy};
use crate::error::{CoreError, Result};
use kchess_domain::js;

const NEG_INF: f64 = f64::NEG_INFINITY;
const INF: f64 = f64::INFINITY;

/// Rating difference (opponent minus player) bands, with their labels.
const RATING_BANDS: [(&str, f64, f64); 5] = [
    ("Much weaker (−200 or less)", NEG_INF, -200.0),
    ("Weaker (−199 to −50)", -199.0, -50.0),
    ("Similar (±49)", -49.0, 49.0),
    ("Stronger (+50 to +199)", 50.0, 199.0),
    ("Much stronger (+200 or more)", 200.0, INF),
];

/// Game length bands in plies, with their labels.
const LENGTH_BANDS: [(&str, f64, f64); 4] = [
    ("Under 20 moves", 0.0, 39.0),
    ("20–39 moves", 40.0, 79.0),
    ("40–59 moves", 80.0, 119.0),
    ("60 moves or more", 120.0, INF),
];

const DAY_MS: f64 = 86_400_000.0;
const ROW_LIMIT: i64 = 20_000;
const REVIEW_LIMIT: i64 = 2_000;

/// This module's storage methods; None when the method is not one of them.
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let result = match method {
        "store.insights.insights" => {
            let query = args.first().unwrap_or(&Value::Null);
            now_ms().and_then(|now| insights(db, query, now))
        }
        _ => return None,
    };
    Some(result)
}

/// Win/loss/draw counts.
#[derive(Default)]
struct Record {
    total: i64,
    win: i64,
    loss: i64,
    draw: i64,
}

impl Record {
    fn add(&mut self, result: &str) {
        self.total += 1;
        match result {
            "win" => self.win += 1,
            "loss" => self.loss += 1,
            _ => self.draw += 1,
        }
    }

    fn json(&self) -> Value {
        json!({ "total": self.total, "win": self.win, "loss": self.loss, "draw": self.draw })
    }
}

/// The entry for `key`, created in first-seen order (a JavaScript `Map`).
fn slot<'a, T>(list: &'a mut Vec<(String, T)>, key: &str, make: impl FnOnce() -> T) -> &'a mut T {
    let index = match list.iter().position(|(k, _)| k == key) {
        Some(index) => index,
        None => {
            list.push((key.to_string(), make()));
            list.len() - 1
        }
    };
    &mut list[index].1
}

/// `SQL` value of a query argument: text, a number, or null.
fn sql_of(value: Option<&Value>) -> Sql {
    match value {
        Some(Value::String(s)) => Sql::Text(s.clone()),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => Sql::Integer(x as i64),
            Some(x) => Sql::Real(x),
            None => Sql::Null,
        },
        _ => Sql::Null,
    }
}

/// The filter clauses on `games`, optionally qualified (`g.`) for the review join.
fn clauses(query: &Value, prefix: &str, now: f64) -> Result<(Vec<String>, Vec<Sql>)> {
    let mut clauses = vec![format!("{prefix}account = ?")];
    let mut params = vec![sql_of(query.get("account"))];
    if truthy(query.get("speed")) {
        clauses.push(format!("{prefix}speed = ?"));
        params.push(sql_of(query.get("speed")));
    }
    if let Some(rated) = query.get("rated") {
        clauses.push(format!("{prefix}rated = ?"));
        params.push(Sql::Integer(i64::from(truthy(Some(rated)))));
    }
    if truthy(query.get("days")) {
        let days = js::to_number(query.get("days"));
        clauses.push(format!("{prefix}createdAt >= ?"));
        params.push(Sql::Real(now - days * DAY_MS));
    }
    // In-progress games have no result yet.
    clauses.push(format!("{prefix}status NOT IN ('created', 'started')"));
    Ok((clauses, params))
}

/// `insights(query)`.
fn insights(db: &Connection, query: &Value, now: f64) -> Result<Value> {
    let (clauses, params) = clauses(query, "", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
          (CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END) AS result,
          CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
          CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
          CASE WHEN moves = '' THEN 0 ELSE LENGTH(moves) - LENGTH(REPLACE(moves, ' ', '')) + 1 END AS plies
         FROM games WHERE {where_sql} ORDER BY createdAt DESC LIMIT {ROW_LIMIT}"
    );
    let mut statement = db.prepare(&sql).map_err(db_err)?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(Row {
                color: row.get(1)?,
                speed: row.get(2)?,
                opening: row.get(4)?,
                opponent_rating: row.get(5)?,
                player_rating: row.get(6)?,
                result: row.get(8)?,
                weekday: row.get(9)?,
                hour: row.get(10)?,
                plies: row.get(11)?,
                status: row.get(3)?,
            })
        })
        .map_err(db_err)?;
    let mut rows_out = Vec::new();
    for row in rows {
        rows_out.push(row.map_err(db_err)?);
    }

    let mut record = Record::default();
    let mut by_color = [Record::default(), Record::default()];
    let mut speeds: Vec<(String, Record)> = Vec::new();
    let mut openings: Vec<(String, (Record, i64))> = Vec::new();
    let mut weekdays: Vec<Record> = (0..7).map(|_| Record::default()).collect();
    let mut hours: Vec<Record> = (0..24).map(|_| Record::default()).collect();
    let mut opponents: Vec<Record> = RATING_BANDS.iter().map(|_| Record::default()).collect();
    let mut lengths: Vec<Record> = LENGTH_BANDS.iter().map(|_| Record::default()).collect();
    let mut endings: Vec<(String, Record)> = Vec::new();
    let mut longest_win = 0;
    let mut longest_loss = 0;
    let mut run = 0;
    let mut run_kind = String::new();
    // Rows are newest first; streaks are counted oldest first.
    for row in rows_out.iter().rev() {
        record.add(&row.result);
        let side = match row.color.as_str() {
            "white" => 0,
            "black" => 1,
            _ => {
                return Err(CoreError::new(
                    "Cannot read properties of undefined (reading 'record')",
                ));
            }
        };
        by_color[side].add(&row.result);
        slot(&mut speeds, &row.speed, Record::default).add(&row.result);
        if let Some(opening) = row.opening.as_deref().filter(|o| !o.is_empty()) {
            // Group variations under their opening family: "Sicilian Defense: Najdorf" → "Sicilian Defense".
            let family = js::trim(opening.split(':').next().unwrap_or_default()).to_string();
            let entry = slot(&mut openings, &family, || (Record::default(), 0));
            entry.0.add(&row.result);
            if row.color == "white" {
                entry.1 += 1;
            }
        }
        if (0..7).contains(&row.weekday) {
            weekdays[row.weekday as usize].add(&row.result);
        }
        if (0..24).contains(&row.hour) {
            hours[row.hour as usize].add(&row.result);
        }
        if let (Some(opponent), Some(player)) = (row.opponent_rating, row.player_rating)
            && opponent != 0.0
            && player != 0.0
        {
            let diff = opponent - player;
            if let Some(band) = RATING_BANDS
                .iter()
                .position(|(_, low, high)| diff >= *low && diff <= *high)
            {
                opponents[band].add(&row.result);
            }
        }
        if let Some(band) = LENGTH_BANDS
            .iter()
            .position(|(_, low, high)| row.plies as f64 >= *low && row.plies as f64 <= *high)
        {
            lengths[band].add(&row.result);
        }
        slot(&mut endings, &row.status, Record::default).add(&row.result);
        if row.result == run_kind {
            run += 1;
        } else {
            run_kind = row.result.clone();
            run = 1;
        }
        if run_kind == "win" {
            longest_win = longest_win.max(run);
        }
        if run_kind == "loss" {
            longest_loss = longest_loss.max(run);
        }
    }
    let current = match run_kind.as_str() {
        "win" => run,
        "loss" => -run,
        _ => 0,
    };

    let mut by_speed: Vec<(String, Record)> = speeds;
    by_speed.sort_by(|a, b| b.1.total.cmp(&a.1.total));
    let mut by_opening: Vec<(String, (Record, i64))> = openings
        .into_iter()
        .filter(|(_, (record, _))| record.total >= 2)
        .collect();
    by_opening.sort_by(|a, b| (b.1).0.total.cmp(&(a.1).0.total));
    by_opening.truncate(25);
    let mut by_ending = endings;
    by_ending.sort_by(|a, b| b.1.total.cmp(&a.1.total));

    let mut report = Map::new();
    report.insert(
        "account".into(),
        query.get("account").cloned().unwrap_or(Value::Null),
    );
    report.insert("total".into(), json!(rows_out.len()));
    report.insert("record".into(), record.json());
    report.insert(
        "byColor".into(),
        json!({ "white": by_color[0].json(), "black": by_color[1].json() }),
    );
    report.insert(
        "bySpeed".into(),
        Value::Array(
            by_speed
                .iter()
                .map(|(speed, record)| json!({ "speed": speed, "record": record.json() }))
                .collect(),
        ),
    );
    report.insert(
        "byOpening".into(),
        Value::Array(
            by_opening
                .iter()
                .map(|(name, (record, as_white))| {
                    json!({ "name": name, "record": record.json(), "asWhite": as_white })
                })
                .collect(),
        ),
    );
    report.insert(
        "byWeekday".into(),
        Value::Array(weekdays.iter().map(Record::json).collect()),
    );
    report.insert(
        "byHour".into(),
        Value::Array(hours.iter().map(Record::json).collect()),
    );
    report.insert(
        "byOpponent".into(),
        Value::Array(
            RATING_BANDS
                .iter()
                .zip(opponents.iter())
                .map(|((label, _, _), record)| json!({ "label": label, "record": record.json() }))
                .collect(),
        ),
    );
    report.insert(
        "byLength".into(),
        Value::Array(
            LENGTH_BANDS
                .iter()
                .zip(lengths.iter())
                .map(|((label, _, _), record)| json!({ "label": label, "record": record.json() }))
                .collect(),
        ),
    );
    report.insert(
        "endings".into(),
        Value::Array(
            by_ending
                .iter()
                .map(|(status, record)| json!({ "status": status, "record": record.json() }))
                .collect(),
        ),
    );
    report.insert(
        "streaks".into(),
        json!({ "longestWin": longest_win, "longestLoss": longest_loss, "current": current }),
    );
    if let Some(accuracy) = accuracy_report(db, query, now)? {
        report.insert("accuracy".into(), accuracy);
    }
    Ok(Value::Object(report))
}

/// One game row of the main query.
struct Row {
    color: String,
    speed: String,
    status: String,
    opening: Option<String>,
    opponent_rating: Option<f64>,
    player_rating: Option<f64>,
    result: String,
    weekday: i64,
    hour: i64,
    plies: i64,
}

/// Accuracy and mistake counts of the reviewed games (by Lichess or locally), newest first.
/// None when no game in the filter has a finished review.
fn accuracy_report(db: &Connection, query: &Value, now: f64) -> Result<Option<Value>> {
    let (clauses, params) = clauses(query, "g.", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT g.color AS color, r.summary AS summary FROM games g JOIN reviews r ON r.key = (
           SELECT r2.key FROM game_reviews gr JOIN reviews r2 ON r2.key = gr.reviewKey
           WHERE gr.gameId = g.id AND r2.complete = 1 ORDER BY r2.updatedAt DESC LIMIT 1)
         WHERE {where_sql} AND r.complete = 1
         ORDER BY g.createdAt DESC LIMIT {REVIEW_LIMIT}"
    );
    let mut statement = db.prepare(&sql).map_err(db_err)?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    let mut reviewed = Vec::new();
    for row in rows {
        reviewed.push(row.map_err(db_err)?);
    }
    if reviewed.is_empty() {
        return Ok(None);
    }
    let mut accuracy_sum = 0.0;
    let mut accuracy_games = 0;
    let mut acpl_sum = 0.0;
    let mut acpl_games = 0;
    let (mut inaccuracy, mut mistake, mut blunder) = (0.0, 0.0, 0.0);
    for (color, text) in &reviewed {
        // A summary that is not JSON is left out; one that is null fails the side lookup, as before.
        let Ok(summary) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if summary.is_null() {
            return Err(CoreError::new(format!(
                "Cannot read properties of null (reading '{color}')"
            )));
        }
        let Some(side) = summary.get(color.as_str()).filter(|s| truthy(Some(s))) else {
            continue;
        };
        if let Some(accuracy) = side.get("accuracy").and_then(Value::as_f64) {
            accuracy_sum += accuracy;
            accuracy_games += 1;
        }
        if let Some(acpl) = side.get("acpl").and_then(Value::as_f64) {
            acpl_sum += acpl;
            acpl_games += 1;
        }
        inaccuracy += count_of(side.get("inaccuracy"));
        mistake += count_of(side.get("mistake"));
        blunder += count_of(side.get("blunder"));
    }
    let games = reviewed.len() as f64;
    let mut accuracy = Map::new();
    accuracy.insert("games".into(), json!(reviewed.len()));
    if accuracy_games > 0 {
        accuracy.insert(
            "average".into(),
            num(accuracy_sum / f64::from(accuracy_games)),
        );
    }
    if acpl_games > 0 {
        accuracy.insert("acpl".into(), num(acpl_sum / f64::from(acpl_games)));
    }
    accuracy.insert(
        "perGame".into(),
        json!({
            "inaccuracy": inaccuracy / games,
            "mistake": mistake / games,
            "blunder": blunder / games,
        }),
    );
    Ok(Some(Value::Object(accuracy)))
}

/// `side.x ?? 0` as a count.
fn count_of(value: Option<&Value>) -> f64 {
    match value {
        None | Some(Value::Null) => 0.0,
        other => js::to_number(other),
    }
}
