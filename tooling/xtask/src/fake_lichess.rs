//! A stand-in Lichess for the signed-in benchmarks (`bench`): a small HTTP/1.1 server on
//! 127.0.0.1 that signs `Bench` in through the app's own OAuth flow and plays out one scripted
//! line, as a live game (`lichess-game`) or on Lichess TV (`lichess-tv`). The app reaches it
//! through the core's test origin (`KCHESS_TEST_LICHESS_BASE` under `KCHESS_STORE_DEBUG=1`).
//! Paths it does not know are answered 404 and listed by `unknown`.

use std::collections::BTreeSet;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kchess_testkit::{Request, Server, json as json_reply, ndjson};
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
    /// Ends the streams; the server stops when it is dropped.
    stop: Arc<AtomicBool>,
    unknown: Arc<Mutex<BTreeSet<String>>>,
    _server: Server,
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
        let stop = Arc::new(AtomicBool::new(false));
        let unknown = Arc::new(Mutex::new(BTreeSet::new()));
        let server = {
            let stop = Arc::clone(&stop);
            let unknown = Arc::clone(&unknown);
            Server::start(move |request, out| serve(request, out, script, &stop, &unknown))
                .map_err(|e| format!("No port for Lichess: {e}"))?
        };
        Ok(Some(FakeLichess {
            base: server.base().to_string(),
            stop,
            unknown,
            _server: server,
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
    }
}

fn serve(
    request: Request,
    out: &mut TcpStream,
    script: Script,
    stop: &AtomicBool,
    unknown: &Mutex<BTreeSet<String>>,
) -> std::io::Result<()> {
    let path = request.path();
    if std::env::var_os("KCHESS_FAKE_LICHESS_TRACE").is_some() {
        eprintln!("[lichess] {} {path}", request.method);
    }
    match (request.method.as_str(), path) {
        ("POST", "/api/token") => json_reply(
            out,
            200,
            &json!({ "token_type": "Bearer", "access_token": "lip_bench", "expires_in": 31_536_000 }),
        ),
        ("GET", "/api/account") => json_reply(out, 200, &account()),
        ("GET", "/api/account/playing") => {
            let playing = if script == Script::Game {
                json!([game_summary()])
            } else {
                json!([])
            };
            json_reply(out, 200, &json!({ "nowPlaying": playing }))
        }
        ("GET", "/api/tv/channels") => json_reply(out, 200, &tv_channels()),
        ("GET", "/api/stream/event") => {
            let start = (script == Script::Game)
                .then(|| json!({ "type": "gameStart", "game": game_summary() }));
            stream(out, stop, start.into_iter().collect(), |_| None)
        }
        ("GET", p) if p == format!("/api/board/game/stream/{GAME}") => {
            let since = Instant::now();
            stream(out, stop, vec![game_full()], move |sent| {
                game_state(sent, since)
            })
        }
        ("GET", p) if p.starts_with("/api/tv/") && p.ends_with("/feed") => {
            let since = Instant::now();
            stream(out, stop, vec![tv_featured()], move |sent| {
                tv_move(sent, since)
            })
        }
        ("GET", p) if p.starts_with("/api/games/user/") => ndjson(out, &[]),
        ("POST", "/api/games/export/_ids") => ndjson(out, &[]),
        ("GET", "/api/user/Bench") => json_reply(out, 200, &account()),
        ("GET", "/api/user/Bench/rating-history") => json_reply(out, 200, &json!([])),
        ("GET", "/api/users/status") => json_reply(out, 200, &json!([])),
        ("GET", "/api/challenge") => json_reply(out, 200, &json!({ "in": [], "out": [] })),
        ("GET", "/api/broadcast/top") => json_reply(
            out,
            200,
            &json!({ "active": [], "upcoming": [], "past": { "currentPage": 1, "maxPerPage": 20, "currentPageResults": [], "nbResults": 0, "nbPages": 1 } }),
        ),
        ("GET", p) if p.starts_with("/api/player/top/") => {
            json_reply(out, 200, &json!({ "users": [] }))
        }
        ("GET", p) if p.starts_with("/api/puzzle/batch/") => {
            json_reply(out, 200, &json!({ "puzzles": [] }))
        }
        ("GET", p) if p.ends_with("/chat") || p.starts_with("/api/team/of/") => {
            json_reply(out, 200, &json!([]))
        }
        ("GET", "/api/tournament") => json_reply(
            out,
            200,
            &json!({ "created": [], "started": [], "finished": [] }),
        ),
        ("GET", p) if p.starts_with("/api/crosstable/") => {
            json_reply(out, 200, &json!({ "users": {}, "nbGames": 0 }))
        }
        _ => {
            unknown
                .lock()
                .unwrap()
                .insert(format!("{} {path}", request.method));
            json_reply(out, 404, &json!({ "error": "Not found" }))
        }
    }
}

/// An ndjson stream: `first`'s lines at once, then `next(sent)`'s whenever it has one (`sent`
/// counts the lines so far), keep-alives in between, until the app hangs up or the run ends.
fn stream(
    out: &mut TcpStream,
    stop: &AtomicBool,
    first: Vec<Value>,
    next: impl FnMut(usize) -> Option<Value>,
) -> std::io::Result<()> {
    kchess_testkit::ndjson_stream(
        out,
        first,
        KEEP_ALIVE,
        || !stop.load(Ordering::SeqCst),
        next,
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    /// One HTTP request to `lichess`: the status line and the body, read until it hangs up.
    fn request(lichess: &FakeLichess, method: &str, path: &str, body: &str) -> (String, String) {
        let mut socket = TcpStream::connect(lichess.base.trim_start_matches("http://")).unwrap();
        write!(
            socket,
            "{method} {path}?x=1 HTTP/1.1\r\nHost: lichess\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        // The head, then the body its length gives; the stand-in waits for us to hang up.
        let mut reader = BufReader::new(socket);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            // A stand-in that hangs up without an answer fails here rather than spinning.
            assert!(reader.read_line(&mut line).unwrap() > 0, "{path}: no answer");
            if line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let reply = Request::from_head(&format!("GET / HTTP/1.1\r\n{head}")).unwrap();
        let mut body = vec![0; reply.content_length()];
        reader.read_exact(&mut body).unwrap();
        let status = head.lines().next().unwrap().to_string();
        (status, String::from_utf8(body).unwrap())
    }

    fn get_json(lichess: &FakeLichess, path: &str) -> Value {
        let (status, body) = request(lichess, "GET", path, "");
        assert_eq!(status, "HTTP/1.1 200 OK", "{path}");
        serde_json::from_str(&body).unwrap()
    }

    /// A stream from `lichess`: its first `lines` non-empty ndjson lines.
    fn stream_lines(lichess: &FakeLichess, path: &str, lines: usize) -> Vec<Value> {
        let mut socket = TcpStream::connect(lichess.base.trim_start_matches("http://")).unwrap();
        write!(socket, "GET {path} HTTP/1.1\r\n\r\n").unwrap();
        let mut reader = BufReader::new(socket);
        let mut found = Vec::new();
        while found.len() < lines {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0, "{path} ended");
            if line.trim_start().starts_with('{') {
                found.push(serde_json::from_str(line.trim()).unwrap());
            }
        }
        found
    }

    #[test]
    fn only_the_lichess_scenarios_have_a_stand_in() {
        assert!(FakeLichess::start("analysis").unwrap().is_none());
        for scenario in ["lichess-idle", "pages-tour", "lichess-game", "lichess-tv"] {
            assert!(
                FakeLichess::start(scenario).unwrap().is_some(),
                "{scenario}"
            );
        }
    }

    #[test]
    fn it_answers_what_the_app_asks_and_lists_the_rest() {
        let lichess = FakeLichess::start("lichess-idle").unwrap().unwrap();
        let (status, token) = request(&lichess, "POST", "/api/token", "code=abc&verifier=x");
        assert_eq!(status, "HTTP/1.1 200 OK");
        let token: Value = serde_json::from_str(&token).unwrap();
        assert_eq!(token["access_token"], "lip_bench");
        assert_eq!(token["token_type"], "Bearer");

        let account = get_json(&lichess, "/api/account");
        assert_eq!(account["username"], USERNAME);
        assert_eq!(account["id"], "bench");
        assert!(account["seenAt"].as_u64().unwrap() > 1_700_000_000_000);
        assert_eq!(account["perfs"]["blitz"]["rating"], 1650);
        assert_eq!(get_json(&lichess, "/api/user/Bench")["username"], USERNAME);
        assert_eq!(
            get_json(&lichess, "/api/account/playing")["nowPlaying"],
            json!([])
        );
        let channels = get_json(&lichess, "/api/tv/channels");
        assert_eq!(channels["blitz"]["gameId"], TV_GAME);
        assert_eq!(channels["blitz"]["user"]["name"], "Featured");

        for (path, answer) in [
            ("/api/user/Bench/rating-history", json!([])),
            ("/api/users/status", json!([])),
            ("/api/challenge", json!({ "in": [], "out": [] })),
            ("/api/player/top/10/blitz", json!({ "users": [] })),
            ("/api/puzzle/batch/mix", json!({ "puzzles": [] })),
            ("/api/game/abc/chat", json!([])),
            ("/api/team/of/bench", json!([])),
            (
                "/api/tournament",
                json!({ "created": [], "started": [], "finished": [] }),
            ),
            ("/api/crosstable/a/b", json!({ "users": {}, "nbGames": 0 })),
        ] {
            assert_eq!(get_json(&lichess, path), answer, "{path}");
        }
        assert_eq!(
            get_json(&lichess, "/api/broadcast/top")["past"]["nbPages"],
            1
        );
        for (method, path) in [
            ("GET", "/api/games/user/Bench"),
            ("POST", "/api/games/export/_ids"),
        ] {
            let (status, body) = request(&lichess, method, path, "a,b");
            assert_eq!(status, "HTTP/1.1 200 OK", "{path}");
            assert_eq!(body, "", "{path}");
        }

        for path in [
            "/api/tv/blitz",
            "/api/other/feed",
            "/api/board/game/stream/Other",
        ] {
            let (status, body) = request(&lichess, "GET", path, "");
            assert_eq!(status, "HTTP/1.1 404 Not Found", "{path}");
            assert_eq!(body, r#"{"error":"Not found"}"#);
        }
        assert_eq!(
            lichess.unknown(),
            [
                "GET /api/board/game/stream/Other",
                "GET /api/other/feed",
                "GET /api/tv/blitz"
            ]
        );
    }

    #[test]
    fn the_game_scenario_starts_bench_s_game() {
        let lichess = FakeLichess::start("lichess-game").unwrap().unwrap();
        let playing = get_json(&lichess, "/api/account/playing");
        assert_eq!(playing["nowPlaying"][0]["gameId"], GAME);
        assert_eq!(playing["nowPlaying"][0]["opponent"]["username"], "Opponent");
        assert_eq!(playing["nowPlaying"][0]["fullId"], "BenchGameabcd");
        let start = stream_lines(&lichess, "/api/stream/event", 1);
        assert_eq!(start[0]["type"], "gameStart");
        assert_eq!(start[0]["game"]["gameId"], GAME);
        let full = stream_lines(&lichess, &format!("/api/board/game/stream/{GAME}"), 1);
        assert_eq!(full[0]["type"], "gameFull");
        assert_eq!(full[0]["id"], GAME);
        assert_eq!(full[0]["white"]["name"], USERNAME);
        assert_eq!(full[0]["state"]["moves"], "");
        assert!(full[0]["createdAt"].as_u64().unwrap() > 1_700_000_000_000);
        let tv = stream_lines(&lichess, "/api/tv/blitz/feed", 1);
        assert_eq!(tv[0]["t"], "featured");
        assert_eq!(tv[0]["d"]["id"], TV_GAME);
        assert_eq!(tv[0]["d"]["players"][1]["user"]["name"], "Challenger");
    }

    #[test]
    fn an_idle_event_stream_stays_quiet_and_ends_with_the_stand_in() {
        let lichess = FakeLichess::start("lichess-idle").unwrap().unwrap();
        let mut socket = TcpStream::connect(lichess.base.trim_start_matches("http://")).unwrap();
        write!(socket, "GET /api/stream/event HTTP/1.1\r\n\r\n").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(400)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        assert!(head.contains("Transfer-Encoding: chunked"), "{head}");
        // No game, and no keep-alive before KEEP_ALIVE has passed.
        let mut more = [0u8; 16];
        assert!(reader.read(&mut more).is_err());
        // Stopping the stand-in ends its streams: the last, empty chunk.
        drop(lichess);
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut last = String::new();
        reader.read_line(&mut last).unwrap();
        assert_eq!(last, "0\r\n");
    }

    #[test]
    fn each_side_s_clock_runs_for_its_own_moves() {
        let start = state_after(0, Duration::ZERO);
        assert_eq!(start["moves"], "");
        assert_eq!(
            (start["wtime"].as_u64(), start["btime"].as_u64()),
            (Some(300_000), Some(300_000))
        );
        // Three plies: White has moved twice and Black once, each gaining the increment.
        let later = state_after(3, Duration::from_secs(10));
        assert_eq!(later["moves"], "e2e4 e7e5 g1f3");
        assert_eq!(later["wtime"], 300_000 - 3_000 + 6_000);
        assert_eq!(later["btime"], 300_000 - 1_500 + 3_000);
        assert_eq!(later["status"], "started");
        // A clock never runs for longer than the game has.
        let early = state_after(3, Duration::from_secs(2));
        assert_eq!(early["wtime"], 300_000 - 2_000 + 6_000);
        let four = state_after(4, Duration::from_secs(60));
        assert_eq!(four["btime"], 300_000 - 3_000 + 6_000);
    }

    fn ago(seconds: f64) -> Instant {
        Instant::now() - Duration::from_secs_f64(seconds)
    }

    #[test]
    fn the_game_moves_once_its_time_has_come() {
        let first = game_state(1, ago(2.0)).unwrap();
        assert_eq!(first["moves"], "e2e4");
        assert!(game_state(2, ago(2.0)).is_none());
        assert_eq!(game_state(24, ago(40.0)).unwrap()["moves"], LINE.join(" "));
        assert!(game_state(25, ago(100.0)).is_none());
    }

    #[test]
    fn the_tv_game_moves_once_its_time_has_come() {
        let first = tv_move(1, ago(2.0)).unwrap();
        assert_eq!(first["t"], "fen");
        assert_eq!(first["d"]["fen"], FENS[0]);
        assert_eq!(first["d"]["lm"], "e2e4");
        assert_eq!(
            (first["d"]["wc"].as_u64(), first["d"]["bc"].as_u64()),
            (Some(179), Some(180))
        );
        let fourth = tv_move(4, ago(10.0)).unwrap();
        assert_eq!(
            (fourth["d"]["wc"].as_u64(), fourth["d"]["bc"].as_u64()),
            (Some(178), Some(178))
        );
        assert!(tv_move(2, ago(2.0)).is_none());
        assert_eq!(tv_move(24, ago(40.0)).unwrap()["d"]["lm"], "c5d4");
        assert!(tv_move(25, ago(100.0)).is_none());
    }
}
