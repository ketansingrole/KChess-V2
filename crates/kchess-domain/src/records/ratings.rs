//! `core/src/domain/ratings.ts`: rating-history keys, display names, histories rebuilt from
//! games and the merge of Lichess's own histories with them.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use super::{Out, arg, list};
use crate::js;

const DAY_MS: f64 = 86_400_000.0;
/// `ms` for `Date`'s range: beyond it a JavaScript Date is invalid.
const MAX_DATE_MS: f64 = 8.64e15;

/// `RATING_DISPLAY_NAMES`: the label for each rating-history key.
pub const DISPLAY_NAMES: &[(&str, &str)] = &[
    ("ultraBullet", "UltraBullet"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
    ("chess960", "Chess960"),
    ("crazyhouse", "Crazyhouse"),
    ("antichess", "Antichess"),
    ("atomic", "Atomic"),
    ("horde", "Horde"),
    ("kingOfTheHill", "King of the Hill"),
    ("racingKings", "Racing Kings"),
    ("threeCheck", "Three-check"),
    ("puzzle", "Puzzles"),
];

/// `KEY_BY_NORMALIZED`: lowercase names without spaces, `_` or `-`, to their key.
const KEY_BY_NORMALIZED: &[(&str, &str)] = &[
    ("ultrabullet", "ultraBullet"),
    ("bullet", "bullet"),
    ("blitz", "blitz"),
    ("rapid", "rapid"),
    ("classical", "classical"),
    ("correspondence", "correspondence"),
    ("chess960", "chess960"),
    ("crazyhouse", "crazyhouse"),
    ("antichess", "antichess"),
    ("atomic", "atomic"),
    ("horde", "horde"),
    ("kingofthehill", "kingOfTheHill"),
    ("racingkings", "racingKings"),
    ("threecheck", "threeCheck"),
    ("puzzle", "puzzle"),
    ("puzzles", "puzzle"),
];

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "normalizeRatingKey" => Ok(json!(normalize(arg(args, 0)))),
        "ratingDisplayName" => Ok(json!(display_name(arg(args, 0)))),
        "isPuzzleHistory" => Ok(json!(normalize(arg(args, 0)) == "puzzle")),
        "ratingDisplayNames" => Ok(Value::Object(
            DISPLAY_NAMES
                .iter()
                .map(|(key, label)| ((*key).to_string(), json!(label)))
                .collect(),
        )),
        "ratingHistoryFromGames" => list(args, 0, "games").map(history_from_games),
        "mergeRatingHistories" => list(args, 0, "official").and_then(|official| {
            list(args, 1, "fromGames").map(|from_games| merge(official, from_games))
        }),
        _ => return None,
    })
}

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// `normalizeRatingKey`: '' for anything but a non-empty string; an unknown name is kept as it is.
fn normalize(name: Option<&Value>) -> String {
    let Some(name) = name.and_then(Value::as_str).filter(|name| !name.is_empty()) else {
        return String::new();
    };
    let key: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !js::is_space(*c) && *c != '_' && *c != '-')
        .collect();
    lookup(KEY_BY_NORMALIZED, &key).map_or_else(|| name.to_string(), str::to_string)
}

/// `ratingDisplayName`: the label for a key, or the name itself when it has none.
fn display_name(name: Option<&Value>) -> String {
    let Some(text) = name.and_then(Value::as_str).filter(|name| !name.is_empty()) else {
        return String::new();
    };
    lookup(DISPLAY_NAMES, &normalize(name)).map_or_else(|| text.to_string(), str::to_string)
}

/// The rating key of a game's `perf`: `normalizeRatingKey(perf) || perf`. A non-string perf is
/// its own key (undefined when absent).
fn perf_key(perf: Option<&Value>) -> Option<Value> {
    match perf {
        Some(Value::String(_)) => Some(json!(normalize(perf))),
        other => other.cloned(),
    }
}

/// Stands in for the identity of a `Map` key: values are equal when their JSON is.
fn key_id(key: &Option<Value>) -> String {
    match key {
        None => "undefined".into(),
        Some(value) => format!("value:{value}"),
    }
}

/// `createdAt`, as the arithmetic reads it (NaN when absent).
fn created_at(game: &Value) -> f64 {
    js::to_number(game.get("createdAt"))
}

/// A `Map` key for a day: SameValueZero, so NaN and -0 are one key each.
fn day_id(day: f64) -> u64 {
    if day.is_nan() {
        u64::MAX
    } else if day == 0.0 {
        0
    } else {
        day.to_bits()
    }
}

struct Perf {
    name: Option<Value>,
    /// (day start in ms, rating after the day's last game), in first-seen order.
    days: Vec<(f64, f64)>,
    day_index: HashMap<u64, usize>,
}

