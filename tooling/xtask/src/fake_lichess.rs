//! A stand-in Lichess for the signed-in benchmarks (`bench`): a small HTTP/1.1 server on
//! 127.0.0.1 that signs `Bench` in through the app's own OAuth flow and plays out one scripted
//! line, as a live game (`lichess-game`) or on Lichess TV (`lichess-tv`). The app reaches it
//! through the core's test origin (`KCHESS_TEST_LICHESS_BASE` under `KCHESS_STORE_DEBUG=1`).
//! Paths it does not know are answered 404 and listed by `unknown`.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::util::Result;

/// The line the game and the TV feed play, one move every `MOVE_EVERY`: 24 plies of the Ruy
/// Lopez, as the app's own benchmarks step through.
const LINE: [&str; 24] = [
    "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4", "g8f6", "e1g1", "f8e7", "f1e1", "b7b5",
    "a4b3", "d7d6", "c2c3", "e8g8", "h2h3", "c6a5", "b3c2", "c7c5", "d2d4", "d8c7", "b1d2", "c5d4",
];
/// The position after each move of `LINE`.
const FENS: [&str; 24] = [
    "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
    "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
    "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",
    "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    "r1bqkbnr/pppp1ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
    "r1bqkbnr/1ppp1ppp/p1n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4",
    "r1bqkbnr/1ppp1ppp/p1n5/4p3/B3P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 1 4",
    "r1bqkb1r/1ppp1ppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 2 5",
    "r1bqkb1r/1ppp1ppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQ1RK1 b kq - 3 5",
    "r1bqk2r/1pppbppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQ1RK1 w kq - 4 6",
    "r1bqk2r/1pppbppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQR1K1 b kq - 5 6",
    "r1bqk2r/2ppbppp/p1n2n2/1p2p3/B3P3/5N2/PPPP1PPP/RNBQR1K1 w kq - 0 7",
    "r1bqk2r/2ppbppp/p1n2n2/1p2p3/4P3/1B3N2/PPPP1PPP/RNBQR1K1 b kq - 1 7",
    "r1bqk2r/2p1bppp/p1np1n2/1p2p3/4P3/1B3N2/PPPP1PPP/RNBQR1K1 w kq - 0 8",
    "r1bqk2r/2p1bppp/p1np1n2/1p2p3/4P3/1BP2N2/PP1P1PPP/RNBQR1K1 b kq - 0 8",
    "r1bq1rk1/2p1bppp/p1np1n2/1p2p3/4P3/1BP2N2/PP1P1PPP/RNBQR1K1 w - - 1 9",
    "r1bq1rk1/2p1bppp/p1np1n2/1p2p3/4P3/1BP2N1P/PP1P1PP1/RNBQR1K1 b - - 0 9",
    "r1bq1rk1/2p1bppp/p2p1n2/np2p3/4P3/1BP2N1P/PP1P1PP1/RNBQR1K1 w - - 1 10",
    "r1bq1rk1/2p1bppp/p2p1n2/np2p3/4P3/2P2N1P/PPBP1PP1/RNBQR1K1 b - - 2 10",
    "r1bq1rk1/4bppp/p2p1n2/npp1p3/4P3/2P2N1P/PPBP1PP1/RNBQR1K1 w - - 0 11",
    "r1bq1rk1/4bppp/p2p1n2/npp1p3/3PP3/2P2N1P/PPB2PP1/RNBQR1K1 b - - 0 11",
    "r1b2rk1/2q1bppp/p2p1n2/npp1p3/3PP3/2P2N1P/PPB2PP1/RNBQR1K1 w - - 1 12",
    "r1b2rk1/2q1bppp/p2p1n2/npp1p3/3PP3/2P2N1P/PPBN1PP1/R1BQR1K1 b - - 2 12",
    "r1b2rk1/2q1bppp/p2p1n2/np2p3/3pP3/2P2N1P/PPBN1PP1/R1BQR1K1 w - - 0 13",
];
const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// How often the game and the TV feed move: the line lasts a warm-up and a 30 s run.
const MOVE_EVERY: Duration = Duration::from_millis(1500);
/// How often an idle stream sends Lichess's keep-alive (an empty line).
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const GAME: &str = "BenchGame";
const TV_GAME: &str = "BenchTv1";
const USERNAME: &str = "Bench";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    /// Signed in; nothing happens.
    Idle,
    /// A game starts once `Bench` is signed in, and both sides play `LINE`.
    Game,
    /// Lichess TV's featured game plays `LINE`.
    Tv,
}

