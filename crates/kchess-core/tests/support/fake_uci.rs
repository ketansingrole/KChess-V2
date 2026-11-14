//! A scripted UCI engine for the engine tests: `kchess-fake-uci <mode> [command-log]`.
//!
//! Handshake modes:
//! - `handshake` (default): answers `uci` and `isready`, ignores everything else.
//! - `silent`: answers nothing.
//! - `no-readyok`: answers `uci` but never `isready`.
//! - `stop-ack`: a handshake engine that acknowledges `stop` with `bestmove e2e4`.
//! - `stop-ignore`: a handshake engine that ignores `stop`.
//! - `ignore-term`: a handshake engine that ignores SIGTERM, so only SIGKILL stops it. It runs
//!   itself again through `sh` after `trap '' TERM`: an ignored signal stays ignored across
//!   `exec`, and the process (its PID and pipes) stays the same.
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
//! - `quick`: `search`, but `go movetime M` waits at most `QUICK_MOVETIME_MS`: for tests that need
//!   the computer's move, not its clock (a level's move takes a second or more).
//!
//! With no arguments, the name of a copy names the mode (`engine-search`), else `handshake`, and
//! the copy logs what it receives beside itself (`engine-search.log`).
//!
//! When a log path is given, every received line is appended to it before any reply, so a test
//! that has read a reply knows the command was received.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Set in the `ignore-term` engine once it runs with SIGTERM ignored.
#[cfg(unix)]
const TERM_IGNORED: &str = "KCHESS_FAKE_UCI_TERM_IGNORED";

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

/// Whether this is the `quick` engine.
static QUICK: AtomicBool = AtomicBool::new(false);

/// The longest a `quick` engine's `go movetime` waits.
const QUICK_MOVETIME_MS: u64 = 20;

/// How long a `go movetime` of `ms` waits: at most `QUICK_MOVETIME_MS` for a `quick` engine.
fn capped(ms: u64, quick: bool) -> u64 {
    if quick { ms.min(QUICK_MOVETIME_MS) } else { ms }
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
                Kind::MoveTime(capped(
                    number("movetime").unwrap_or(0),
                    QUICK.load(Ordering::SeqCst),
                ))
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
    // Started as an app starts its engine, with no arguments, the mode comes from a copy's name
    // (`engine-search`, `support::engine_copy` and the GPUI app's end-to-end tests).
    let named = args.next();
    let started_as_copy = named.is_none();
    let mode = named
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            let stem = exe.file_stem()?.to_str()?;
            Some(stem.strip_prefix("engine-")?.to_string())
        })
        .unwrap_or_else(|| "handshake".to_string());
    // `slow-review`: the review script, taking 20 ms per search so reviews overlap in tests.
    let slow = mode == "slow-review";
    let mode = if slow { "review".to_string() } else { mode };
    let mode = if mode == "quick" {
        QUICK.store(true, Ordering::SeqCst);
        "search".to_string()
    } else {
        mode
    };
    let log = args.next().or_else(|| {
        let exe = std::env::current_exe().ok()?;
        started_as_copy.then(|| exe.with_extension("log").display().to_string())
    });
    let searching = matches!(mode.as_str(), "search" | "review" | "stall");
    let state = Mutex::new(Search::default());

    #[cfg(unix)]
    if mode == "ignore-term" && std::env::var_os(TERM_IGNORED).is_none() {
        use std::os::unix::process::CommandExt as _;
        let exe = std::env::current_exe().expect("the fake engine's path");
        let mut again = std::process::Command::new("/bin/sh");
        again
            .args(["-c", "trap '' TERM; exec \"$0\" \"$@\""])
            .arg(exe)
            .arg(&mode)
            .args(&log)
            .env(TERM_IGNORED, "1");
        // Only returns when it failed.
        panic!("the fake engine could not run again: {}", again.exec());
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
            // One write per line: `writeln!` on an unbuffered file writes the line and its newline
            // separately, and an engine closing as the next starts wrote "stop" and "uci" as
            // "stopuci".
            file.write_all(format!("{line}\n").as_bytes())
                .expect("write the fake engine's command log");
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
        // `long-line`: a line past the controller's limit (a million bytes) before `uciok`.
        if mode == "long-line" && line == "uci" {
            emit(&"x".repeat(1_000_001));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_quick_engine_cuts_a_timed_search() {
        assert_eq!(capped(1_000, false), 1_000);
        assert_eq!(capped(1_000, true), QUICK_MOVETIME_MS);
        assert_eq!(capped(5, true), 5);
    }
}
