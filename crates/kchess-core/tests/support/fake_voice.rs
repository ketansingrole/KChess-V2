//! A scripted `kchess-voice` helper for the recognizer tests: `kchess-fake-voice <model folder>`.
//! It speaks the helper's pipe protocol (`crates/kchess-voice/src/protocol.rs`) without Vosk.
//! The folder's `mode` file picks the behaviour, so parallel tests never share settings:
//! - `normal` (or no file): ready; a grammar is acknowledged; audio with any non-zero sample ends
//!   an utterance heard as "e four", silent audio is a partial "e"; the final result is empty.
//! - `refuse`: answers an error instead of ready, as a helper that cannot load the model does.
//! - `crash-on-audio`: exits without answering the first audio.
//! - `hang-on-audio`: never answers audio.
//!
//! Every request's tag is appended to the folder's `log` file before it is answered.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).expect("model folder"));
    let mode = std::fs::read_to_string(folder.join("mode")).unwrap_or_default();
    let mode = mode.trim();
    let mut out = std::io::stdout().lock();
    let mut say = |line: &str| {
        out.write_all(line.as_bytes()).expect("write");
        out.write_all(b"\n").expect("write");
        out.flush().expect("flush");
    };
    if mode == "refuse" {
        say(r#"{"error":"The offline voice model couldn’t load."}"#);
        std::process::exit(1);
    }
    say(r#"{"ready":true}"#);
    let mut input = std::io::stdin().lock();
    loop {
        let mut head = [0u8; 5];
        if input.read_exact(&mut head).is_err() {
            return;
        }
        let length = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
        let mut payload = vec![0u8; length];
        input.read_exact(&mut payload).expect("payload");
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join("log"))
            .expect("log");
        writeln!(log, "{}", head[0] as char).expect("log line");
        match head[0] {
            b'G' => say(r#"{"ok":true}"#),
            b'A' if mode == "crash-on-audio" => std::process::exit(3),
            b'A' if mode == "hang-on-audio" => loop {
                std::thread::park();
            },
            b'A' if payload.iter().any(|byte| *byte != 0) => say(
                r#"{"result":{"text":"e four","words":[{"word":"e","conf":0.9},{"word":"four","conf":0.8}]}}"#,
            ),
            b'A' => say(r#"{"partial":"e"}"#),
            b'F' => say(r#"{"result":{"text":"","words":[]}}"#),
            _ => say(r#"{"error":"unknown request"}"#),
        }
    }
}