pub struct FakeLichess {
    pub base: String,
    stop: Arc<AtomicBool>,
    unknown: Arc<Mutex<BTreeSet<String>>>,
    accept: Option<thread::JoinHandle<()>>,
}

impl FakeLichess {
    /// The stand-in for benchmark `scenario`, or `None` if it does not use Lichess.
    pub fn start(scenario: &str) -> Result<Option<FakeLichess>> {
        let script = match scenario {
            "lichess-idle" | "pages-tour" => Script::Idle,
            "lichess-game" => Script::Game,
            "lichess-tv" => Script::Tv,
            _ => return Ok(None),
        };
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| format!("No port for Lichess: {e}"))?;
        let base = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let stop = Arc::new(AtomicBool::new(false));
        let unknown = Arc::new(Mutex::new(BTreeSet::new()));
        let accept = {
            let stop = Arc::clone(&stop);
            let unknown = Arc::clone(&unknown);
            thread::spawn(move || {
                for socket in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(socket) = socket else { continue };
                    let stop = Arc::clone(&stop);
                    let unknown = Arc::clone(&unknown);
                    thread::spawn(move || {
                        let _ = serve(socket, script, &stop, &unknown);
                    });
                }
            })
        };
        Ok(Some(FakeLichess {
            base,
            stop,
            unknown,
            accept: Some(accept),
        }))
    }

    /// The paths asked for that the stand-in does not answer.
    pub fn unknown(&self) -> Vec<String> {
        self.unknown.lock().unwrap().iter().cloned().collect()
    }
}

impl Drop for FakeLichess {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees the stop.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
}

fn read_request(socket: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    socket.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        socket.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body).ok()?;
    let path = target.split('?').next().unwrap_or("").to_string();
    Some(Request { method, path })
}