/// `ratingHistoryFromGames`: one point per UTC day per perf, the day's last rated game.
fn history_from_games(games: &[Value]) -> Value {
    let mut sorted: Vec<&Value> = games.iter().collect();
    sorted.sort_by(|a, b| {
        created_at(a)
            .partial_cmp(&created_at(b))
            .unwrap_or(Ordering::Equal)
    });
    let mut perfs: Vec<Perf> = Vec::new();
    let mut perf_index: HashMap<String, usize> = HashMap::new();
    for game in sorted {
        if js::falsy(game.get("rated")) {
            continue;
        }
        let (Some(before), Some(diff)) = (game.get("playerRating"), game.get("ratingDiff")) else {
            continue;
        };
        let rating = js::to_number(Some(before)) + js::to_number(Some(diff));
        let name = perf_key(game.get("perf"));
        let day = (created_at(game) / DAY_MS).floor() * DAY_MS;
        let slot = match perf_index.get(&key_id(&name)) {
            Some(&slot) => slot,
            None => {
                perfs.push(Perf {
                    name: name.clone(),
                    days: Vec::new(),
                    day_index: HashMap::new(),
                });
                perf_index.insert(key_id(&name), perfs.len() - 1);
                perfs.len() - 1
            }
        };
        let perf = &mut perfs[slot];
        match perf.day_index.get(&day_id(day)) {
            Some(&at) => perf.days[at].1 = rating,
            None => {
                perf.day_index.insert(day_id(day), perf.days.len());
                perf.days.push((day, rating));
            }
        }
    }
    Value::Array(
        perfs
            .into_iter()
            .map(|perf| {
                let mut entry = Map::new();
                if let Some(name) = perf.name {
                    entry.insert("name".into(), name);
                }
                let points = perf
                    .days
                    .iter()
                    .map(|&(day, rating)| {
                        let (year, month0, date) = utc_date(day);
                        json!([year, month0, date, rating])
                    })
                    .collect();
                entry.insert("points".into(), Value::Array(points));
                Value::Object(entry)
            })
            .collect(),
    )
}

/// `[year, month0, day]` of a day start in UTC; nulls where JavaScript's Date is invalid.
fn utc_date(day: f64) -> (Value, Value, Value) {
    if !day.is_finite() || day.abs() > MAX_DATE_MS {
        return (Value::Null, Value::Null, Value::Null);
    }
    let (year, month0, date) = civil_from_days((day / DAY_MS).round() as i64);
    (json!(year), json!(month0), json!(date))
}

/// Days since 1970-01-01 as (year, month 0-11, day 1-31), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let date = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month - 1, date)
}

/// Whether a history entry has points (`entry.points?.length`).
fn has_points(entry: &Value) -> bool {
    match entry.get("points") {
        Some(Value::Array(points)) => !points.is_empty(),
        Some(Value::String(text)) => !text.is_empty(),
        _ => false,
    }
}

/// `mergeRatingHistories`: Lichess's histories, plus the game-built ones for perfs it has no
/// points for.
fn merge(official: &[Value], from_games: &[Value]) -> Value {
    let covered: HashSet<String> = official
        .iter()
        .filter(|entry| has_points(entry))
        .map(|entry| normalize(entry.get("name")))
        .collect();
    let mut merged = official.to_vec();
    merged.extend(
        from_games
            .iter()
            .filter(|entry| !covered.contains(&normalize(entry.get("name"))))
            .cloned(),
    );
    Value::Array(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_days_match_the_gregorian_calendar() {
        assert_eq!(civil_from_days(0), (1970, 0, 1));
        assert_eq!(civil_from_days(-1), (1969, 11, 31));
        assert_eq!(civil_from_days(19_723), (2024, 0, 1));
        // A leap day, and the day after it.
        assert_eq!(civil_from_days(19_782), (2024, 1, 29));
        assert_eq!(civil_from_days(19_783), (2024, 2, 1));
    }

    #[test]
    fn keys_normalize_like_lichess_names() {
        let name = |s: &str| json!(s);
        assert_eq!(normalize(Some(&name("King of the Hill"))), "kingOfTheHill");
        assert_eq!(normalize(Some(&name("Puzzles"))), "puzzle");
        assert_eq!(normalize(Some(&name("Unknown_Thing"))), "Unknown_Thing");
        assert_eq!(normalize(Some(&name(""))), "");
        assert_eq!(normalize(None), "");
    }

    #[test]
    fn invalid_dates_become_null() {
        assert_eq!(utc_date(f64::NAN).0, Value::Null);
        assert_eq!(utc_date(9.0e15).0, Value::Null);
    }
}
