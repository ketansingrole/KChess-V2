//! `kchess-voice <model folder> [--library <libvosk>]`: offline speech recognition for the KChess
//! core, which starts this helper and talks to it over its pipes (`protocol.rs`). Running Vosk in
//! its own process keeps a crash in the native recognizer or a bad model file away from the core.
//! libvosk is loaded from beside this executable unless `--library` names it.

mod protocol;
mod vosk;

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use protocol::{Answer, Request};

#[cfg(target_os = "macos")]
const LIBRARY: &str = "libvosk.dylib";
#[cfg(target_os = "windows")]
const LIBRARY: &str = "libvosk.dll";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const LIBRARY: &str = "libvosk.so";

struct Args {
    model: PathBuf,
    library: PathBuf,
}

fn args() -> Result<Args, String> {
    let usage = || "usage: kchess-voice <model folder> [--library <libvosk>]".to_owned();
    let mut model = None;
    let mut library = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--library" {
            library = Some(PathBuf::from(args.next().ok_or_else(usage)?));
        } else if model.is_none() {
            model = Some(PathBuf::from(arg));
        } else {
            return Err(usage());
        }
    }
    let library = match library {
        Some(library) => library,
        None => std::env::current_exe()
            .map_err(|cause| format!("Could not locate the helper: {cause}"))?
            .with_file_name(LIBRARY),
    };
    Ok(Args {
        model: model.ok_or_else(usage)?,
        library,
    })
}

fn send(out: &mut impl Write, answer: &Answer) -> io::Result<()> {
    out.write_all(answer.line().as_bytes())?;
    out.flush()
}

fn serve(args: &Args, out: &mut impl Write) -> io::Result<ExitCode> {
    let vosk = match vosk::Vosk::load(&args.library) {
        Ok(vosk) => vosk,
        Err(message) => {
            send(out, &Answer::Error(message))?;
            return Ok(ExitCode::FAILURE);
        }
    };
    let model = match vosk.model(&args.model) {
        Ok(model) => model,
        Err(message) => {
            send(out, &Answer::Error(message))?;
            return Ok(ExitCode::FAILURE);
        }
    };
    send(out, &Answer::Ready(true))?;
    let mut recognizer = None;
    let mut input = io::stdin().lock();
    loop {
        let request = match protocol::read_request(&mut input) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(ExitCode::SUCCESS),
            // A broken frame leaves the stream unaligned: report it and stop.
            Err(cause) => {
                send(out, &Answer::Error(cause.to_string()))?;
                return Ok(ExitCode::FAILURE);
            }
        };
        let answer = match request {
            Request::Grammar(grammar) => {
                // The old recognizer goes first: one per process is enough.
                recognizer = None;
                let phrases = serde_json::to_string(&grammar.grammar).unwrap_or_default();
                match model.recognizer(grammar.sample_rate, &phrases) {
                    Ok(created) => {
                        recognizer = Some(created);
                        Answer::Ok(true)
                    }
                    Err(message) => Answer::Error(message),
                }
            }
            Request::Audio(samples) => match recognizer.as_mut() {
                None => Answer::Error("No grammar was set.".into()),
                Some(recognizer) => match recognizer.accept(&samples) {
                    Ok(true) => protocol::result(&recognizer.result()),
                    Ok(false) => protocol::partial(&recognizer.partial()),
                    Err(message) => Answer::Error(message),
                },
            },
            Request::Final => match recognizer.as_mut() {
                None => Answer::Error("No grammar was set.".into()),
                Some(recognizer) => protocol::result(&recognizer.finish()),
            },
        };
        send(out, &answer)?;
    }
}

fn main() -> ExitCode {
    let mut out = BufWriter::new(io::stdout().lock());
    let outcome = match args() {
        Ok(args) => serve(&args, &mut out),
        Err(message) => send(&mut out, &Answer::Error(message)).map(|()| ExitCode::FAILURE),
    };
    // A closed stdout means the core is gone; there is nobody left to tell.
    outcome.unwrap_or(ExitCode::FAILURE)
}