fn serve(
    socket: TcpStream,
    script: Script,
    stop: &AtomicBool,
    unknown: &Mutex<BTreeSet<String>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let Some(request) = read_request(&mut reader) else {
        return Ok(());
    };
    let mut out = socket;
    let path = request.path.as_str();
    if std::env::var_os("KCHESS_FAKE_LICHESS_TRACE").is_some() {
        eprintln!("[lichess] {} {}", request.method, request.path);
    }
    match (request.method.as_str(), path) {
        ("POST", "/api/token") => json_reply(
            &mut out,
            200,
            &json!({ "token_type": "Bearer", "access_token": "lip_bench", "expires_in": 31_536_000 }),
        ),
        ("GET", "/api/account") => json_reply(&mut out, 200, &account()),
        ("GET", "/api/account/playing") => {
            let playing = if script == Script::Game {
                json!([game_summary()])
            } else {
                json!([])
            };
            json_reply(&mut out, 200, &json!({ "nowPlaying": playing }))
        }
        ("GET", "/api/tv/channels") => json_reply(&mut out, 200, &tv_channels()),
        ("GET", "/api/stream/event") => {
            let start = (script == Script::Game)
                .then(|| json!({ "type": "gameStart", "game": game_summary() }));
            stream(&mut out, stop, |_| None, start.into_iter().collect())
        }
        ("GET", p) if p == format!("/api/board/game/stream/{GAME}") => {
            let since = Instant::now();
            stream(
                &mut out,
                stop,
                move |sent| game_state(sent, since),
                vec![game_full()],
            )
        }
        ("GET", p) if p.starts_with("/api/tv/") && p.ends_with("/feed") => {
            let since = Instant::now();
            stream(
                &mut out,
                stop,
                move |sent| tv_move(sent, since),
                vec![tv_featured()],
            )
        }
        ("GET", p) if p.starts_with("/api/games/user/") => ndjson_reply(&mut out, ""),
        ("POST", "/api/games/export/_ids") => ndjson_reply(&mut out, ""),
        ("GET", "/api/user/Bench") => json_reply(&mut out, 200, &account()),
        ("GET", "/api/user/Bench/rating-history") => json_reply(&mut out, 200, &json!([])),
        ("GET", "/api/users/status") => json_reply(&mut out, 200, &json!([])),
        ("GET", "/api/challenge") => json_reply(&mut out, 200, &json!({ "in": [], "out": [] })),
        ("GET", "/api/broadcast/top") => json_reply(
            &mut out,
            200,
            &json!({ "active": [], "upcoming": [], "past": { "currentPage": 1, "maxPerPage": 20, "currentPageResults": [], "nbResults": 0, "nbPages": 1 } }),
        ),
        ("GET", p) if p.starts_with("/api/player/top/") => {
            json_reply(&mut out, 200, &json!({ "users": [] }))
        }
        ("GET", p) if p.starts_with("/api/puzzle/batch/") => {
            json_reply(&mut out, 200, &json!({ "puzzles": [] }))
        }
        ("GET", p) if p.ends_with("/chat") || p.starts_with("/api/team/of/") => {
            json_reply(&mut out, 200, &json!([]))
        }
        ("GET", "/api/timeline") => {
            json_reply(&mut out, 200, &json!({ "entries": [], "users": {} }))
        }
        ("GET", "/api/tournament") => json_reply(
            &mut out,
            200,
            &json!({ "created": [], "started": [], "finished": [] }),
        ),
        ("GET", p) if p.starts_with("/api/crosstable/") => {
            json_reply(&mut out, 200, &json!({ "users": {}, "nbGames": 0 }))
        }
        _ => {
            unknown
                .lock()
                .unwrap()
                .insert(format!("{} {}", request.method, request.path));
            json_reply(&mut out, 404, &json!({ "error": "Not found" }))
        }
    }
}

fn json_reply(out: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    let reason = if status == 200 { "OK" } else { "Not Found" };
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    out.flush()
}

fn ndjson_reply(out: &mut TcpStream, body: &str) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    out.flush()
}

/// An ndjson stream: `first`'s lines at once, then `next(sent)`'s whenever it has one (`sent`
/// counts the lines so far), keep-alives in between, until the app hangs up or the run ends.
fn stream(
    out: &mut TcpStream,
    stop: &AtomicBool,
    mut next: impl FnMut(usize) -> Option<Value>,
    first: Vec<Value>,
) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )?;
    let mut sent = 0;
    for line in first {
        chunk(out, &format!("{line}\n"))?;
        sent += 1;
    }
    let mut quiet = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        if let Some(line) = next(sent) {
            chunk(out, &format!("{line}\n"))?;
            sent += 1;
            quiet = Instant::now();
        } else if quiet.elapsed() >= KEEP_ALIVE {
            chunk(out, "\n")?;
            quiet = Instant::now();
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = out.shutdown(Shutdown::Both);
    Ok(())
}

