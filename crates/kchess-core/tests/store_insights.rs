//! Insights output parity and opening-family ordering (`store/insights.rs`).

mod support;

use serde_json::{Value, json};
use support::store::{Store, game, profile};

#[derive(Clone, Copy, Default)]
struct Counts {
    total: usize,
    win: usize,
    loss: usize,
    draw: usize,
}

impl Counts {
    fn add(&mut self, result: &str) {
        self.total += 1;
        match result {
            "win" => self.win += 1,
            "loss" => self.loss += 1,
            _ => self.draw += 1,
        }
    }

    fn json(self) -> Value {
        json!({ "total": self.total, "win": self.win, "loss": self.loss, "draw": self.draw })
    }
}

fn fresh() -> Store {
    let store = profile();
    store.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1)");
    store
}

fn save_games(store: &Store, games: Vec<Value>) {
    store.ok(
        "store.games.saveGames",
        vec![json!("Alice"), json!(games), json!(1), json!(true)],
    );
}

fn insights(store: &Store) -> Value {
    store.ok(
        "store.insights.insights",
        vec![json!({ "account": "Alice" })],
    )
}

/// UTC milliseconds from a Gregorian civil date.
fn epoch_ms(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    (((days * 24 + hour) * 60 + minute) * 60) * 1000
}

#[test]
fn combined_local_time_matches_the_original_weekday_and_hour_results_at_boundaries() {
    let store = fresh();
    let mut games = Vec::new();
    let mut index = 0;
    // Include half-hour samples around Japan's historical spring/autumn DST periods and modern
    // dates. The direct SQL below is the pre-change two-expression oracle in the active host zone.
    for year in 1948..=1951 {
        for (month, day) in [(5, 1), (9, 10)] {
            let center = epoch_ms(year, month, day, 0, 0);
            for half_hour in -48..=48 {
                let created_at = center + half_hour * 30 * 60_000;
                let winner = match index % 3 {
                    0 => json!("white"),
                    1 => json!("black"),
                    _ => Value::Null,
                };
                games.push(game(
                    &format!("Timezone{index:04}"),
                    &[("createdAt", json!(created_at)), ("winner", winner)],
                ));
                index += 1;
            }
        }
    }
    for hour in 0..48 {
        let created_at = epoch_ms(2026, 3, 8, 0, 0) + hour * 30 * 60_000;
        games.push(game(
            &format!("Timezone{index:04}"),
            &[("createdAt", json!(created_at))],
        ));
        index += 1;
    }
    save_games(&store, games);

    let oracle = store.rows(
        "SELECT
           CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
           CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
           CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END AS result
         FROM games WHERE account = 'Alice' AND status NOT IN ('created', 'started')
         ORDER BY createdAt DESC LIMIT 20000",
    );
    let mut weekdays = [Counts::default(); 7];
    let mut hours = [Counts::default(); 24];
    for row in &oracle {
        let result = row["result"].as_str().expect("result");
        weekdays[row["weekday"].as_u64().expect("weekday") as usize].add(result);
        hours[row["hour"].as_u64().expect("hour") as usize].add(result);
    }

    let report = insights(&store);
    assert_eq!(report["total"], json!(oracle.len()));
    assert_eq!(
        report["byWeekday"],
        json!(weekdays.into_iter().map(Counts::json).collect::<Vec<_>>())
    );
    assert_eq!(
        report["byHour"],
        json!(hours.into_iter().map(Counts::json).collect::<Vec<_>>())
    );
}

#[test]
fn repeated_timestamps_share_a_calendar_value_without_changing_report_order() {
    let store = fresh();
    let mut games = Vec::new();
    for minute in 0..24 {
        // Multiple games can share one server timestamp. Reuse exact values around a historical
        // Tokyo DST transition so the cache key remains the original instant, not a UTC bucket.
        let created_at = epoch_ms(1950, 9, 10, 0, 0) + minute * 30 * 60_000;
        for duplicate in 0..8 {
            let index = minute * 8 + duplicate;
            let winner = match index % 3 {
                0 => json!("white"),
                1 => json!("black"),
                _ => Value::Null,
            };
            games.push(game(
                &format!("Repeated{index:04}"),
                &[
                    ("createdAt", json!(created_at)),
                    ("winner", winner),
                    (
                        "color",
                        json!(if index % 2 == 0 { "white" } else { "black" }),
                    ),
                ],
            ));
        }
    }
    save_games(&store, games);

    // The previous per-row SQLite expression is the oracle for both calendar counts and
    // newest-first result ordering used by streak calculations.
    let oracle = store.rows(
        "SELECT id, createdAt,
           CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
           CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
           CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END AS result
         FROM games WHERE account = 'Alice' AND status NOT IN ('created', 'started')
         ORDER BY createdAt DESC LIMIT 20000",
    );
    let mut weekdays = [Counts::default(); 7];
    let mut hours = [Counts::default(); 24];
    let mut longest_win = 0;
    let mut longest_loss = 0;
    let mut run = 0;
    let mut run_kind = "";
    for row in &oracle {
        let result = row["result"].as_str().expect("result");
        weekdays[row["weekday"].as_u64().expect("weekday") as usize].add(result);
        hours[row["hour"].as_u64().expect("hour") as usize].add(result);
    }
    for row in oracle.iter().rev() {
        let result = row["result"].as_str().expect("result");
        if result == run_kind {
            run += 1;
        } else {
            run_kind = result;
            run = 1;
        }
        if run_kind == "win" {
            longest_win = longest_win.max(run);
        }
        if run_kind == "loss" {
            longest_loss = longest_loss.max(run);
        }
    }
    let current = match run_kind {
        "win" => run as i64,
        "loss" => -(run as i64),
        _ => 0,
    };

    let report = insights(&store);
    assert_eq!(report["total"], json!(oracle.len()));
    assert_eq!(
        report["byWeekday"],
        json!(weekdays.into_iter().map(Counts::json).collect::<Vec<_>>())
    );
    assert_eq!(
        report["byHour"],
        json!(hours.into_iter().map(Counts::json).collect::<Vec<_>>())
    );
    assert_eq!(
        report["streaks"],
        json!({ "longestWin": longest_win, "longestLoss": longest_loss, "current": current })
    );
}

#[test]
fn many_opening_families_keep_first_seen_order_for_equal_counts() {
    let store = fresh();
    let family_count = 1_024;
    let mut games = Vec::with_capacity(family_count * 2);
    for family in 0..family_count {
        for variation in ["line A", "line B"] {
            let index = games.len();
            games.push(game(
                &format!("Opening{index:04}"),
                &[
                    (
                        "createdAt",
                        json!(1_700_000_000_000_i64 + index as i64 * 1_000),
                    ),
                    ("opening", json!(format!("Family {family:04}: {variation}"))),
                ],
            ));
        }
    }
    save_games(&store, games);

    let report = insights(&store);
    assert_eq!(report["total"], json!(family_count * 2));
    let openings = report["byOpening"].as_array().expect("opening families");
    assert_eq!(openings.len(), 25);
    for (index, opening) in openings.iter().enumerate() {
        assert_eq!(opening["name"], json!(format!("Family {index:04}")));
        assert_eq!(opening["record"]["total"], json!(2));
        assert_eq!(opening["asWhite"], json!(2));
    }
}
