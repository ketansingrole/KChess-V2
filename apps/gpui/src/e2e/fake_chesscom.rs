//! A stand-in Chess.com public API for the end-to-end tests: one HTTP/1.1 server on 127.0.0.1 for
//! the whole test process, which the core reaches through its test origin
//! (`KCHESS_TEST_CHESSCOM_BASE`). Players whose names start with `e2e` exist and have played two
//! games in January 2020; every other player is unknown.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;

use serde_json::{Value, json};

/// The month every stand-in player's games are in.
const MONTH: &str = "2020/01";

/// The origin the core is pointed at.
pub fn base() -> String {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port for Chess.com");
        let base = format!("http://{}", listener.local_addr().expect("its address"));
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = serve(socket);
                });
            }
        });
        base
    })
    .clone()
}

/// The games `name` played: a win as White and a loss as Black.
fn games(name: &str) -> Value {
    let game = |number: u64, white: &str, black: &str, results: (&str, &str), moves: &str| {
        json!({
            "url": format!("https://www.chess.com/game/live/{number}"),
            "pgn": format!(
                "[Event \"Live Chess\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[UTCDate \"2020.01.1{}\"]\n[UTCTime \"12:00:00\"]\n\n{moves}",
                number % 10
            ),
            "time_control": "600",
            "time_class": "rapid",
            "end_time": 1_579_000_000 + number,
            "rated": true,
            "rules": "chess",
            "white": { "username": white, "rating": 1500, "result": results.0 },
            "black": { "username": black, "rating": 1450, "result": results.1 },
        })
    };
    json!({ "games": [
        game(
            1001,
            name,
            "Rival",
            ("win", "resigned"),
            "1. e4 c5 2. Nf3 d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 a6 1-0",
        ),
        game(
            1002,
            "Rival",
            name,
            ("win", "checkmated"),
            "1. f3 e5 2. g4 Qh4# 0-1",
        ),
    ]})
}

fn answer(path: &str) -> (u16, Value) {
    let Some(rest) = path.strip_prefix("/pub/player/") else {
        return (404, json!({ "code": 0 }));
    };
    let mut parts = rest.splitn(2, '/');
    let name = parts.next().unwrap_or_default();
    if !name.starts_with("e2e") {
        return (404, json!({ "code": 0, "message": "User not found." }));
    }
    match parts.next() {
        None => (
            200,
            json!({ "username": name, "url": format!("https://www.chess.com/member/{name}") }),
        ),
        Some("games/archives") => (
            200,
            json!({ "archives": [format!("https://api.chess.com/pub/player/{name}/games/{MONTH}")] }),
        ),
        Some(month) if month == format!("games/{MONTH}") => (200, games(name)),
        Some(_) => (404, json!({ "code": 0 })),
    }
}

fn serve(socket: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    // The headers, up to the blank line.
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
    }
    let (status, body) = answer(&path);
    let body = body.to_string();
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let mut out = socket;
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    out.flush()?;
    out.set_read_timeout(Some(kchess_core::loopback::HANG_UP_WAIT))?;
    std::io::copy(&mut out, &mut std::io::sink()).map(|_| ())
}
