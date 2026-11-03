//! A stand-in Lichess for the end-to-end tests: one HTTP/1.1 server on 127.0.0.1 for the whole test
//! process, which the core reaches through its test origin. Tests run side by side, so each one
//! that signs in (`Harness::signed_in`) has a `Session` of its own, told apart by its login.
//!
//! A request without a login is dropped unanswered: to every test that does not sign in, Lichess
//! cannot be reached, as if nothing listened.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const OPPONENT: &str = "Opponent";
const GAME: &str = "E2eGame1";
const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// Black answers White's moves from this line while the game follows it.
const REPLIES: [(&str, &str); 3] = [("e2e4", "e7e5"), ("g1f3", "b8c6"), ("f1c4", "g8f6")];
const KEEP_ALIVE: Duration = Duration::from_secs(5);

/// What Lichess holds for one signed-in test.
#[derive(Default)]
pub struct Session {
    pub username: String,
    /// How many times the app opened the event stream, which brings games and challenges.
    pub streams: usize,
    /// How many of those streams the app is still reading. A request the app gave up on can
    /// still arrive late, so when a stream opened says less than whether it is still open.
    pub listening: usize,
    /// The login no longer works: its streams close and Lichess answers 401.
    pub revoked: bool,
    /// A game is under way: reported on the event stream and in `account/playing`.
    pub game: bool,
    pub moves: Vec<String>,
    /// Lichess's `status`, once the game has ended; `started` until then.
    pub finished: Option<&'static str>,
    pub winner: Option<&'static str>,
    /// Black offers a draw or a takeback.
    pub draw_offered: bool,
    pub takeback_offered: bool,
    /// The app's writes, in order ("POST /api/board/game/E2eGame1/move/e2e4").
    pub writes: Vec<String>,
    /// Counts changes: a game stream sends the state again whenever this moves on.
    version: usize,
}

impl Session {
    /// The game changed: its streams send the new state.
    pub fn changed(&mut self) {
        self.version += 1;
    }
}

#[derive(Clone)]
pub struct Handle(Arc<Mutex<Session>>);

impl Handle {
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        self.0.lock().expect("the session")
    }
}

struct Server {
    base: String,
    next: AtomicUsize,
    /// By the code the sign-in returns with, which is also the end of the login.
    sessions: Mutex<HashMap<String, Handle>>,
}

fn server() -> &'static Server {
    static SERVER: OnceLock<Server> = OnceLock::new();
    SERVER.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port for Lichess");
        let base = format!("http://{}", listener.local_addr().expect("its address"));
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = serve(socket);
                });
            }
        });
        Server {
            base,
            next: AtomicUsize::new(0),
            sessions: Mutex::new(HashMap::new()),
        }
    })
}

/// The origin the core is pointed at.
pub fn base() -> String {
    server().base.clone()
}

/// A new session, and the code that signs in to it.
pub fn session(username: &str, game: bool) -> (String, Handle) {
    let server = server();
    let code = server.next.fetch_add(1, Ordering::SeqCst).to_string();
    let session = Handle(Arc::new(Mutex::new(Session {
        username: username.to_string(),
        game,
        ..Session::default()
    })));
    server
        .sessions
        .lock()
        .expect("the sessions")
        .insert(code.clone(), session.clone());
    (code, session)
}

/// Return from the login page to the core's callback, as the browser does; `url` is the page the
/// core asked to open.
pub fn return_from_login(url: &str, code: &str) {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| value.replace("%3A", ":").replace("%2F", "/"))
        })
    };
    let redirect = param("redirect_uri").expect("the login's redirect");
    let state = param("state").expect("the login's state");
    let rest = redirect.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut socket = TcpStream::connect(host).expect("the core's callback");
    write!(
        socket,
        "GET /{path}?code={code}&state={state} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .expect("the callback is written");
    // Read the page to its end, then hang up first, as a browser does: the core's callback
    // waits for that before it closes.
    let mut response = Vec::new();
    let mut chunk = [0u8; 1024];
    while !response_complete(&response) {
        match socket.read(&mut chunk) {
            Ok(read) if read > 0 => response.extend_from_slice(&chunk[..read]),
            _ => break,
        }
    }
}

