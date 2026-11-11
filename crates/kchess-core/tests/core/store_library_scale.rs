//! The game library at its largest: ten followed players at the retention cap (`MAX_GAMES` each,
//! 50,000 games) with reviews. Every read a page makes of it answers with a bounded response;
//! `library_scale_probe` (ignored) prints each read's time and size.

use std::time::{Duration, Instant};

use crate::support;

use kchess_core::store::{
    games::{self, MAX_GAMES},
    insights, library, reviews, usage,
};
use serde_json::{Value, json};
use support::store::{Store, profile};

const ACCOUNTS: [&str; 10] = [
    "Alice", "Bob", "Carol", "Dave", "Erin", "Frank", "Grace", "Heidi", "Ivan", "Judy",
];
const START: i64 = 1_700_000_000_000;
/// One game every ten minutes, interleaved across the players.
const STRIDE: i64 = 600_000;
const REVIEWED: i64 = 2_000;
/// Enough games for the query plans and the storage cache. The full library's database peaks at
/// about 430 MB of temporary disk, too much to build once per test while other tests run.
const SMALL: i64 = 50;

/// A profile with `per_account` finished games for each of `ACCOUNTS` (the first two connected),
/// realistic move and PGN lengths, and `REVIEWED` complete reviews of Alice's newest games.
fn seeded(per_account: i64) -> Store {
    let store = profile();
    // The one-time import runs before the seed, so a later call does not import over it.
    store
        .read(|c| games::load_app_data(c.db))
        .expect("loadData");
    let accounts = ACCOUNTS
        .iter()
        .enumerate()
        .map(|(index, name)| format!("('{name}', {})", i64::from(index < 2)))
        .collect::<Vec<_>>()
        .join(", ");
    let names = ACCOUNTS
        .iter()
        .enumerate()
        .map(|(index, name)| format!("SELECT {index} AS n, '{name}' AS name"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let accounts_n = ACCOUNTS.len();
    // 80 plies of moves and a PGN with headers and clock comments, as Lichess exports them.
    let moves = ["e4 e5 Nf3 Nc6 Bb5 a6 Ba4 Nf6"; 10].join(" ");
    let pgn = format!(
        "[Event \"Rated blitz game\"]\n[Site \"https://lichess.org/abcdefgh\"]\n[White \"a\"]\n[Black \"b\"]\n\n{}",
        ["1. e4 { [%clk 0:03:00] } e5 { [%clk 0:03:00] }"; 40].join(" ")
    );
    store.exec(&format!(
        "INSERT INTO accounts (username, connected) VALUES {accounts};
         WITH RECURSIVE seq(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM seq WHERE i < {total} - 1),
           names AS ({names})
         INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner,
             color, opponent, opponentRating, playerRating, ratingDiff, opening, moves, pgn)
         SELECT names.name, printf('g%07d', i), {START} + i * {STRIDE}, {START} + i * {STRIDE} + 300000,
           i % 3 != 0,
           CASE i % 4 WHEN 0 THEN 'bullet' WHEN 1 THEN 'blitz' WHEN 2 THEN 'rapid' ELSE 'classical' END,
           CASE i % 4 WHEN 0 THEN 'bullet' WHEN 1 THEN 'blitz' WHEN 2 THEN 'rapid' ELSE 'classical' END,
           CASE i % 5 WHEN 0 THEN 'draw' WHEN 1 THEN 'resign' WHEN 2 THEN 'outoftime' ELSE 'mate' END,
           CASE (i / {accounts_n}) % 5 WHEN 0 THEN NULL WHEN 1 THEN 'white' WHEN 2 THEN 'black' WHEN 3 THEN 'white' ELSE 'black' END,
           CASE (i / {accounts_n}) % 2 WHEN 0 THEN 'white' ELSE 'black' END,
           printf('opponent%d', i % 700), 1400 + i % 400, 1500 + i % 300, (i % 17) - 8,
           printf('Opening %d: Variation %d', i % 60, i % 7), '{moves}', '{pgn}'
         FROM seq JOIN names ON names.n = i % {accounts_n};
         WITH RECURSIVE seq(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM seq WHERE i < {reviewed} - 1)
         INSERT INTO reviews (key, gameId, source, complete, depth, data, summary, updatedAt)
         SELECT printf('review%07d', i), printf('g%07d', ({total} - 1 - i * {accounts_n}) / {accounts_n} * {accounts_n}),
           'local', 1, 18, '{{}}',
           '{{\"white\":{{\"accuracy\":81,\"acpl\":32,\"inaccuracy\":2,\"mistake\":1,\"blunder\":0}},\"black\":{{\"accuracy\":77,\"acpl\":41,\"inaccuracy\":3,\"mistake\":1,\"blunder\":1}}}}',
           {START} + i
         FROM seq;
         INSERT INTO game_reviews (gameId, reviewKey) SELECT gameId, key FROM reviews;",
        total = per_account * ACCOUNTS.len() as i64,
        reviewed = REVIEWED.min(per_account),
    ));
    store
}

/// A read straight to the database (the app's import has already run).
type Read = Box<dyn Fn(&Store) -> Value>;

/// The reads the app's pages make of a large library, as (label, read).
fn reads() -> Vec<(&'static str, Read)> {
    let page = |query: Value| -> Read {
        Box::new(move |store: &Store| {
            store
                .read_direct(|c| games::game_page(c, query.clone()))
                .expect("gamePage")
        })
    };
    vec![
        (
            "loadData",
            Box::new(|store: &Store| {
                store
                    .read_direct(|c| games::load_app_data(c.db))
                    .expect("loadData")
            }),
        ),
        ("gamePage all", page(json!({ "offset": 0, "limit": 50 }))),
        (
            "gamePage all, deep",
            page(json!({ "offset": 40_000, "limit": 50 })),
        ),
        (
            "gamePage account+result",
            page(json!({ "account": "alice", "result": "loss", "offset": 0, "limit": 50 })),
        ),
        (
            "gamePage rated",
            page(json!({ "rated": true, "offset": 0, "limit": 50 })),
        ),
        (
            "gamePage search",
            page(json!({ "search": "opponent42", "offset": 0, "limit": 50 })),
        ),
        (
            "gameLibraryOverview",
            Box::new(|store: &Store| {
                store
                    .read_direct(games::game_library_overview)
                    .expect("gameLibraryOverview")
            }),
        ),
        (
            "gameRatingHistory",
            Box::new(|store: &Store| {
                store
                    .read_direct(|c| games::rating_history(c.db, "Alice"))
                    .expect("gameRatingHistory")
            }),
        ),
        (
            "insights",
            Box::new(|store: &Store| {
                let query = json!({ "account": "Alice" });
                store
                    .read_direct(|c| insights::insights(c.host, c.db, &query, library::now_ms()?))
                    .expect("insights")
            }),
        ),
        (
            "gamesToReview",
            Box::new(|store: &Store| {
                let accounts = ["Alice".to_string(), "Bob".to_string()];
                store
                    .read_direct(|c| reviews::games_to_review(c.db, &accounts, 0.0, 300.0))
                    .expect("gamesToReview")
            }),
        ),
        (
            "usageReport",
            Box::new(|store: &Store| store.read_direct(usage::usage_report).expect("usageReport")),
        ),
    ]
}

/// The median of `runs` timed reads, the first (cold) read, and the response's JSON size.
fn measure(store: &Store, read: &Read, runs: usize) -> (Duration, Duration, usize) {
    let start = Instant::now();
    let value = read(store);
    let cold = start.elapsed();
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            read(store);
            start.elapsed()
        })
        .collect();
    times.sort();
    let size = serde_json::to_string(&value).map_or(0, |text| text.len());
    (cold, times[times.len() / 2], size)
}

