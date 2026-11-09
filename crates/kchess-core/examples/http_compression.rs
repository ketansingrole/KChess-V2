//! Before/after benchmark for gzip on the core's HTTP client: the same requests made with the
//! client as it was (`no_gzip`, every body uncompressed) and as it is now (gzip accepted), with
//! the bytes on the wire, the time to the headers and to the whole body, as medians.
//!
//! `cargo run --release -p kchess-core --example http_compression` — a local Lichess stand-in
//! over throttled links (bandwidth and round trip), serving answers shaped as the app asks for
//! them: the game sync's export (moves, PGN in JSON, clocks, evals, accuracy, opening), a
//! profile and a cloud evaluation. The games are random legal games, which repeat less than real
//! ones, so the savings are if anything understated. The stand-in compresses at gzip level 1,
//! nginx's default; it compresses a whole
//! answer before its headers (nginx compresses as it sends), so its headers come a few
//! milliseconds late for large answers.
//!
//! `… --example http_compression -- live [--user <name>] [--runs 5]` — the same against
//! lichess.org itself (public endpoints, no sign-in), so the result is Lichess's own compression
//! over the network the machine is on.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use shakmaty::san::San;
use shakmaty::{Chess, Position};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";

#[derive(Clone, Copy)]
struct Link {
    name: &'static str,
    /// Bytes per second.
    bandwidth: u64,
    round_trip: Duration,
}

const LINKS: [Link; 3] = [
    Link {
        name: "mobile 3 Mbit/s, 120 ms",
        bandwidth: 3_000_000 / 8,
        round_trip: Duration::from_millis(120),
    },
    Link {
        name: "home 25 Mbit/s, 40 ms",
        bandwidth: 25_000_000 / 8,
        round_trip: Duration::from_millis(40),
    },
    Link {
        name: "fast 200 Mbit/s, 15 ms",
        bandwidth: 200_000_000 / 8,
        round_trip: Duration::from_millis(15),
    },
];

#[derive(Default, Clone, Copy)]
struct Sample {
    wire_bytes: u64,
    body_bytes: u64,
    headers: Duration,
    total: Duration,
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1).cloned())
    };
    let runs: usize = value("--runs").and_then(|v| v.parse().ok()).unwrap_or(5);
    if args.first().map(String::as_str) == Some("live") {
        let user = value("--user").unwrap_or_else(|| "DrNykterstein".into());
        live(&user, runs).await;
    } else {
        local(runs).await;
    }
}

// ---- Clients: before and after ----

fn before_client(direct: bool) -> reqwest::Client {
    let builder = kchess_core::lichess::client::http_client_builder().no_gzip();
    if direct { builder.no_proxy() } else { builder }
        .build()
        .expect("client")
}

fn after_client(direct: bool) -> reqwest::Client {
    let builder = kchess_core::lichess::client::http_client_builder();
    if direct { builder.no_proxy() } else { builder }
        .build()
        .expect("client")
}

