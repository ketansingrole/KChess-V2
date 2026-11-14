//! Insights output parity and opening-family ordering (`store/insights.rs`).

use crate::support;

use kchess_contracts::types::{InsightsQuery, InsightsReport};
use kchess_core::store::{games, insights};
use serde_json::{Value, json};
use support::store::{Store, game, insights_query, list, profile};

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
    store
        .write(|c| games::save_games(c.host, c.db, "Alice", list(&json!(games)), Some(1)))
        .expect("saveGames");
}

fn insights(store: &Store) -> Value {
    store
        .read(|c| {
            insights::insights(
                c.host,
                c.db,
                &insights_query(&json!({ "account": "Alice" })),
                kchess_core::time::now_ms() as f64,
            )
        })
        .expect("insights")
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

/// Variant games stay out of the standard figures, are listed to choose, and each variant reports
/// its own games.
#[test]
fn variant_games_are_counted_apart_from_standard_chess() {
    let store = fresh();
    save_games(
        &store,
        vec![
            game("Standard01", &[]),
            game("Standard02", &[("winner", json!("black"))]),
            game("House00001", &[("perf", json!("crazyhouse"))]),
            game("Nine600001", &[("perf", json!("chess960"))]),
            game("Nine600002", &[("perf", json!("chess960"))]),
        ],
    );
    let standard = insights(&store);
    assert_eq!(standard["total"], json!(2));
    assert_eq!(standard["record"]["loss"], json!(1));
    // In KChess's order, whichever came first.
    assert_eq!(standard["variants"], json!(["chess960", "crazyhouse"]));

    let only = |variant: &str| {
        store
            .read(|c| {
                insights::insights(
                    c.host,
                    c.db,
                    &insights_query(&json!({ "account": "Alice", "variant": variant })),
                    kchess_core::time::now_ms() as f64,
                )
            })
            .expect("insights")
    };
    assert_eq!(only("chess960")["total"], json!(2));
    assert_eq!(only("crazyhouse")["total"], json!(1));
    assert_eq!(only("standard")["total"], json!(2));
    assert_eq!(only("atomic")["total"], json!(0));
}

const NOW: f64 = 1_800_000_000_000.0;
const DAY: f64 = 86_400_000.0;

/// The typed report for `query`, as of `NOW`.
fn report(store: &Store, query: Value) -> InsightsReport {
    report_of(store, &insights_query(&query))
}

fn report_of(store: &Store, query: &InsightsQuery) -> InsightsReport {
    store
        .db
        .read(|c| insights::insights(c.host, c.db, query, NOW))
        .expect("insights")
}

/// "The last n days" counts the games since n days before now; none counts them all, and so
/// does 0, which the store reads as none (the facade's check refuses it).
#[test]
fn the_last_days_filter_keeps_the_games_since_that_many_days_ago() {
    let store = fresh();
    save_games(
        &store,
        vec![
            game("TwoDaysAg", &[("createdAt", json!(NOW - 2.0 * DAY))]),
            game("TenDaysAg", &[("createdAt", json!(NOW - 10.0 * DAY))]),
        ],
    );
    for (days, total) in [(None, 2), (Some(0), 2), (Some(5), 1), (Some(20), 2)] {
        let query = InsightsQuery {
            days,
            ..insights_query(&json!({ "account": "Alice" }))
        };
        assert_eq!(report_of(&store, &query).total, total, "days {days:?}");
    }
}

/// Accuracy averages the reviewed games that have a figure, and mistakes are per reviewed game;
/// with no figure at all, there is no average.
#[test]
fn accuracy_averages_the_reviewed_games_that_have_a_figure() {
    let store = fresh();
    save_games(
        &store,
        vec![
            game("Reviewed1", &[("createdAt", json!(NOW - 3.0))]),
            game("Reviewed2", &[("createdAt", json!(NOW - 2.0))]),
            game(
                "Reviewed3",
                &[("createdAt", json!(NOW - 1.0)), ("color", json!("black"))],
            ),
        ],
    );
    let summaries = [
        (
            "Reviewed1",
            json!({ "white": { "accuracy": 90, "acpl": 20, "inaccuracy": 1, "mistake": 2, "blunder": 3 } }),
        ),
        (
            "Reviewed2",
            json!({ "white": { "accuracy": 70, "acpl": 40, "inaccuracy": 2, "blunder": 1 } }),
        ),
        ("Reviewed3", json!({ "black": { "inaccuracy": 1 } })),
    ];
    for (game, summary) in &summaries {
        store.exec(&format!(
            "INSERT INTO reviews (key, source, complete, depth, data, summary, updatedAt)
               VALUES ('{game}-review', 'local', 1, 18, '{{}}', '{summary}', 1);
             INSERT INTO game_reviews VALUES ('{game}', '{game}-review');"
        ));
    }
    let accuracy = report(&store, json!({ "account": "Alice" }))
        .accuracy
        .expect("accuracy");
    assert_eq!(accuracy.games, 3);
    assert_eq!(accuracy.average, Some(80.0));
    assert_eq!(accuracy.acpl, Some(30.0));
    assert_eq!(
        (
            accuracy.per_game.inaccuracy,
            accuracy.per_game.mistake,
            accuracy.per_game.blunder
        ),
        (4.0 / 3.0, 2.0 / 3.0, 4.0 / 3.0)
    );

    store.exec("DELETE FROM game_reviews WHERE gameId != 'Reviewed3'");
    let accuracy = report(&store, json!({ "account": "Alice" }))
        .accuracy
        .expect("accuracy");
    assert_eq!(accuracy.games, 1);
    assert_eq!((accuracy.average, accuracy.acpl), (None, None));
}

/// A PGN of 41 moves shuffling the knights from 200 seconds each: White's clock reads `200 - 2k`
/// after its k-th move, Black's `200 - k`, or every clock reads 200 when `idle`.
fn clocked_pgn(idle: bool) -> String {
    let shuffle = ["Nf3", "Nf6", "Ng1", "Ng8"];
    let mut pgn = String::from("[TimeControl \"200+0\"]\n\n");
    for ply in 0..82 {
        let k = ply / 2 + 1;
        let spent = if ply % 2 == 0 { 2 * k } else { k };
        let clock = if idle { 200 } else { 200 - spent };
        if ply % 2 == 0 {
            pgn.push_str(&format!("{k}. "));
        }
        pgn.push_str(&format!(
            "{} {{ [%clk 0:{:02}:{:02}] }} ",
            shuffle[ply % 4],
            clock / 60,
            clock % 60
        ));
    }
    pgn.push_str("1-0");
    pgn
}

/// Time use averages each phase's seconds per move and each move's clock over the games that
/// reached it, as a percentage of the start.
#[test]
fn time_use_averages_seconds_per_move_and_clocks_over_the_games() {
    let store = fresh();
    let pgn = clocked_pgn(false);
    save_games(
        &store,
        vec![
            game("Clocked01", &[("pgn", json!(pgn))]),
            game("Clocked02", &[("pgn", json!(pgn))]),
        ],
    );
    let time = report(&store, json!({ "account": "Alice" }))
        .time_use
        .expect("time use");
    assert_eq!((time.games, time.spare.win, time.trouble.total), (2, 2, 0));
    assert_eq!(
        time.phases.iter().map(|phase| phase.moves).sum::<i64>(),
        82,
        "41 moves in each game"
    );
    for phase in &time.phases {
        assert_eq!(phase.seconds, 2.0, "{}", phase.phase);
        let share = phase.share.expect("a share of the time");
        assert!(
            (share - 100.0 * phase.moves as f64 / 82.0).abs() < 1e-9,
            "{}: {share}",
            phase.phase
        );
    }
    let clocks: Vec<_> = time
        .clock_at
        .iter()
        .map(|at| (at.at_move, at.games, at.player, at.opponent))
        .collect();
    assert_eq!(clocks, [(20, 2, 80.0, 90.0), (40, 2, 60.0, 80.0)]);
    assert_eq!(time.blunders, None);

    // Moves that took no time at all have no share of it.
    let store = fresh();
    save_games(
        &store,
        vec![game("Instant01", &[("pgn", json!(clocked_pgn(true)))])],
    );
    let time = report(&store, json!({ "account": "Alice" }))
        .time_use
        .expect("time use");
    assert!(!time.phases.is_empty());
    for phase in &time.phases {
        assert_eq!((phase.seconds, phase.share), (0.0, None), "{}", phase.phase);
    }
}
