//! How long the store's reads take over a full profile: the library (its maximum of played games,
//! studies and mistakes, which every frontend reads to start) and pages of synced games from the
//! newest to the oldest. Not part of `check`; it prints the figures:
//! `cargo test --release -p kchess-core --test store_speed -- --ignored --nocapture`

use kchess_contracts::CoreEvent;
use std::time::{Duration, Instant};

use kchess_core::store::{self, StoreCache, StoreContext, games, library};
use kchess_core::{Config, CoreError, Host, Level};
use rusqlite::{Connection, params};
use serde_json::json;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Position};

struct Quiet;

impl Host for Quiet {
    fn log(&self, _: Level, _: &str, _: &str) {}
    fn emit(&self, _event: CoreEvent) {}
}

/// xorshift64: the same games on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A random legal game of up to `plies` moves, in UCI.
fn random_game(rng: &mut Rng, plies: usize) -> Vec<String> {
    let mut pos = Chess::default();
    let mut moves = Vec::new();
    for _ in 0..plies {
        let legal = pos.legal_moves();
        if legal.is_empty() {
            break;
        }
        let m = legal[(rng.next() % legal.len() as u64) as usize];
        moves.push(UciMove::from_move(m, CastlingMode::Standard).to_string());
        pos.play_unchecked(m);
    }
    moves
}

fn run<T>(
    db: &Connection,
    cache: &StoreCache,
    work: impl FnOnce(&StoreContext) -> Result<T, CoreError>,
) -> T {
    let config = Config::new(std::path::PathBuf::new());
    let ctx = StoreContext {
        db,
        host: &Quiet,
        config: &config,
        cache,
    };
    work(&ctx).unwrap_or_else(|cause| panic!("{cause:?}"))
}

/// The median of `runs` timings of `f`.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let started = Instant::now();
            f();
            started.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.)
}

/// A profile with `games` played games of 80 moves (500 is the limit) and `studies` studies.
fn full_library(db: &Connection, cache: &StoreCache, games: usize, studies: usize) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for n in 0..games {
        let game = json!({
            "id": format!("game-{n}"), "source": "computer", "startedAt": n, "updatedAt": n,
            "white": "You", "black": "Stockfish", "result": "*", "reason": "Unfinished",
            "timeControl": "-", "finished": false,
            "setup": { "variant": "standard", "fen": shakmaty::fen::Fen::from_position(&Chess::default(), shakmaty::EnPassantMode::Legal).to_string() },
            "moves": random_game(&mut rng, 80),
        });
        run(db, cache, |c| library::write_game(c.db, &game));
    }
    for n in 0..studies {
        let moves = random_game(&mut rng, 60);
        let mut pos = Chess::default();
        let mut pgn = String::new();
        for (ply, uci) in moves.iter().enumerate() {
            let m = uci
                .parse::<UciMove>()
                .expect("uci")
                .to_move(&pos)
                .expect("legal");
            if ply % 2 == 0 {
                pgn.push_str(&format!("{}. ", ply / 2 + 1));
            }
            pgn.push_str(&format!("{} ", shakmaty::san::San::from_move(&pos, m)));
            pos.play_unchecked(m);
        }
        pgn.push('*');
        let command = kchess_contracts::types::StudyCommand::Save {
            name: format!("Study {n}"),
            pgn,
            id: None,
            chapter_id: None,
        };
        run(db, cache, |c| {
            library::run_study_command(c.db, command, n as f64, false, None)
        });
    }
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn library_read_over_a_full_profile() {
    for (label, games, studies) in [
        ("full", 500, 50),
        ("games only", 500, 0),
        ("studies only", 0, 50),
    ] {
        let dir = tempfile::tempdir().expect("dir");
        let db = store::open(&dir.path().join("kchess.db"), &Quiet).expect("open");
        let cache = StoreCache::default();
        full_library(&db, &cache, games, studies);
        let library = run(&db, &cache, |c| library::cached_library(c, 0.0));
        assert_eq!(library.games.len(), games);
        let bytes = serde_json::to_string(&library).expect("json").len();
        // A fresh cache each time: what a first read (or one after a write) costs.
        let first = median(15, || {
            run(&db, &StoreCache::default(), |c| {
                library::cached_library(c, 0.0)
            });
        });
        // What every later read costs, written as JSON as the service sends it.
        let again = median(15, || {
            let value = run(&db, &cache, |c| library::cached_library(c, 0.0));
            std::hint::black_box(serde_json::to_string(&value).expect("json"));
        });
        println!(
            "library, {label} ({} KiB): first read {}, read again (and as JSON) {}",
            bytes / 1024,
            ms(first),
            ms(again)
        );
    }
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn game_pages_from_newest_to_oldest() {
    let dir = tempfile::tempdir().expect("dir");
    let db = store::open(&dir.path().join("kchess.db"), &Quiet).expect("open");
    let cache = StoreCache::default();
    let total = 50_000;
    let mut rng = Rng(7);
    let moves = random_game(&mut rng, 80).join(" ");
    db.execute_batch("BEGIN").expect("begin");
    for n in 0..total {
        db.execute(
            "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status,
                                winner, color, opponent, opponentRating, playerRating, moves)
             VALUES ('Alice', ?1, ?2, ?2, 1, 'blitz', 'blitz', 'mate', ?3, 'white', ?4, 1500, 1500, ?5)",
            params![
                format!("g{n:06}"),
                1_600_000_000_000_i64 + n * 60_000,
                if n % 3 == 0 { "white" } else { "black" },
                format!("Opponent{}", n % 400),
                moves
            ],
        )
        .expect("insert");
    }
    db.execute_batch("COMMIT").expect("commit");
    let size = 50;
    for (label, account) in [("every account", None), ("one account", Some("alice"))] {
        let mut figures = Vec::new();
        for page in [0, 100, 500, 999] {
            let mut query = json!({ "offset": page * size, "limit": size });
            if let Some(account) = account {
                query["account"] = json!(account);
            }
            let time = median(15, || {
                let answer = run(&db, &cache, |c| games::game_page(c, query.clone()));
                assert_eq!(answer["games"].as_array().map(Vec::len), Some(size));
            });
            figures.push(format!("page {page}: {}", ms(time)));
        }
        println!("gamePage, {label}, {total} games: {}", figures.join(", "));
    }
}
