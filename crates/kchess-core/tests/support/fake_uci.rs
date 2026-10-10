//! A scripted UCI engine for the engine tests: `kchess-fake-uci <mode> [command-log]`.
//!
//! Handshake modes mirror the TypeScript fakes in `tests/core/uci-lifecycle.test.ts`:
//! - `handshake` (default): answers `uci` and `isready`, ignores everything else.
//! - `silent`: answers nothing.
//! - `no-readyok`: answers `uci` but never `isready`.
//! - `stop-ack`: a handshake engine that acknowledges `stop` with `bestmove e2e4`.
//! - `stop-ignore`: a handshake engine that ignores `stop`.
//! - `ignore-term`: a handshake engine that ignores SIGTERM, so only SIGKILL stops it.
//!
//! Search modes script a search for the engine services:
//! - `search`: `go infinite` streams scored `info` lines (up to `MultiPV` of them) until `stop`;
//!   `go depth D movetime M` answers at once; `go movetime M` answers after M ms or at `stop`;
//!   `go perft 1` lists the legal moves of the random-move test position (`h1h2`). Every search
//!   ends with `bestmove` (the first principal variation move), as Stockfish's does.
//! - `review`: like `search`, but the score follows the game: after six moves White is `+900`,
//!   before that `0`, from White's side. A review of the Scholar's mate game therefore sees
//!   a blunder at move 6 (Black's `Nf6`).
//! - `stall`: a deep search (depth 18 and up) reports one scored line and then waits for `stop`;
//!   shallower searches answer at once.
//!
//! When a log path is given, every received line is appended to it before any reply, so a test
//! that has read a reply knows the command was received.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How a `go` command runs.
enum Kind {
    Infinite,
    Depth { depth: u32 },
    MoveTime(u64),
}

/// The search state the engine remembers between commands.
#[derive(Default)]
struct Search {
    position: String,
    multipv: usize,
    stop: Option<Arc<AtomicBool>>,
}

fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}").expect("write a reply");
    out.flush().expect("flush a reply");
}

/// The score and principal variation the engine reports for `position`.
fn scripted(mode: &str, position: &str) -> (String, &'static str) {
    if mode == "review" {
        let count = position
            .split_once(" moves ")
            .map_or(0, |(_, moves)| moves.split_whitespace().count());
        let white = if count >= 6 { 900 } else { 0 };
        // The score is from the side to move: Black to move after an odd number of moves.
        let cp = if count.is_multiple_of(2) {
            white
        } else {
            -white
        };
        (format!("score cp {cp}"), "d8e7")
    } else if position.contains("R5K1 w") {
        ("score mate 1".to_string(), "a1a8")
    } else {
        ("score cp 31".to_string(), "e2e4 e7e5")
    }
}

fn info(mode: &str, position: &str, depth: u32, rank: usize) {
    let (score, pv) = scripted(mode, position);
    emit(&format!(
        "info depth {depth} seldepth {depth} multipv {rank} {score} nodes 9 pv {pv}"
    ));
}

fn run_search(mode: String, position: String, multipv: usize, stop: Arc<AtomicBool>, kind: Kind) {
    let stalls = mode == "stall";
    let best = scripted(&mode, &position)
        .1
        .split_whitespace()
        .next()
        .unwrap_or("0000")
        .to_string();
    match kind {
        Kind::Infinite => {
            let mut depth = 1;
            while !stop.load(Ordering::SeqCst) {
                for rank in 1..=multipv {
                    info(&mode, &position, depth, rank);
                }
                depth = (depth + 1).min(20);
                thread::sleep(Duration::from_millis(5));
            }
        }
        Kind::Depth { depth } => {
            // A stall holds only the deep searches (depth 18 and up), so a quick pass finishes.
            let stall_here = stalls && depth >= 18;
            info(&mode, &position, if stall_here { 1 } else { depth }, 1);
            if stall_here {
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
        Kind::MoveTime(ms) => {
            info(&mode, &position, 1, 1);
            let started = Instant::now();
            while !stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_millis(ms) {
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    emit(&format!("bestmove {best}"));
}

/// Answers one command in a search mode.
fn search_command(mode: &str, line: &str, state: &Mutex<Search>) {
    match line {
        "uci" => {
            emit("id name kchess-fake");
            emit("uciok");
        }
        "isready" => emit("readyok"),
        "stop" => {
            if let Some(flag) = &lock(state).stop {
                flag.store(true, Ordering::SeqCst);
            }
        }
        "go perft 1" => {
            emit("h1h2: 1");
            emit("Nodes searched: 1");
        }
        _ if line.starts_with("position ") => lock(state).position = line.to_string(),
        _ if line.starts_with("setoption name MultiPV value ") => {
            let value = line
                .rsplit(' ')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            lock(state).multipv = value;
        }
        _ if line.starts_with("go ") => {
            let words: Vec<&str> = line.split_whitespace().collect();
            let number = |name: &str| {
                words
                    .iter()
                    .position(|word| *word == name)
                    .and_then(|at| words.get(at + 1))
                    .and_then(|value| value.parse::<u64>().ok())
            };
            let kind = if words.contains(&"infinite") {
                Kind::Infinite
            } else if let Some(depth) = number("depth") {
                Kind::Depth {
                    depth: depth as u32,
                }
            } else {
                Kind::MoveTime(number("movetime").unwrap_or(0))
            };
            let stop = Arc::new(AtomicBool::new(false));
            let (position, multipv) = {
                let mut search = lock(state);
                search.stop = Some(Arc::clone(&stop));
                (search.position.clone(), search.multipv.max(1))
            };
            let mode = mode.to_string();
            thread::spawn(move || run_search(mode, position, multipv, stop, kind));
        }
        _ => {}
    }
}

fn lock(state: &Mutex<Search>) -> std::sync::MutexGuard<'_, Search> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "handshake".to_string());
    // `slow-review`: the review script, taking 20 ms per search so reviews overlap in tests.
    let slow = mode == "slow-review";
    let mode = if slow { "review".to_string() } else { mode };
    let log = args.next();
    let searching = matches!(mode.as_str(), "search" | "review" | "stall");
    let state = Mutex::new(Search {
        multipv: 1,
        ..Search::default()
    });

    #[cfg(unix)]
    if mode == "ignore-term" {
        // SAFETY: installing SIG_IGN has no preconditions; the disposition is this process's own.
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
    }

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Some(path) = &log {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("open the fake engine's command log");
            writeln!(file, "{line}").expect("write the fake engine's command log");
        }
        if line == "quit" {
            break;
        }
        if searching {
            if slow && line.starts_with("go ") {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            search_command(&mode, &line, &state);
            continue;
        }
        let reply = match (mode.as_str(), line.as_str()) {
            ("silent", _) => None,
            ("no-readyok", "uci") => Some("uciok"),
            ("no-readyok", _) => None,
            ("stop-ack", "stop") => Some("bestmove e2e4"),
            (_, "uci") => Some("uciok"),
            (_, "isready") => Some("readyok"),
            _ => None,
        };
        if let Some(reply) = reply {
            emit(reply);
        }
    }
}
