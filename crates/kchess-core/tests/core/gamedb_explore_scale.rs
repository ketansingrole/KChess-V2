//! How fast the explorer looks a position up in a large game database, with and without filters.
//! Ignored by default. `KCHESS_GAMEDB_PROFILE` names a profile folder kept between runs: the
//! first run imports `KCHESS_GAMEDB_SCALE` (a `.pgn` or `.pgn.zst` file) into it, later runs
//! reuse that database. Run
//! `cargo test --release -p kchess-core --test core gamedb_explore_scale -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use kchess_contracts::types::{DatabaseSource, LookupOptions};
use kchess_core::Config;
use kchess_core::gamedb::{GameDbService, schema};

use crate::support;

fn millis(took: Duration) -> String {
    format!("{:.0} ms", took.as_secs_f64() * 1e3)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a profile folder: KCHESS_GAMEDB_PROFILE"]
async fn gamedb_explore_scale() {
    let Some(profile) = kchess_host::env::var("KCHESS_GAMEDB_PROFILE") else {
        panic!("set KCHESS_GAMEDB_PROFILE to a folder kept between runs");
    };
    let profile = std::path::PathBuf::from(profile);
    let games = GameDbService::new(&Config::new(profile.clone()), support::host());
    let mut status = games.status().await.expect("status");
    if status.databases.is_empty() {
        let Some(file) = kchess_host::env::var("KCHESS_GAMEDB_SCALE") else {
            panic!("set KCHESS_GAMEDB_SCALE to a .pgn or .pgn.zst file for the first run");
        };
        let started = Instant::now();
        games
            .import(
                file.clone(),
                file.ends_with(".zst"),
                None,
                Some("Scale".into()),
                DatabaseSource::Imported,
            )
            .await
            .expect("imports");
        println!("import: {:.0} s", started.elapsed().as_secs_f64());
        status = games.status().await.expect("status");
    }
    let info = status.databases[0].clone();
    println!("database: {} games", info.games);

    // Reading the matching games' rows alone, without replaying them: what SQLite costs.
    let db = schema::open_read(&profile.join("databases").join(format!("{}.kdb", info.id)))
        .expect("opens");
    for (label, condition) in [
        ("both 2700+", "low_elo >= 2700"),
        ("both 2400+", "low_elo >= 2400"),
        ("since 2025", "year >= 2025"),
        ("every game", "1"),
    ] {
        let begun = Instant::now();
        let mut select = db
            .prepare(&format!(
                "SELECT id, fen, moves, result, rating, date_key FROM games
                 WHERE {condition} AND result IS NOT NULL"
            ))
            .expect("prepares");
        let mut rows = select.query([]).expect("query");
        let (mut count, mut bytes) = (0u64, 0usize);
        while let Some(row) = rows.next().expect("row") {
            let moves: Vec<u8> = row.get(2).expect("moves");
            bytes += moves.len();
            count += 1;
        }
        println!(
            "read {label}: {count} rows, {:.0} MB of moves in {}",
            bytes as f64 / 1e6,
            millis(begun.elapsed())
        );
    }

    // The first lookup opens the file, which upgrades one an older release wrote.
    let started = Instant::now();
    games
        .lookup(
            kchess_domain::rules::make_fen(&kchess_domain::rules::default_position(
                shakmaty::variant::Variant::Chess,
            )),
            LookupOptions {
                database: Some(info.id.clone()),
                ..LookupOptions::default()
            },
        )
        .await
        .expect("first lookup");
    println!(
        "first lookup (opens, upgrades): {}",
        millis(started.elapsed())
    );
    for (label, player, event) in [
        ("player carlsen", Some("carlsen"), None),
        ("player \"magnus c\"", Some("magnus c"), None),
        ("event olympiad", None, Some("olympiad")),
    ] {
        let mut best = Duration::MAX;
        let mut total = 0;
        for _ in 0..3 {
            let begun = Instant::now();
            total = games
                .games(kchess_contracts::types::GameDbQuery {
                    database: info.id.clone(),
                    player: player.map(str::to_string),
                    event: event.map(str::to_string),
                    limit: 20,
                    ..Default::default()
                })
                .await
                .expect("search")
                .total;
            best = best.min(begun.elapsed());
        }
        println!("search {label} ({total} games): {}", millis(best));
    }
    let lines = [
        ("start", ""),
        ("1.e4 c5", "e4 c5"),
        ("Najdorf", "e4 c5 Nf3 d6 d4 cxd4 Nxd4 Nf6 Nc3 a6"),
        (
            "Berlin endgame",
            "e4 e5 Nf3 Nc6 Bb5 Nf6 O-O Nxe4 d4 Nd6 Bxc6 dxc6 dxe5 Nf5 Qxd8+ Kxd8",
        ),
    ];
    let filter = |player: Option<&str>, color, min_elo, since: Option<&str>| LookupOptions {
        database: Some(info.id.clone()),
        player: player.map(str::to_string),
        color,
        min_elo,
        since: since.map(str::to_string),
        ..LookupOptions::default()
    };
    let filters = [
        ("no filter", filter(None, None, None, None)),
        ("carlsen", filter(Some("carlsen"), None, None, None)),
        (
            "carlsen as white",
            filter(Some("carlsen"), Some(shakmaty::Color::White), None, None),
        ),
        ("both 2700+", filter(None, None, Some(2700), None)),
        ("both 2400+", filter(None, None, Some(2400), None)),
        ("both 2000+", filter(None, None, Some(2000), None)),
        ("since 2025", filter(None, None, None, Some("2025"))),
        (
            "since 1900 (every dated game)",
            filter(None, None, None, Some("1900")),
        ),
    ];
    for (line, sans) in lines {
        let mut pos = kchess_domain::rules::default_position(shakmaty::variant::Variant::Chess);
        for san in sans.split_whitespace() {
            let m = kchess_domain::rules::parse_san(&pos, san).expect("legal line");
            shakmaty::Position::play_unchecked(&mut pos, m);
        }
        let fen = kchess_domain::rules::make_fen(&pos);
        for (label, options) in &filters {
            let mut best = Duration::MAX;
            let mut answer = String::new();
            for _ in 0..3 {
                let begun = Instant::now();
                answer = match games.lookup(fen.clone(), options.clone()).await {
                    Ok(found) => format!(
                        "{} games, {} moves",
                        found.saved.total.unwrap_or(0),
                        found.saved.moves.len()
                    ),
                    Err(error) => error.message,
                };
                best = best.min(begun.elapsed());
            }
            println!("lookup {line}, {label} ({answer}): {}", millis(best));
        }
    }
    games.close().await;
}
