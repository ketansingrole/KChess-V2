//! Time the stages of library decoding on benchmark fixtures:
//! `cargo run --release -p kchess-domain --example profile -- <fixtures dir>`
//! (`pnpm run benchmark:core` writes fixtures to test-results/benchmark-fixtures).

use std::time::Instant;

use kchess_domain::{library, pgn, puzzle, replay, review, tree};
use serde_json::Value;

fn time<T>(label: &str, runs: u32, mut f: impl FnMut() -> T) {
    f();
    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(f());
    }
    println!(
        "{label:<28} {:>9.3} ms",
        start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs)
    );
}

fn main() {
    let dir = std::env::args().nth(1).expect("fixtures directory");
    let studies = std::fs::read_to_string(format!("{dir}/studies.json")).expect("studies.json");
    let doc: Value = serde_json::from_str(&studies).expect("json");
    let pgns: Vec<String> = doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .flat_map(|s| s["chapters"].as_array().expect("chapters").iter())
        .map(|c| c["pgn"].as_str().expect("pgn").to_string())
        .collect();
    time("serde parse studies", 20, || {
        serde_json::from_str::<Value>(&studies).map(|_| ()).ok()
    });
    time("parse_pgn x400", 20, || {
        pgns.iter().map(|p| pgn::parse_pgn(p).len()).sum::<usize>()
    });
    time("valid_pgn x400", 20, || {
        pgns.iter().filter(|p| tree::valid_pgn(p)).count()
    });
    time("tree_from_pgn x400", 20, || {
        pgns.iter().filter_map(|p| tree::tree_from_pgn(p)).count()
    });
    time("decode_studies", 20, || {
        library::decode_studies(&doc).map(|s| s.len())
    });
    time("decode_studies + to_string", 20, || {
        library::decode_studies(&doc).map(|s| serde_json::to_string(&s).map(|t| t.len()))
    });

    let games: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{dir}/games.json")).expect("games.json"),
    )
    .expect("json");
    let rows: Vec<String> = games
        .as_array()
        .expect("games")
        .iter()
        .map(Value::to_string)
        .collect();
    time("serde parse 500 game rows", 20, || {
        rows.iter()
            .filter_map(|r| serde_json::from_str::<Value>(r).ok())
            .count()
    });
    time("replay_setup_count x500", 20, || {
        games
            .as_array()
            .expect("games")
            .iter()
            .filter_map(|g| {
                let moves: Vec<&str> = g["moves"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                replay::replay_setup_count(
                    g["setup"]["variant"].as_str()?,
                    g["setup"]["fen"].as_str()?,
                    &moves,
                )
            })
            .sum::<usize>()
    });
    time("decode_archived_game x500", 20, || {
        rows.iter()
            .filter_map(|r| serde_json::from_str::<Value>(r).ok())
            .filter_map(|v| library::decode_archived_game(&v))
            .map(|g| serde_json::to_string(&g).map(|s| s.len()).unwrap_or(0))
            .sum::<usize>()
    });

    let csv = std::fs::read(format!("{dir}/puzzles.csv")).expect("puzzles.csv");
    time("puzzle sampler push", 5, || {
        let mut sampler = puzzle::Sampler::new(puzzle::Random::new(1));
        for chunk in csv.chunks(65_536) {
            sampler.push(chunk).expect("csv");
        }
        sampler.finish().expect("csv");
        sampler.lines
    });
    time("puzzle sampler + kept json", 5, || {
        let mut sampler = puzzle::Sampler::new(puzzle::Random::new(1));
        for chunk in csv.chunks(65_536) {
            sampler.push(chunk).expect("csv");
        }
        sampler.finish().expect("csv");
        serde_json::to_string(&sampler.kept())
            .map(|s| s.len())
            .unwrap_or(0)
    });
    let reviews: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(format!("{dir}/reviews.json")).expect("reviews.json"),
    )
    .expect("json");
    let review_texts: Vec<String> = reviews.iter().map(Value::to_string).collect();
    time("review parse x300", 20, || {
        review_texts
            .iter()
            .filter_map(|r| serde_json::from_str::<Value>(r).ok())
            .count()
    });
    time("replay_positions x300", 20, || {
        reviews
            .iter()
            .map(|r| {
                let moves: Vec<&str> = r["moves"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                replay::replay_positions(r["fen"].as_str().unwrap(), &moves).len()
            })
            .sum::<usize>()
    });
    time("summarize x300", 20, || {
        reviews.iter().filter_map(review::summarize).count()
    });
    let synced: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(format!("{dir}/synced-studies.json")).expect("synced"),
    )
    .expect("json");
    let downloaded: Vec<String> = synced
        .iter()
        .map(|s| s["cloud"]["downloadedPgn"].as_str().unwrap().to_string())
        .collect();
    time("parse_pgn downloaded x50", 20, || {
        downloaded
            .iter()
            .map(|p| pgn::parse_pgn(p).len())
            .sum::<usize>()
    });
    time("study_content x50", 20, || {
        downloaded
            .iter()
            .map(|p| pgn::study_content(p).len())
            .sum::<usize>()
    });
}