/// Whether `response` holds a whole response: its head and the `Content-Length` bytes after it.
fn response_complete(response: &[u8]) -> bool {
    let Some(end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&response[..end]);
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    response.len() >= end + 4 + length
}

struct Request {
    method: String,
    path: String,
    login: Option<String>,
    body: String,
}

fn read_request(socket: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    socket.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.split('?').next()?.to_string();
    let (mut length, mut login) = (0usize, None);
    loop {
        let mut header = String::new();
        socket.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("authorization") {
                login = value
                    .trim()
                    .strip_prefix("Bearer lip_e2e_")
                    .map(str::to_string);
            }
        }
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        login,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn serve(socket: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let Some(request) = read_request(&mut reader) else {
        return Ok(());
    };
    let mut out = socket;
    if (request.method.as_str(), request.path.as_str()) == ("POST", "/api/token") {
        let code = request
            .body
            .split('&')
            .find_map(|pair| pair.strip_prefix("code="))
            .unwrap_or_default();
        return reply(
            &mut out,
            &json!({ "token_type": "Bearer", "access_token": format!("lip_e2e_{code}"), "expires_in": 31_536_000 }),
        );
    }
    let session = request.login.as_ref().and_then(|login| {
        let sessions = server().sessions.lock().expect("the sessions");
        sessions.get(login).cloned()
    });
    // A player's activity is public too: a few weeks of it, which each name staggers.
    if let Some(name) = request
        .path
        .strip_prefix("/api/user/")
        .and_then(|rest| rest.strip_suffix("/activity"))
    {
        return reply(&mut out, &activity(name));
    }
    // Leaderboards are public, so they are asked for without the login.
    if let Some(perf) = request.path.strip_prefix("/api/player/top/100/") {
        let user = |username: &str, title: Option<&str>, rating: u32, progress: i32| {
            json!({
                "username": username,
                "title": title,
                "perfs": { perf: { "rating": rating, "progress": progress } },
            })
        };
        return reply(
            &mut out,
            &json!({ "users": [
                user("FirstGm", Some("GM"), 3004, 3),
                user("Untitled", None, 2974, 16),
                user("SomeIm", Some("IM"), 2916, -8),
                user("SecondGm", Some("GM"), 2908, 0),
            ] }),
        );
    }
    // Nobody signed in: no answer at all.
    let Some(session) = session else {
        return Ok(());
    };
    if session.lock().revoked {
        return unauthorized(&mut out);
    }
    let path = request.path.as_str();
    let stream_path = format!("/api/board/game/stream/{GAME}");
    match (request.method.as_str(), path) {
        ("GET", "/api/account") => {
            let username = session.lock().username.clone();
            reply(&mut out, &account(&username))
        }
        ("GET", "/api/account/playing") => {
            let session = session.lock();
            let playing: Vec<Value> = session
                .game
                .then(summary)
                .filter(|_| session.finished.is_none())
                .into_iter()
                .collect();
            reply(&mut out, &json!({ "nowPlaying": playing }))
        }
        ("GET", "/api/stream/event") => {
            let start = {
                let mut session = session.lock();
                session.streams += 1;
                session.listening += 1;
                session.game
            };
            let first = start.then(|| json!({ "type": "gameStart", "game": summary() }));
            let listening = session.clone();
            let open = move || !session.lock().revoked;
            // Until the app hangs up (a keep-alive fails to send) or the login is revoked.
            let ended = stream(&mut out, first.into_iter().collect(), open, |_| None);
            listening.lock().listening -= 1;
            ended
        }
        ("GET", p) if p == stream_path => {
            let (first, seen) = {
                let session = session.lock();
                (full(&session), session.version)
            };
            let mut seen = seen;
            stream(
                &mut out,
                vec![first],
                || true,
                move |_| {
                    let session = session.lock();
                    (session.version != seen).then(|| {
                        seen = session.version;
                        state(&session)
                    })
                },
            )
        }
        ("POST", p) if p.starts_with("/api/board/game/") => {
            write_game(&session, p);
            reply(&mut out, &json!({ "ok": true }))
        }
        ("GET", "/api/challenge") => reply(&mut out, &json!({ "in": [], "out": [] })),
        ("GET", "/api/users/status") => reply(&mut out, &json!([])),
        ("GET", "/api/timeline") => reply(&mut out, &json!({ "entries": [], "users": {} })),
        ("GET", p) if p.ends_with("/chat") || p.starts_with("/api/team/of/") => {
            reply(&mut out, &json!([]))
        }
        ("GET", p) if p.starts_with("/api/crosstable/") => {
            reply(&mut out, &json!({ "users": {}, "nbGames": 0 }))
        }
        ("GET", p) if p.starts_with("/api/games/user/") => ndjson(&mut out),
        ("POST", "/api/games/export/_ids") => ndjson(&mut out),
        _ => not_found(&mut out),
    }
}

/// The app acted in the game: Lichess records it, and Black answers a move.
fn write_game(session: &Handle, path: &str) {
    let mut session = session.lock();
    session.writes.push(format!("POST {path}"));
    let action = path.trim_start_matches("/api/board/game/");
    let action = action.split_once('/').map_or("", |(_, action)| action);
    match action.split('/').collect::<Vec<_>>().as_slice() {
        ["move", uci] => {
            session.moves.push((*uci).to_string());
            if let Some((_, reply)) = REPLIES.iter().find(|(played, _)| played == uci) {
                session.moves.push((*reply).to_string());
            }
        }
        ["resign"] => {
            session.finished = Some("resign");
            session.winner = Some("black");
        }
        ["abort"] => session.finished = Some("aborted"),
        ["draw", "yes"] if session.draw_offered => {
            session.draw_offered = false;
            session.finished = Some("draw");
        }
        ["draw", "no" | "false"] => session.draw_offered = false,
        ["takeback", "yes"] if session.takeback_offered => {
            session.takeback_offered = false;
            let kept = session.moves.len().saturating_sub(1);
            session.moves.truncate(kept);
        }
        ["takeback", "no" | "false"] => session.takeback_offered = false,
        _ => {}
    }
    session.changed();
}

fn reply(out: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    answered(out)
}

fn unauthorized(out: &mut TcpStream) -> std::io::Result<()> {
    let body = json!({ "error": "No such token" }).to_string();
    write!(
        out,
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    answered(out)
}

fn not_found(out: &mut TcpStream) -> std::io::Result<()> {
    let body = json!({ "error": "Not found" }).to_string();
    write!(
        out,
        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    answered(out)
}

fn ndjson(out: &mut TcpStream) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    answered(out)
}

/// An ndjson stream: `first`'s lines at once, then `next`'s whenever it has one, with keep-alives
/// in between, until the app hangs up or `open` says it is over.
fn stream(
    out: &mut TcpStream,
    first: Vec<Value>,
    open: impl Fn() -> bool,
    mut next: impl FnMut(usize) -> Option<Value>,
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
    // A hang-up is seen at once, not at the next keep-alive: the app does not send on a stream,
    // so a read that ends means it has gone.
    out.set_read_timeout(Some(Duration::from_millis(1)))?;
    while open() {
        if matches!(out.peek(&mut [0u8; 1]), Ok(0)) {
            return Ok(());
        }
        if let Some(line) = next(sent) {
            chunk(out, &format!("{line}\n"))?;
            sent += 1;
            quiet = Instant::now();
        } else if quiet.elapsed() >= KEEP_ALIVE {
            chunk(out, "\n")?;
            quiet = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    chunk(out, "")?;
    answered(out)
}

/// A response is written in full: the app hangs up first, or it can read a reset instead
/// (`kchess_core::loopback::close_after_client`).
fn answered(out: &mut TcpStream) -> std::io::Result<()> {
    out.flush()?;
    out.set_read_timeout(Some(kchess_core::loopback::HANG_UP_WAIT))?;
    std::io::copy(out, &mut std::io::sink()).map(|_| ())
}

fn chunk(out: &mut TcpStream, text: &str) -> std::io::Result<()> {
    write!(out, "{:x}\r\n{text}\r\n", text.len())?;
    out.flush()
}

fn account(username: &str) -> Value {
    json!({
        "id": username.to_lowercase(),
        "username": username,
        "createdAt": 1_600_000_000_000u64,
        "seenAt": 1_600_000_000_000u64,
        "perfs": { "rapid": { "games": 80, "rating": 1720, "rd": 60, "prog": 4 } },
        "count": { "all": 80, "rated": 80, "win": 40, "loss": 30, "draw": 10 },
        "playTime": { "total": 360_000, "tv": 0 }
    })
}

fn summary() -> Value {
    json!({
        "gameId": GAME,
        "fullId": format!("{GAME}abcd"),
        "color": "white",
        "fen": START_FEN,
        "hasMoved": false,
        "isMyTurn": true,
        "lastMove": "",
        "opponent": { "id": OPPONENT.to_lowercase(), "username": OPPONENT, "rating": 1700 },
        "perf": "rapid",
        "rated": false,
        "secondsLeft": 600,
        "source": "lobby",
        "speed": "rapid",
        "variant": { "key": "standard", "name": "Standard" },
        "compat": { "bot": false, "board": true }
    })
}

fn full(session: &Session) -> Value {
    json!({
        "type": "gameFull",
        "id": GAME,
        "rated": false,
        "variant": { "key": "standard", "name": "Standard", "short": "Std" },
        "clock": { "initial": 600_000, "increment": 5_000 },
        "speed": "rapid",
        "perf": { "name": "Rapid" },
        "createdAt": 1_600_000_000_000u64,
        "white": { "id": session.username.to_lowercase(), "name": session.username, "rating": 1720 },
        "black": { "id": OPPONENT.to_lowercase(), "name": OPPONENT, "rating": 1700 },
        "initialFen": "startpos",
        "state": state(session)
    })
}

fn state(session: &Session) -> Value {
    let mut state = json!({
        "type": "gameState",
        "moves": session.moves.join(" "),
        "wtime": 600_000,
        "btime": 600_000,
        "winc": 5_000,
        "binc": 5_000,
        "status": session.finished.unwrap_or("started"),
        "bdraw": session.draw_offered,
        "btakeback": session.takeback_offered,
    });
    if let Some(winner) = session.winner {
        state["winner"] = json!(winner);
    }
    state
}

/// A player's recent activity as Lichess sends it, days ago from today, different for each name
/// so a merged timeline interleaves and its ratings go both ways.
fn activity(name: &str) -> Value {
    const DAY: u64 = 86_400_000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    let today = now - now % DAY;
    let seed = name.bytes().map(u64::from).sum::<u64>();
    let shift = seed % 3;
    let base = 1800 + (seed % 7) * 110;
    let day = |ago: u64, body: Value| {
        let start = today - (ago + shift) * DAY;
        let mut day = json!({ "interval": { "start": start, "end": start + DAY } });
        day.as_object_mut()
            .expect("an object")
            .extend(body.as_object().expect("an object").clone());
        day
    };
    let (win, loss) = (1 + seed % 6, seed % 4);
    let moved = (win as i64 - loss as i64) * 6;
    json!([
        day(
            0,
            json!({
                "games": { "blitz": { "win": win, "loss": loss, "draw": seed % 2,
                                      "rp": { "before": base, "after": base as i64 + moved } } },
                "puzzles": { "score": { "win": 4 + seed % 9, "loss": 1 + seed % 5, "draw": 0,
                                        "rp": { "before": base + 300, "after": base as i64 + 300 - 7 } } },
            })
        ),
        day(
            1,
            json!({
                "tournaments": { "nb": 1, "best": [{ "tournament": { "id": "t", "name": "Hourly Blitz Arena" },
                                                     "nbGames": 6 + seed % 5, "rank": 2 + seed % 30,
                                                     "rankPercent": 1 + seed % 20 }] },
            })
        ),
        day(
            2 + seed % 2,
            json!({
                "follows": { "out": { "ids": ["Fiona"] } },
                "games": { "rapid": { "win": 1, "loss": 0, "draw": 0,
                                      "rp": { "before": base - 90, "after": base - 82 } } },
            })
        ),
        day(
            9,
            json!({
                "studies": [{ "name": "Caro-Kann ideas" }],
                "storm": { "runs": 3, "score": 20 + seed % 30 },
            })
        ),
        day(
            40,
            json!({
                "games": { "bullet": { "win": 0, "loss": 3, "draw": 0,
                                       "rp": { "before": base + 40, "after": base + 4 } } },
            })
        ),
    ])
}