async fn timed(client: &reqwest::Client, url: &str) -> Sample {
    let start = Instant::now();
    let mut response = client.get(url).send().await.expect("request");
    assert!(
        response.status().is_success(),
        "{url}: {}",
        response.status()
    );
    let headers = start.elapsed();
    let mut body_bytes = 0u64;
    while let Some(chunk) = response.chunk().await.expect("body") {
        body_bytes += chunk.len() as u64;
    }
    Sample {
        wire_bytes: 0,
        body_bytes,
        headers,
        total: start.elapsed(),
    }
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn summarize(samples: &[Sample]) -> Sample {
    Sample {
        wire_bytes: samples[0].wire_bytes,
        body_bytes: samples[0].body_bytes,
        headers: median(samples.iter().map(|s| s.headers).collect()),
        total: median(samples.iter().map(|s| s.total).collect()),
    }
}

fn kib(bytes: u64) -> String {
    format!("{:.1} KiB", bytes as f64 / 1024.0)
}

fn ms(duration: Duration) -> String {
    format!("{:.0} ms", duration.as_secs_f64() * 1000.0)
}

fn report(name: &str, before: Sample, after: Sample) {
    let wire = 100.0 * (after.wire_bytes as f64 / before.wire_bytes.max(1) as f64 - 1.0);
    let time = 100.0 * (after.total.as_secs_f64() / before.total.as_secs_f64() - 1.0);
    println!(
        "| {name} | {} → {} ({wire:+.0}%) | {} → {} | {} → {} ({time:+.0}%) |",
        kib(before.wire_bytes),
        kib(after.wire_bytes),
        ms(before.headers),
        ms(after.headers),
        ms(before.total),
        ms(after.total),
    );
}

fn table_head() {
    println!("| Request | On the wire | Headers | Whole body |");
    println!("| --- | --- | --- | --- |");
}

// ---- Local: a throttled stand-in ----

/// A deterministic random number source (xorshift), so every run serves the same games.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// One NDJSON line as the sync's export returns it.
fn export_game(random: &mut Random, index: u64) -> String {
    let mut position = Chess::default();
    let mut sans = Vec::new();
    let plies = 40 + random.below(80);
    for _ in 0..plies {
        let moves = position.legal_moves();
        if moves.is_empty() {
            break;
        }
        let chosen = moves[random.below(moves.len() as u64) as usize];
        sans.push(San::from_move(&position, chosen).to_string());
        position.play_unchecked(chosen);
    }
    let mut clocks = Vec::new();
    let (mut white, mut black) = (18_003i64, 18_003i64);
    for ply in 0..sans.len() {
        let spent = random.below(600) as i64;
        let clock = if ply % 2 == 0 { &mut white } else { &mut black };
        *clock = (*clock - spent + 200).max(10);
        clocks.push(*clock);
    }
    let analysed = random.below(4) == 0;
    let evals: Vec<String> = sans
        .iter()
        .map(|_| format!("{{\"eval\":{}}}", random.below(400) as i64 - 200))
        .collect();
    let id = format!("g{index:07}");
    let (white_name, black_name) = ("Alice", format!("Opponent{}", random.below(500)));
    let (white_rating, black_rating) = (1500 + random.below(400), 1500 + random.below(400));
    let created = 1_760_000_000_000u64 + index * 600_000;
    let mut pgn = format!(
        "[Event \"Rated blitz game\"]\n[Site \"https://lichess.org/{id}\"]\n[Date \"2026.10.{:02}\"]\n[White \"{white_name}\"]\n[Black \"{black_name}\"]\n[Result \"1-0\"]\n[GameId \"{id}\"]\n[UTCDate \"2026.10.{:02}\"]\n[UTCTime \"12:{:02}:00\"]\n[WhiteElo \"{white_rating}\"]\n[BlackElo \"{black_rating}\"]\n[WhiteRatingDiff \"+6\"]\n[BlackRatingDiff \"-6\"]\n[Variant \"Standard\"]\n[TimeControl \"180+2\"]\n[ECO \"C20\"]\n[Opening \"King's Pawn Game\"]\n[Termination \"Normal\"]\n\n",
        1 + index % 28,
        1 + index % 28,
        index % 60
    );
    for (ply, san) in sans.iter().enumerate() {
        if ply % 2 == 0 {
            pgn.push_str(&format!("{}. ", ply / 2 + 1));
        }
        let centis = clocks[ply];
        pgn.push_str(&format!(
            "{san} {{ [%clk 0:{:02}:{:02}] }} ",
            centis / 6000,
            centis / 100 % 60
        ));
    }
    pgn.push_str("1-0\n\n\n");
    let analysis = if analysed {
        format!(
            ",\"analysis\":[{}],\"players\":{{\"white\":{{\"user\":{{\"name\":\"{white_name}\",\"id\":\"alice\"}},\"rating\":{white_rating},\"ratingDiff\":6,\"analysis\":{{\"inaccuracy\":2,\"mistake\":1,\"blunder\":0,\"acpl\":31,\"accuracy\":88}}}},\"black\":{{\"user\":{{\"name\":\"{black_name}\",\"id\":\"{}\"}},\"rating\":{black_rating},\"ratingDiff\":-6,\"analysis\":{{\"inaccuracy\":3,\"mistake\":2,\"blunder\":1,\"acpl\":54,\"accuracy\":76}}}}}}",
            evals.join(","),
            black_name.to_lowercase()
        )
    } else {
        format!(
            ",\"players\":{{\"white\":{{\"user\":{{\"name\":\"{white_name}\",\"id\":\"alice\"}},\"rating\":{white_rating},\"ratingDiff\":6}},\"black\":{{\"user\":{{\"name\":\"{black_name}\",\"id\":\"{}\"}},\"rating\":{black_rating},\"ratingDiff\":-6}}}}",
            black_name.to_lowercase()
        )
    };
    let clocks: Vec<String> = clocks.iter().map(i64::to_string).collect();
    format!(
        "{{\"id\":\"{id}\",\"rated\":true,\"variant\":\"standard\",\"speed\":\"blitz\",\"perf\":\"blitz\",\"createdAt\":{created},\"lastMoveAt\":{},\"status\":\"resign\",\"source\":\"pool\"{analysis},\"winner\":\"white\",\"opening\":{{\"eco\":\"C20\",\"name\":\"King's Pawn Game\",\"ply\":1}},\"moves\":\"{}\",\"clocks\":[{}],\"pgn\":{},\"clock\":{{\"initial\":180,\"increment\":2,\"totalTime\":260}}}}\n",
        created + 400_000,
        sans.join(" "),
        clocks.join(","),
        serde_json::to_string(&pgn).unwrap()
    )
}

const NDJSON: &str = "application/x-ndjson";
const JSON: &str = "application/json";

/// Each answer: its name, body, content type and path.
fn payloads() -> Vec<(&'static str, String, &'static str, &'static str)> {
    let mut random = Random(0x5eed_cafe);
    let export: String = (0..300).map(|i| export_game(&mut random, i)).collect();
    let small_export: String = (0..20)
        .map(|i| export_game(&mut random, 1000 + i))
        .collect();
    let perfs: serde_json::Map<String, serde_json::Value> = [
        "bullet",
        "blitz",
        "rapid",
        "classical",
        "correspondence",
        "chess960",
        "kingOfTheHill",
        "threeCheck",
        "antichess",
        "atomic",
        "horde",
        "racingKings",
        "crazyhouse",
        "puzzle",
        "storm",
        "racer",
        "streak",
    ]
    .iter()
    .map(|perf| {
        let stats =
            serde_json::json!({"games": 812, "rating": 1834, "rd": 45, "prog": 12, "prov": false});
        (perf.to_string(), stats)
    })
    .collect();
    let profile = serde_json::json!({
        "id": "alice", "username": "Alice", "createdAt": 1_600_000_000_000u64,
        "seenAt": 1_760_000_000_000u64, "playTime": {"total": 3_200_000, "tv": 0},
        "url": "https://lichess.org/@/Alice",
        "count": {"all": 9120, "rated": 8700, "ai": 12, "draw": 400, "loss": 4100, "win": 4620},
        "perfs": perfs,
        "profile": {"flag": "NL", "bio": "Chess player.", "fideRating": 1800, "links": "https://example.org"},
    })
    .to_string();
    let cloud = serde_json::json!({
        "fen": START_FEN, "knodes": 85_000_000, "depth": 52,
        "pvs": [
            {"moves": "e7e5 g1f3 b8c6 f1b5 g8f6 e1g1 f6e4 f1e1 e4d6 f3e5 f8e7 b5f1 c6e5 e1e5 e8g8 d2d4", "cp": 26},
            {"moves": "c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 a7a6 c1e3 e7e5 d4b3 c8e6 f2f3 f8e7 d1d2", "cp": 31},
            {"moves": "e7e6 d2d4 d7d5 b1c3 g8f6 c1g5 f8e7 e4e5 f6d7 g5e7 d8e7 f2f4 a7a6 g1f3 c7c5 d4c5", "cp": 34},
        ]
    })
    .to_string();
    vec![
        (
            "Game sync, 300 games",
            export,
            NDJSON,
            "/api/games/user/Alice",
        ),
        (
            "Game sync, 20 games",
            small_export,
            NDJSON,
            "/api/games/user/Bob",
        ),
        ("Profile", profile, JSON, "/api/user/Alice"),
        ("Cloud evaluation", cloud, JSON, "/api/cloud-eval"),
    ]
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(1));
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

/// Serves `routes` over `link`: one round trip before the headers, then the body paced at the
/// link's bandwidth. Counts the body bytes it writes into `wire`.
async fn stand_in(
    link: Link,
    routes: Arc<Vec<(String, &'static str, String)>>,
    wire: Arc<AtomicU64>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let routes = Arc::clone(&routes);
            let wire = Arc::clone(&wire);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buffer = [0u8; 2048];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(read) = socket.read(&mut buffer).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    head.extend_from_slice(&buffer[..read]);
                }
                let head = String::from_utf8_lossy(&head).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let head = head.to_lowercase();
                let path = path.split('?').next().unwrap_or("").to_string();
                let accepts_gzip = head
                    .lines()
                    .any(|line| line.starts_with("accept-encoding:") && line.contains("gzip"));
                let Some((_, kind, body)) = routes.iter().find(|(p, _, _)| *p == path) else {
                    return;
                };
                let (body, encoding) = if accepts_gzip {
                    (gzip(body.as_bytes()), "Content-Encoding: gzip\r\n")
                } else {
                    (body.as_bytes().to_vec(), "")
                };
                tokio::time::sleep(link.round_trip).await;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{encoding}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if socket.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                let tick = Duration::from_millis(5);
                let slice = (link.bandwidth / 200).max(1) as usize;
                let started = Instant::now();
                for (at, piece) in body.chunks(slice).enumerate() {
                    if socket.write_all(piece).await.is_err() {
                        return;
                    }
                    wire.fetch_add(piece.len() as u64, Ordering::Relaxed);
                    let due = started + tick * (at as u32 + 1);
                    tokio::time::sleep_until(due.into()).await;
                }
                let _ = socket.shutdown().await;
            });
        }
    });
    base
}

