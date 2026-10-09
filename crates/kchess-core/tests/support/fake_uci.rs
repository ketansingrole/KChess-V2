//! A scripted UCI engine for the engine tests: `kchess-fake-uci <mode> [command-log]`.
//!
//! Modes mirror the TypeScript fakes in `core/tests/unit/uci-lifecycle.test.ts`:
//! - `handshake` (default): answers `uci` and `isready`, ignores everything else.
//! - `silent`: answers nothing.
//! - `no-readyok`: answers `uci` but never `isready`.
//! - `stop-ack`: a handshake engine that acknowledges `stop` with `bestmove e2e4`.
//! - `stop-ignore`: a handshake engine that ignores `stop`.
//! - `ignore-term`: a handshake engine that ignores SIGTERM, so only SIGKILL stops it.
//!
//! When a log path is given, every received line is appended to it before any reply, so a test
//! that has read a reply knows the command was received.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "handshake".to_string());
    let log = args.next();

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
            let mut out = std::io::stdout().lock();
            writeln!(out, "{reply}").expect("write a reply");
            out.flush().expect("flush a reply");
        }
    }
}
