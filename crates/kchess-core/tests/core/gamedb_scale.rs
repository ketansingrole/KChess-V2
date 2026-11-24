//! How the game database holds up at scale: import time, file size, tree lookups and searches on
//! a real file. Ignored by default; set `KCHESS_GAMEDB_SCALE` to a `.pgn` or `.pgn.zst` file and
//! run `cargo test --release -p kchess-core --test core gamedb_scale -- --ignored --nocapture`.
//! Monthly `.pgn.zst` files concatenate into one: zstd frames follow each other.

use std::time::{Duration, Instant};

use kchess_contracts::types::{DatabaseSource, GameDbQuery, GameDbSort};
use kchess_core::Config;
use kchess_core::gamedb::{GameDbService, schema};
use rusqlite::OptionalExtension as _;

use crate::support;

fn percentiles(mut samples: Vec<Duration>) -> String {
    samples.sort();
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p) as usize];
    format!(
        "p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms ({} samples)",
        at(0.5).as_secs_f64() * 1e3,
        at(0.95).as_secs_f64() * 1e3,
        at(1.0).as_secs_f64() * 1e3,
        samples.len()
    )
}

/// SplitMix64, so a run's samples can be repeated.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a large PGN file: KCHESS_GAMEDB_SCALE"]
async fn gamedb_scale() {
    let Some(file) = kchess_host::env::var("KCHESS_GAMEDB_SCALE") else {
        panic!("set KCHESS_GAMEDB_SCALE to a .pgn or .pgn.zst file");
    };
    let profile = tempfile::tempdir_in(
        kchess_host::env::var("KCHESS_GAMEDB_SCALE_DIR").unwrap_or_else(|| ".".into()),
    )
    .expect("profile");
    let host = support::host();
    let games = GameDbService::new(&Config::new(profile.path().to_path_buf()), host.clone());
    let started = Instant::now();
    // The process's memory every few seconds beside the import's phase, to see what grows. On
    // macOS the footprint (what the process holds, without memory freed but kept by malloc for
    // reuse, which the resident size still counts) comes from `footprint`.
    let sampler = {
        let host = host.clone();
        tokio::spawn(async move {
            let pid = std::process::id().to_string();
            let run = async |program: &str, args: &[&str]| {
                tokio::process::Command::new(program)
                    .args(args)
                    .output()
                    .await
                    .ok()
                    .and_then(|out| String::from_utf8(out.stdout).ok())
                    .unwrap_or_default()
            };
            let (mut peak_rss, mut peak_footprint) = (0f64, 0f64);
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let rss = run("ps", &["-o", "rss=", "-p", &pid])
                    .await
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    / 1024.0;
                // "zsh [91370]: 64-bit    Footprint: 1920 KB (16384 bytes per page)"
                let footprint = run("footprint", &[&pid])
                    .await
                    .split("Footprint: ")
                    .nth(1)
                    .and_then(|rest| {
                        let mut words = rest.split_whitespace();
                        let value: f64 = words.next()?.parse().ok()?;
                        Some(match words.next()? {
                            "KB" => value / 1024.0,
                            "GB" => value * 1024.0,
                            _ => value,
                        })
                    })
                    .unwrap_or(0.0);
                peak_rss = peak_rss.max(rss);
                peak_footprint = peak_footprint.max(footprint);
                let last = host.named("gamedb:progress").pop().unwrap_or_default();
                println!(
                    "memory {:>5.0} s: footprint {footprint:>6.0} MB (peak {peak_footprint:>6.0}), \
                     resident {rss:>6.0} MB (peak {peak_rss:>6.0}) {} {} games",
                    started.elapsed().as_secs_f64(),
                    last["phase"].as_str().unwrap_or("?"),
                    last["games"]
                );
            }
        })
    };
    let imported = games
        .import(
            file.clone(),
            file.ends_with(".zst"),
            None,
            Some("Scale".into()),
            DatabaseSource::Imported,
        )
        .await
        .expect("imports");
    let took = started.elapsed();
    sampler.abort();
    let info = &imported.database;
    println!(
        "import: {} games kept, {} skipped in {:.1} s ({:.0} games/s)",
        imported.added,
        imported.skipped,
        took.as_secs_f64(),
        imported.added as f64 / took.as_secs_f64()
    );
    let path = profile
        .path()
        .join("databases")
        .join(format!("{}.kdb", info.id));
    let db = schema::open_read(&path).expect("opens");
    let pages: i64 = db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .expect("pages");
    let page: i64 = db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .expect("page size");
    println!(
        "file: {:.0} MB ({:.0} bytes a game)",
        (pages * page) as f64 / 1e6,
        (pages * page) as f64 / imported.added.max(1) as f64
    );
    let tree_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM tree", [], |r| r.get(0))
        .expect("rows");
    println!(
        "tree: {tree_rows} rows ({:.1} a game)",
        tree_rows as f64 / imported.added.max(1) as f64
    );
    // How many positions' moves only one game played, and how many count no game (unfinished).
    let by_games = |condition: &str| -> i64 {
        db.query_row(
            &format!("SELECT COUNT(*) FROM tree WHERE {condition}"),
            [],
            |r| r.get(0),
        )
        .unwrap_or(-1)
    };
    let lone = by_games("white + draws + black = 1");
    let none = by_games("white + draws + black = 0");
    let lone_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM lone", [], |r| r.get(0))
        .unwrap_or(0);
    println!(
        "tree rows of one game: {lone} ({:.1}%), of none: {none}, of more: {}; lone rows: {lone_rows}",
        100.0 * lone as f64 / tree_rows.max(1) as f64,
        tree_rows - lone - none
    );
    for table in ["games", "tree", "lone", "players", "events"] {
        let bytes: Option<i64> = db
            .query_row(
                "SELECT SUM(pgsize) FROM dbstat WHERE name = ?1",
                [table],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten();
        if let Some(bytes) = bytes {
            println!("  {table}: {:.0} MB", bytes as f64 / 1e6);
        }
    }
    let mut seed = 7;
    // Tree lookups: positions drawn from the tree, each read as the explorer reads one.
    let mut keys = Vec::new();
    for _ in 0..2_000 {
        let probe = next(&mut seed) as i64;
        let key: Option<i64> = db
            .query_row(
                "SELECT key FROM tree WHERE key >= ?1 LIMIT 1",
                [probe],
                |r| r.get(0),
            )
            .optional()
            .expect("probe");
        if let Some(key) = key {
            keys.push(key as u64);
        }
    }
    let start = kchess_domain::gamedb::position_key(&kchess_domain::rules::default_position(
        shakmaty::variant::Variant::Chess,
    ));
    keys.insert(0, start);
    let mut samples = Vec::new();
    for key in &keys {
        let begun = Instant::now();
        games.tree(&info.id, *key).await.expect("tree");
        samples.push(begun.elapsed());
    }
    println!("tree lookup: {}", percentiles(samples));
    // Searches: by a player drawn from the names, by event words, by rating, and every game.
    let players: i64 = db
        .query_row("SELECT MAX(id) FROM players", [], |r| r.get(0))
        .expect("players");
    let query = |q: GameDbQuery| GameDbQuery {
        database: info.id.clone(),
        offset: 0,
        limit: 20,
        ..q
    };
    let mut by_player = Vec::new();
    for _ in 0..200 {
        let id = 1 + (next(&mut seed) % players.max(1) as u64) as i64;
        let name: Option<String> = db
            .query_row("SELECT norm FROM players WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .expect("name");
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            continue;
        };
        let surname = name.split(' ').next().unwrap_or(&name).to_string();
        let begun = Instant::now();
        games
            .games(query(GameDbQuery {
                player: Some(surname),
                ..GameDbQuery::default()
            }))
            .await
            .expect("search");
        by_player.push(begun.elapsed());
    }
    println!("search by player: {}", percentiles(by_player));
    let fixed = [
        ("every game, newest", GameDbQuery::default()),
        (
            "every game, by rating",
            GameDbQuery {
                sort: Some(GameDbSort::Rating),
                ..GameDbQuery::default()
            },
        ),
        (
            "both 2600+",
            GameDbQuery {
                min_elo: Some(2600),
                ..GameDbQuery::default()
            },
        ),
        (
            "event \"olympiad\"",
            GameDbQuery {
                event: Some("olympiad".into()),
                ..GameDbQuery::default()
            },
        ),
        (
            "player \"carlsen\" 2020-2024",
            GameDbQuery {
                player: Some("carlsen".into()),
                year_from: Some(2020),
                year_to: Some(2024),
                ..GameDbQuery::default()
            },
        ),
    ];
    for (label, q) in fixed {
        let mut samples = Vec::new();
        let mut total = 0;
        for _ in 0..5 {
            let begun = Instant::now();
            total = games.games(query(q.clone())).await.expect("search").total;
            samples.push(begun.elapsed());
        }
        println!("search {label} ({total} games): {}", percentiles(samples));
    }
    games.close().await;
}

/// The start of a `.pgn.zst` file's text, to see what its games hold.
#[test]
#[ignore = "needs a .pgn.zst file: KCHESS_GAMEDB_SAMPLE"]
fn gamedb_sample() {
    use std::io::Read as _;
    let Some(file) = kchess_host::env::var("KCHESS_GAMEDB_SAMPLE") else {
        panic!("set KCHESS_GAMEDB_SAMPLE to a .pgn.zst file");
    };
    let bytes = std::fs::read(file).expect("reads");
    // Lichess's files start with a skippable frame (magic 0x184D2A5?, a length, its payload).
    let mut at = 0;
    while bytes.len() >= at + 8
        && u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) & 0xFFFF_FFF0
            == 0x184D_2A50
    {
        let length = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("four bytes"));
        at += 8 + length as usize;
    }
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(&bytes[at..]).expect("a zstd frame");
    let mut text = vec![0; 6_000];
    decoder.read_exact(&mut text).expect("reads");
    println!("{}", String::from_utf8_lossy(&text));
}