async fn local(runs: usize) {
    let payloads = payloads();
    let routes: Arc<Vec<(String, &'static str, String)>> = Arc::new(
        payloads
            .iter()
            .map(|(_, body, kind, path)| (path.to_string(), *kind, body.clone()))
            .collect(),
    );
    println!("Local stand-in, gzip level 1, median of {runs} runs.\n");
    for link in LINKS {
        println!("### {}\n", link.name);
        table_head();
        let wire = Arc::new(AtomicU64::new(0));
        let base = stand_in(link, Arc::clone(&routes), Arc::clone(&wire)).await;
        let (before_http, after_http) = (before_client(true), after_client(true));
        for (name, _, _, path) in &payloads {
            let url = format!("{base}{path}");
            let (mut before, mut after) = (Vec::new(), Vec::new());
            for run in 0..runs {
                // Alternate the order, so a change in load falls on both.
                for first in [run % 2 == 0, run % 2 != 0] {
                    let client = if first { &before_http } else { &after_http };
                    wire.store(0, Ordering::Relaxed);
                    let mut sample = timed(client, &url).await;
                    sample.wire_bytes = wire.load(Ordering::Relaxed);
                    if first { &mut before } else { &mut after }.push(sample);
                }
            }
            let (before, after) = (summarize(&before), summarize(&after));
            assert_eq!(before.body_bytes, after.body_bytes, "decoded bodies differ");
            report(name, before, after);
        }
        println!();
    }
}

