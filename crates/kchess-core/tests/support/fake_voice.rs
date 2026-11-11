//! A scripted `kchess-voice` helper for the recognizer tests: `kchess-fake-voice <model folder>`.
//! It speaks the helper's pipe protocol (`kchess_voice::protocol`) without Vosk.
//! The folder's `mode` file picks the behaviour, so parallel tests never share settings:
//! - `normal` (or no file): ready; a grammar is acknowledged; audio with any non-zero sample ends
//!   an utterance heard as "e four", silent audio is a partial "e"; the final result is empty.
//! - `refuse`: answers an error instead of ready, as a helper that cannot load the model does.
//! - `crash-on-audio`: exits without answering the first audio.
//! - `hang-on-audio`: never answers audio.
//! - `slow-audio`: as `normal`, but takes half a second to answer audio, so a test can give up
//!   on a request the helper has already been sent.
//! - `slow-grammar`: as `normal`, but takes two seconds to acknowledge a grammar, longer than
//!   the tests' per-request deadline, as a new helper on a loaded machine can.
//!
//! Every request's tag is appended to the folder's `log` file before it is answered. The file is
//! opened once, before `ready`: reopening it for each request put a file open (which antivirus
//! scanning can hold up on Windows) inside every request's deadline.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use kchess_voice::protocol::{Answer, Recognized, Request, Word, read_request};

fn heard(text: &str, words: &[(&str, f64)]) -> Answer {
    Answer::Result(Recognized {
        text: text.into(),
        words: words
            .iter()
            .map(|(word, conf)| Word {
                word: (*word).into(),
                conf: *conf,
            })
            .collect(),
    })
}

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).expect("model folder"));
    let mode = std::fs::read_to_string(folder.join("mode")).unwrap_or_default();
    let mode = mode.trim();
    let mut out = std::io::stdout().lock();
    let mut say = |answer: Answer| {
        out.write_all(answer.line().as_bytes()).expect("write");
        out.flush().expect("flush");
    };
    if mode == "refuse" {
        say(Answer::Error(
            "The offline voice model couldn’t load.".into(),
        ));
        std::process::exit(1);
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(folder.join("log"))
        .expect("log");
    say(Answer::Ready(true));
    let mut input = std::io::stdin().lock();
    while let Ok(Some(request)) = read_request(&mut input) {
        let tag = match &request {
            Request::Grammar(_) => 'G',
            Request::Audio(_) => 'A',
            Request::Final => 'F',
            Request::Invalid(_) => '?',
        };
        writeln!(log, "{tag}").expect("log line");
        match request {
            Request::Grammar(_) => {
                if mode == "slow-grammar" {
                    std::thread::sleep(Duration::from_secs(2));
                }
                say(Answer::Ok(true));
            }
            Request::Audio(_) if mode == "crash-on-audio" => std::process::exit(3),
            Request::Audio(_) if mode == "hang-on-audio" => loop {
                std::thread::park();
            },
            Request::Audio(samples) if mode == "slow-audio" => {
                std::thread::sleep(Duration::from_millis(500));
                if samples.iter().any(|sample| *sample != 0) {
                    say(heard("e four", &[]));
                } else {
                    say(Answer::Partial("e".into()));
                }
            }
            Request::Audio(samples) if samples.iter().any(|sample| *sample != 0) => {
                say(heard("e four", &[("e", 0.9), ("four", 0.8)]));
            }
            Request::Audio(_) => say(Answer::Partial("e".into())),
            Request::Final => say(heard("", &[])),
            Request::Invalid(message) => say(Answer::Error(message.into())),
        }
    }
}