/// `EXPLAIN QUERY PLAN` of `sql`, one detail line per step.
fn plan(store: &Store, sql: &str) -> Vec<String> {
    store
        .rows(&format!("EXPLAIN QUERY PLAN {sql}"))
        .iter()
        .map(|row| row["detail"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// A count straight from the file, to check the store's answers against.
fn count(store: &Store, sql: &str) -> i64 {
    store.rows(sql)[0]["n"].as_i64().expect("a count")
}

#[test]
fn a_full_library_answers_every_read_with_a_bounded_response() {
    let store = seeded(MAX_GAMES);
    let total = MAX_GAMES * ACCOUNTS.len() as i64;
    assert_eq!(
        store
            .read_direct(|c| games::load_app_data(c.db))
            .expect("loadData")["gameCount"],
        json!(total)
    );
    // What the app's pages transfer at 50,000 games, never the library itself.
    let budgets: [(&str, usize); 11] = [
        ("loadData", 4 * 1024),
        ("gamePage all", 64 * 1024),
        ("gamePage all, deep", 64 * 1024),
        ("gamePage account+result", 64 * 1024),
        ("gamePage rated", 64 * 1024),
        ("gamePage search", 64 * 1024),
        ("gameLibraryOverview", 4 * 1024),
        ("gameRatingHistory", 32 * 1024),
        ("insights", 16 * 1024),
        // A review pass reads its 300 candidates with their PGN: bounded by its limit.
        ("gamesToReview", 1024 * 1024),
        ("usageReport", 4 * 1024),
    ];
    for (label, read) in reads() {
        let value = read(&store);
        let size = serde_json::to_string(&value).expect("json").len();
        let budget = budgets
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, budget)| *budget)
            .unwrap_or_else(|| panic!("{label} has no budget"));
        assert!(size <= budget, "{label}: {size} bytes over {budget}");
        if label.starts_with("gamePage") {
            assert_eq!(value["games"].as_array().map(Vec::len), Some(50), "{label}");
        }
    }

    // The totals are the filters' own counts over the whole library.
    let total_of = |query: Value| {
        store
            .read_direct(|c| games::game_page(c, query))
            .expect("gamePage")["total"]
            .clone()
    };
    assert_eq!(total_of(json!({ "offset": 0, "limit": 1 })), json!(total));
    assert_eq!(
        total_of(json!({ "rated": true, "offset": 0, "limit": 1 })),
        json!(count(
            &store,
            "SELECT COUNT(*) AS n FROM games WHERE rated = 1"
        ))
    );
    assert_eq!(
        total_of(json!({ "search": "opponent42", "offset": 0, "limit": 1 })),
        json!(count(
            &store,
            "SELECT COUNT(*) AS n FROM games WHERE opponent LIKE '%opponent42%'"
        ))
    );
    assert_eq!(
        total_of(
            json!({ "account": "carol", "result": "win", "rated": false, "offset": 0, "limit": 1 })
        ),
        json!(count(
            &store,
            "SELECT COUNT(*) AS n FROM games WHERE account = 'Carol' AND rated = 0 AND winner = color"
        ))
    );
    let last = store
        .read_direct(|c| games::game_page(c, json!({ "offset": total - 7, "limit": 50 })))
        .expect("gamePage");
    assert_eq!(last["games"].as_array().map(Vec::len), Some(7));

    let overview = store
        .read_direct(games::game_library_overview)
        .expect("gameLibraryOverview");
    let records = overview["byAccount"]
        .as_object()
        .expect("records by player");
    assert_eq!(records.len(), ACCOUNTS.len());
    for name in ACCOUNTS {
        let record = &records[&name.to_lowercase()];
        assert_eq!(record["total"], json!(MAX_GAMES), "{name}");
        let parts = ["win", "loss", "draw"].map(|key| record[key].as_i64().unwrap_or_default());
        assert_eq!(parts.iter().sum::<i64>(), MAX_GAMES, "{name}");
    }

    let usage = store.read_direct(usage::usage_report).expect("usageReport");
    for name in ACCOUNTS {
        assert_eq!(
            usage["storage"][name.to_lowercase()]["games"],
            json!(MAX_GAMES)
        );
    }
}

#[test]
fn the_library_aggregates_read_the_summary_index_not_every_game() {
    let store = seeded(SMALL);
    // Each of these read every game's row, moves and PGN with it, before the summary index: cold
    // at 50,000 games a rated total took 82 ms, a search's 83 ms and the overview 112–170 ms;
    // covered by the index they took 5, 23 and 8 ms.
    for sql in [
        "SELECT COUNT(*) FROM games WHERE rated = 1",
        r"SELECT COUNT(*) FROM games
          WHERE (opponent LIKE '%a%' ESCAPE '\' OR COALESCE(opening, '') LIKE '%a%' ESCAPE '\')",
        "SELECT lower(account) AS name, COUNT(*) AS total, SUM(winner = color) AS win,
           SUM(winner IS NOT NULL AND winner != color) AS loss, SUM(winner IS NULL) AS draw
         FROM games GROUP BY account COLLATE NOCASE",
        "SELECT lower(opponent) AS name, COUNT(*) AS total, SUM(winner = color) AS win
         FROM games WHERE account IN (SELECT username FROM accounts WHERE connected = 1)
           AND opponent COLLATE NOCASE IN (SELECT username FROM accounts)
         GROUP BY lower(opponent)",
    ] {
        let steps = plan(&store, sql);
        assert!(
            steps
                .iter()
                .any(|step| step.contains("COVERING INDEX idx_games_summary")),
            "{sql}: {steps:?}"
        );
    }
}

#[test]
fn the_games_storage_is_kept_until_the_games_change() {
    let store = seeded(SMALL);
    let alice = |store: &Store| {
        store.read_direct(usage::usage_report).expect("usageReport")["storage"]["alice"].clone()
    };
    let before = alice(&store);
    assert_eq!(before["games"], json!(SMALL));
    // A second connection's write is not one the core makes: the kept sums still answer.
    store.exec("DELETE FROM games WHERE account = 'Alice' AND id = 'g0000000'");
    assert_eq!(alice(&store), before);
    // A write through the core sums them again.
    store
        .write_direct(|c| games::save_games(c.host, c.db, "Alice", &[], Some(START)))
        .expect("saveGames");
    let after = alice(&store);
    assert_eq!(after["games"], json!(SMALL - 1));
    assert!(after["bytes"].as_i64() < before["bytes"].as_i64());
}

#[test]
#[ignore = "a timing probe: cargo test --release -p kchess-core --test core library_scale_probe -- --ignored --nocapture"]
fn library_scale_probe() {
    let store = seeded(MAX_GAMES);
    eprintln!("{} games", MAX_GAMES * ACCOUNTS.len() as i64);
    for (label, read) in reads() {
        let (cold, warm, size) = measure(&store, &read, 7);
        eprintln!(
            "{label:<28} cold {:>9.3} ms  warm {:>9.3} ms  {size:>8} bytes",
            cold.as_secs_f64() * 1000.0,
            warm.as_secs_f64() * 1000.0
        );
    }
}