// ---- Live: lichess.org itself ----

/// The bytes Lichess sends for `url` when gzip is accepted, counted before decoding.
async fn wire_size(url: &str, gzip: bool) -> (u64, Option<String>) {
    let client = before_client(false);
    let mut request = client.get(url);
    if gzip {
        request = request.header(reqwest::header::ACCEPT_ENCODING, "gzip");
    }
    let mut response = request.send().await.expect("request");
    let encoding = response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut bytes = 0;
    while let Some(chunk) = response.chunk().await.expect("body") {
        bytes += chunk.len() as u64;
    }
    (bytes, encoding)
}

async fn live(user: &str, runs: usize) {
    let flags = "moves=true&pgnInJson=true&clocks=true&evals=true&accuracy=true&opening=true";
    let fen = START_FEN.replace(' ', "%20");
    let requests = [
        (
            "Game sync, 100 games",
            format!("https://lichess.org/api/games/user/{user}?max=100&{flags}"),
        ),
        (
            "Game sync, 20 games",
            format!("https://lichess.org/api/games/user/{user}?max=20&{flags}"),
        ),
        ("Profile", format!("https://lichess.org/api/user/{user}")),
        (
            "Cloud evaluation",
            format!("https://lichess.org/api/cloud-eval?fen={fen}&multiPv=3"),
        ),
    ];
    println!("lichess.org, median of {runs} runs.\n");
    table_head();
    let (before_http, after_http) = (before_client(false), after_client(false));
    for (name, url) in &requests {
        let (plain, _) = wire_size(url, false).await;
        let (packed, encoding) = wire_size(url, true).await;
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for run in 0..runs {
            for first in [run % 2 == 0, run % 2 != 0] {
                let client = if first { &before_http } else { &after_http };
                let sample = timed(client, url).await;
                if first { &mut before } else { &mut after }.push(sample);
                // Lichess asks API clients to make one request at a time.
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
        }
        let mut before = summarize(&before);
        let mut after = summarize(&after);
        before.wire_bytes = plain;
        after.wire_bytes = packed;
        report(name, before, after);
        if encoding.as_deref() != Some("gzip") {
            println!("|   ↳ Lichess sent this one uncompressed | | | |");
        }
    }
}
