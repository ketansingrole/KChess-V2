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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
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
    /// The game was started where the Board API cannot play it (`compat.board` false): its
    /// board stream answers 400, as Lichess does.
    pub elsewhere: bool,
    /// The account has years of history: ratings in every speed, a rating history and a few
    /// dozen synced games, for screenshots of the pages that show them.
    pub showcase: bool,
    /// The game's listing says the Board API can play it, but its board stream answers 400 (an
    /// open one ends).
    pub board_refused: bool,
    /// How many board streams were refused.
    pub refusals: usize,
    /// How many times the app opened the game's board stream.
    pub board_streams: usize,
    pub moves: Vec<String>,
    /// The game's Lichess variant key; standard when unset.
    pub variant: Option<&'static str>,
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
    /// A test that fails while holding the session must not fail every test after it: the
    /// server is shared, so a poisoned lock is taken over as it is.
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Server {
    base: String,
    next: AtomicUsize,
    /// By the code the sign-in returns with, which is also the end of the login.
    sessions: Mutex<HashMap<String, Handle>>,
    /// Seconds each player has played beyond `account`'s, by lowercase name (`play`).
    played: Mutex<HashMap<String, u64>>,
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
            played: Mutex::new(HashMap::new()),
        }
    })
}

/// `name` finishes a game of `seconds`: their profile's play time grows by it.
pub fn play(name: &str, seconds: u64) {
    *server()
        .played
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(name.to_lowercase())
        .or_default() += seconds;
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
        .unwrap_or_else(PoisonError::into_inner)
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
        let sessions = server()
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
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
    // Profiles are public: many at once, by the comma-separated names in the body.
    if request.method == "POST" && request.path == "/api/users" {
        let played = server()
            .played
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let users: Vec<Value> = request
            .body
            .split(',')
            .map(|name| {
                let mut user = account(name.trim());
                user["playTime"]["total"] = json!(
                    360_000
                        + played
                            .get(&name.trim().to_lowercase())
                            .copied()
                            .unwrap_or(0)
                );
                user
            })
            .collect();
        return reply(&mut out, &json!(users));
    }
    // A followed player's games are public too: they have none.
    if request.method == "GET" && request.path.starts_with("/api/games/user/") && session.is_none()
    {
        return ndjson(&mut out);
    }
    // Name search is public: a few players whatever is typed, one of them online.
    if request.path == "/api/player/autocomplete" {
        return reply(
            &mut out,
            &json!({ "result": [
                { "id": "magnuscarlsen", "name": "MagnusCarlsen", "title": "GM", "online": true },
                { "id": "magicmike", "name": "MagicMike" },
                { "id": "magnolia_77", "name": "Magnolia_77" },
            ] }),
        );
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
    // A showcase account's profile, rating history and games are public, as on Lichess.
    if let Some(reply_to) = showcase_public(&request.path) {
        return match reply_to {
            Showcase::Profile(name) => reply(&mut out, &showcase_account(&name)),
            Showcase::History => reply(&mut out, &showcase_history()),
            Showcase::Games(name) => ndjson_lines(&mut out, &showcase_games(&name)),
        };
    }
    // Games are public: the one a pasted link can fetch here.
    if (request.method.as_str(), request.path.as_str()) == ("POST", "/api/games/export/_ids")
        && request.body.trim() == LINKED_GAME
    {
        return pgn(&mut out, LINKED_PGN);
    }
    // Puzzles are public too: the one a pasted training link names.
    if (request.method.as_str(), request.path.as_str())
        == ("GET", format!("/api/puzzle/{LINKED_PUZZLE}").as_str())
    {
        return reply(
            &mut out,
            &json!({
                "game": { "id": LINKED_GAME },
                "puzzle": {
                    "id": LINKED_PUZZLE,
                    "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
                    "lastMove": "g8f6",
                    "solution": ["h5f7"],
                    "rating": 1100,
                    "themes": ["mateIn1", "short"],
                },
            }),
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
    if session.lock().showcase {
        let username = session.lock().username.clone();
        let name = username.to_lowercase();
        let user = format!("/api/user/{name}");
        if path == "/api/account" || path.to_lowercase() == user {
            return reply(&mut out, &showcase_account(&username));
        }
        if path.to_lowercase() == format!("{user}/rating-history") {
            return reply(&mut out, &showcase_history());
        }
        if path.to_lowercase() == format!("/api/games/user/{name}") {
            return ndjson_lines(&mut out, &showcase_games(&username));
        }
    }
    match (request.method.as_str(), path) {
        ("GET", "/api/account") => {
            let username = session.lock().username.clone();
            reply(&mut out, &account(&username))
        }
        ("GET", "/api/account/playing") => {
            let session = session.lock();
            let playing: Vec<Value> = session
                .game
                .then(|| summary(session.elsewhere, session.variant))
                .filter(|_| session.finished.is_none())
                .into_iter()
                .collect();
            reply(&mut out, &json!({ "nowPlaying": playing }))
        }
        ("GET", "/api/stream/event") => {
            let (start, elsewhere, variant) = {
                let mut session = session.lock();
                session.streams += 1;
                session.listening += 1;
                (session.game, session.elsewhere, session.variant)
            };
            let first =
                start.then(|| json!({ "type": "gameStart", "game": summary(elsewhere, variant) }));
            let listening = session.clone();
            // A refused game's announcement comes after the refusal, the order that once lost it.
            let waited = Instant::now();
            while session.lock().board_refused
                && session.lock().refusals == 0
                && waited.elapsed() < Duration::from_secs(5)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            let open = move || !session.lock().revoked;
            // Until the app hangs up (a keep-alive fails to send) or the login is revoked.
            let ended = stream(&mut out, first.into_iter().collect(), open, |_| None);
            listening.lock().listening -= 1;
            ended
        }
        ("GET", p) if p == stream_path => {
            let (first, seen, elsewhere) = {
                let mut session = session.lock();
                session.board_streams += 1;
                let refused = session.elsewhere || session.board_refused;
                (full(&session), session.version, refused)
            };
            if elsewhere {
                session.lock().refusals += 1;
                return refuse(
                    &mut out,
                    &json!({ "error": "This game cannot be played with the Board API." }),
                );
            }
            let mut seen = seen;
            let watched = session.clone();
            stream(
                &mut out,
                vec![first],
                // Refusing the game later ends its open stream; the app's reconnect gets the 400.
                move || !watched.lock().board_refused,
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

fn refuse(out: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    write!(
        out,
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

/// The game a Lichess link in the analysis board's import names, and its PGN.
pub const LINKED_GAME: &str = "kchessE2";
/// The puzzle a pasted lichess.org/training link opens.
pub const LINKED_PUZZLE: &str = "KcPz1";
const LINKED_PGN: &str = "[Event \"Rated rapid game\"]\n[Site \"https://lichess.org/kchessE2\"]\n\
[White \"alice\"]\n[Black \"Tester\"]\n[Result \"1-0\"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 1-0\n";

fn pgn(out: &mut TcpStream, body: &str) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-chess-pgn\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

fn ndjson_lines(out: &mut TcpStream, lines: &[Value]) -> std::io::Result<()> {
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
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

fn summary(elsewhere: bool, variant: Option<&str>) -> Value {
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
        "variant": { "key": variant.unwrap_or("standard") },
        "compat": { "bot": false, "board": !elsewhere }
    })
}

fn full(session: &Session) -> Value {
    json!({
        "type": "gameFull",
        "id": GAME,
        "rated": false,
        "variant": { "key": session.variant.unwrap_or("standard") },
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

/// A showcase account: rated in every speed, Blitz climbing.
fn showcase_account(username: &str) -> Value {
    let perf = |games: u32, rating: u32, prog: i32| json!({ "games": games, "rating": rating, "rd": 55, "prog": prog });
    json!({
        "id": username.to_lowercase(),
        "username": username,
        "createdAt": 1_600_000_000_000u64,
        "seenAt": now_ms(),
        "perfs": {
            "bullet": perf(412, 1302, -6),
            "blitz": perf(2870, 1445, 41),
            "rapid": perf(560, 1538, 12),
            "classical": perf(55, 1610, 0),
            "puzzle": perf(1240, 1820, 18),
        },
        "count": { "all": 3897, "rated": 3810, "win": 2014, "loss": 1779, "draw": 104 },
        "playTime": { "total": 4_200_000, "tv": 0 }
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// A pseudo-random walk, the same every run: two years of a rating, ending at `end`.
fn walk(seed: u64, points: usize, end: i64, spread: i64) -> Vec<i64> {
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        ((state >> 33) % 1000) as i64
    };
    let mut ratings = vec![end];
    for _ in 1..points {
        let last = *ratings.last().unwrap_or(&end);
        ratings.push(last - (next() * spread / 1000 - spread / 2) - 1);
    }
    ratings.reverse();
    ratings
}

/// Lichess's rating history: one series per speed, a point a few days apart.
fn showcase_history() -> Value {
    use chrono::{Datelike as _, Duration as Days, Utc};
    let today = Utc::now().date_naive();
    let series = |name: &str, seed: u64, points: usize, end: i64, every: i64| {
        let ratings = walk(seed, points, end, 34);
        let points: Vec<Value> = ratings
            .iter()
            .enumerate()
            .map(|(index, rating)| {
                let day = today - Days::days((points - 1 - index) as i64 * every);
                json!([day.year(), day.month0(), day.day(), rating])
            })
            .collect();
        json!({ "name": name, "points": points })
    };
    json!([
        series("Bullet", 7, 60, 1302, 12),
        series("Blitz", 11, 240, 1445, 3),
        series("Rapid", 13, 90, 1538, 8),
        series("Classical", 17, 12, 1610, 60),
        series("Puzzles", 19, 120, 1820, 6),
    ])
}

/// The account's latest games, newest first: mostly Blitz, a few Rapid, all openings sound.
fn showcase_games(username: &str) -> Vec<Value> {
    const OPPONENTS: [&str; 12] = [
        "yrod_doma",
        "lore_puli",
        "Ohnooob",
        "khoinguyen2602",
        "PawnStormer",
        "quiet_bishop",
        "Mirela_K",
        "endgame_enjoyer",
        "TacticalTom",
        "zugzwang42",
        "Nadia_R",
        "knightrider77",
    ];
    const LINES: [(&str, &str, &str); 4] = [
        (
            "e4 e5 Nf3 Nc6 Bb5 a6 Ba4 Nf6 O-O Be7",
            "C84",
            "Ruy Lopez: Closed",
        ),
        (
            "d4 d5 c4 e6 Nc3 Nf6 Bg5 Be7 e3 O-O",
            "D53",
            "Queen's Gambit Declined",
        ),
        (
            "e4 c5 Nf3 d6 d4 cxd4 Nxd4 Nf6 Nc3 a6",
            "B90",
            "Sicilian Defense: Najdorf",
        ),
        (
            "e4 e6 d4 d5 Nc3 Bb4 e5 c5 a3 Bxc3+",
            "C18",
            "French Defense: Winawer",
        ),
    ];
    const HOUR: u64 = 3_600_000;
    let now = now_ms();
    let user = json!({ "name": username, "id": username.to_lowercase() });
    (0..40u64)
        .map(|index| {
            let opponent = OPPONENTS[index as usize % OPPONENTS.len()];
            let (moves, eco, opening) = LINES[index as usize % LINES.len()];
            let white = index % 2 == 0;
            let outcome = [1, 0, 1, 2, 1, 0, 1, 1, 0, 1][index as usize % 10];
            let mine = 1445 - index as i64 * 3;
            let theirs = mine + [12, -30, 5, 44, -8, 21, -15, 3, 60, -22][index as usize % 10];
            let diff = match outcome {
                1 => 6,
                0 => -6,
                _ => 0,
            };
            let me = json!({ "user": user, "rating": mine - diff, "ratingDiff": diff });
            let them = json!({
                "user": { "name": opponent, "id": opponent.to_lowercase() },
                "rating": theirs,
                "ratingDiff": -diff,
            });
            let winner = match (outcome, white) {
                (2, _) => None,
                (1, true) | (0, false) => Some("white"),
                _ => Some("black"),
            };
            let rapid = index % 5 == 4;
            let created = now - (index * 7 + 2) * HOUR;
            let mut game = json!({
                "id": format!("Show{index:04}"),
                "rated": true,
                "variant": "standard",
                "speed": if rapid { "rapid" } else { "blitz" },
                "perf": if rapid { "rapid" } else { "blitz" },
                "createdAt": created,
                "lastMoveAt": created + 600_000,
                "status": if outcome == 2 { "draw" } else if index % 3 == 0 { "mate" } else { "resign" },
                "players": if white {
                    json!({ "white": me, "black": them })
                } else {
                    json!({ "white": them, "black": me })
                },
                "opening": { "eco": eco, "name": opening, "ply": 10 },
                "moves": moves,
                "clock": if rapid {
                    json!({ "initial": 600, "increment": 0, "totalTime": 600 })
                } else {
                    json!({ "initial": 180, "increment": 2, "totalTime": 260 })
                },
            });
            if let Some(winner) = winner {
                game["winner"] = json!(winner);
            }
            game
        })
        .collect()
}

enum Showcase {
    Profile(String),
    History,
    Games(String),
}

/// What a public request asks of a showcase account, if it names one.
fn showcase_public(path: &str) -> Option<Showcase> {
    let lower = path.to_lowercase();
    let (name, kind) = if let Some(rest) = lower.strip_prefix("/api/games/user/") {
        (rest.to_string(), 2)
    } else {
        let rest = lower.strip_prefix("/api/user/")?;
        match rest.split_once('/') {
            None => (rest.to_string(), 0),
            Some((name, "rating-history")) => (name.to_string(), 1),
            Some(_) => return None,
        }
    };
    let sessions = server()
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let username = sessions.values().find_map(|session| {
        let session = session.lock();
        (session.showcase && session.username.to_lowercase() == name)
            .then(|| session.username.clone())
    })?;
    Some(match kind {
        0 => Showcase::Profile(username),
        1 => Showcase::History,
        _ => Showcase::Games(username),
    })
}