fn chunk(out: &mut TcpStream, text: &str) -> std::io::Result<()> {
    write!(out, "{:x}\r\n{text}\r\n", text.len())?;
    out.flush()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn account() -> Value {
    json!({
        "id": USERNAME.to_lowercase(),
        "username": USERNAME,
        "createdAt": 1_600_000_000_000u64,
        "seenAt": now_ms(),
        "perfs": {
            "blitz": { "games": 120, "rating": 1650, "rd": 60, "prog": 12 },
            "rapid": { "games": 80, "rating": 1720, "rd": 60, "prog": -4 },
            "puzzle": { "games": 300, "rating": 1800, "rd": 70, "prog": 20 }
        },
        "count": { "all": 200, "rated": 180, "win": 100, "loss": 90, "draw": 10 },
        "playTime": { "total": 360_000, "tv": 0 }
    })
}

fn opponent() -> Value {
    json!({ "id": "opponent", "username": "Opponent", "rating": 1640 })
}

fn game_summary() -> Value {
    json!({
        "gameId": GAME,
        "fullId": format!("{GAME}abcd"),
        "color": "white",
        "fen": START_FEN,
        "hasMoved": false,
        "isMyTurn": true,
        "lastMove": "",
        "opponent": opponent(),
        "perf": "blitz",
        "rated": false,
        "secondsLeft": 300,
        "source": "lobby",
        "speed": "blitz",
        "variant": { "key": "standard", "name": "Standard" },
        "compat": { "bot": false, "board": true }
    })
}

fn game_full() -> Value {
    json!({
        "type": "gameFull",
        "id": GAME,
        "rated": false,
        "variant": { "key": "standard", "name": "Standard", "short": "Std" },
        "clock": { "initial": 300_000, "increment": 3_000 },
        "speed": "blitz",
        "perf": { "name": "Blitz" },
        "createdAt": now_ms(),
        "white": { "id": USERNAME.to_lowercase(), "name": USERNAME, "rating": 1650 },
        "black": { "id": "opponent", "name": "Opponent", "rating": 1640 },
        "initialFen": "startpos",
        "state": state_after(0, Duration::ZERO)
    })
}

/// The game after `plies` moves, `elapsed` into it: each side's clock has run for its own moves.
fn state_after(plies: usize, elapsed: Duration) -> Value {
    let spent = |side: usize| {
        let moves = (plies + 1 - side) / 2;
        (moves as u64 * MOVE_EVERY.as_millis() as u64).min(elapsed.as_millis() as u64)
    };
    let increment = |side: usize| ((plies + 1 - side) / 2) as u64 * 3_000;
    json!({
        "type": "gameState",
        "moves": LINE[..plies].join(" "),
        "wtime": 300_000 - spent(0) + increment(0),
        "btime": 300_000 - spent(1) + increment(1),
        "winc": 3_000,
        "binc": 3_000,
        "status": "started"
    })
}

/// The next move of the game, once its time has come (`sent` lines so far, `gameFull` first).
fn game_state(sent: usize, since: Instant) -> Option<Value> {
    let plies = sent;
    (plies <= LINE.len() && since.elapsed() >= MOVE_EVERY * plies as u32)
        .then(|| state_after(plies, since.elapsed()))
}

fn tv_channels() -> Value {
    json!({
        "blitz": {
            "user": { "id": "featured", "name": "Featured", "title": "GM" },
            "rating": 2800,
            "gameId": TV_GAME,
            "color": "white"
        }
    })
}

fn tv_featured() -> Value {
    json!({
        "t": "featured",
        "d": {
            "id": TV_GAME,
            "fen": START_FEN,
            "orientation": "white",
            "players": [
                { "color": "white", "seconds": 180, "user": { "name": "Featured", "title": "GM" }, "rating": 2800 },
                { "color": "black", "seconds": 180, "user": { "name": "Challenger", "title": "IM" }, "rating": 2650 }
            ]
        }
    })
}

/// The TV game's next move, once its time has come (`sent` lines so far, `featured` first).
fn tv_move(sent: usize, since: Instant) -> Option<Value> {
    let ply = sent;
    (ply <= LINE.len() && since.elapsed() >= MOVE_EVERY * ply as u32).then(|| {
        let seconds = |side: usize| 180 - ((ply + 1 - side) / 2) as u64;
        json!({
            "t": "fen",
            "d": { "fen": FENS[ply - 1], "lm": LINE[ply - 1], "wc": seconds(0), "bc": seconds(1) }
        })
    })
}
