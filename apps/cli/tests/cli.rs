//! The command line as a person runs it: an embedded core over a scratch profile, a local game
//! played through stdin and saved to the library, and the errors of commands it cannot run.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

/// Runs `kchess-cli --embedded --no-credentials --data-dir <profile> <args>` with `input` on stdin.
fn cli(profile: &std::path::Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kchess-cli"))
        .args(["--embedded", "--no-credentials", "--data-dir"])
        .arg(profile)
        .args(args)
        // Never the person's own profile or the shared service.
        .env("KCHESS_USER_DATA_DIR", profile)
        .env_remove("KCHESS_SERVICE_BINARY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kchess-cli starts");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("input");
    child.wait_with_output().expect("kchess-cli finishes")
}

fn lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

#[test]
fn a_local_game_is_played_through_stdin_and_kept_in_the_library() {
    let profile = tempfile::tempdir().expect("profile");
    let played = cli(
        profile.path(),
        &["local"],
        "e2e4\nzzzz\ne7e5\n\nstatus\nundo\ndraw\nnew\nf2f4\nresign\nquit\n",
    );
    assert!(played.status.success(), "{played:?}");
    let states = lines(&played);
    let moves: Vec<Value> = states.iter().map(|state| state["moves"].clone()).collect();
    assert_eq!(
        moves,
        [
            json!([]),
            json!(["e2e4"]),
            // The illegal move changes nothing.
            json!(["e2e4"]),
            json!(["e2e4", "e7e5"]),
            json!(["e2e4", "e7e5"]),
            json!(["e2e4"]),
            json!(["e2e4"]),
            json!([]),
            json!(["f2f4"]),
            json!(["f2f4"]),
        ]
    );
    assert_eq!(states[6]["result"]["reason"], json!("Draw agreed"));
    assert_eq!(
        states[9]["result"],
        json!({ "winner": "white", "reason": "Black resigned" })
    );
    assert!(String::from_utf8_lossy(&played.stderr).contains("Illegal move or game unavailable."));

    // Both finished games are in the history, newest first.
    let library = cli(profile.path(), &["library"], "");
    assert!(library.status.success(), "{library:?}");
    let library = &lines(&library)[0];
    let games = library["games"].as_array().expect("games");
    assert_eq!(games.len(), 2);
    assert_eq!(
        (games[0]["result"].clone(), games[0]["moves"].clone()),
        (json!("1-0"), json!(["f2f4"]))
    );
    assert_eq!(games[1]["result"], json!("1/2-1/2"));
    assert!(games.iter().all(|game| game["source"] == json!("board")));

    // Any core method by name, with JSON arguments inline or from a file.
    let called = cli(profile.path(), &["call", "loadData"], "");
    assert!(called.status.success(), "{called:?}");
    assert!(lines(&called)[0].is_object());
    let arguments = profile.path().join("arguments.json");
    std::fs::write(&arguments, "[]").expect("arguments");
    let from_file = cli(
        profile.path(),
        &[
            "call",
            "library",
            "--args-file",
            &arguments.to_string_lossy(),
        ],
        "",
    );
    assert_eq!(
        lines(&from_file)[0]["games"].as_array().map(Vec::len),
        Some(2)
    );
    let status = cli(profile.path(), &["status"], "");
    assert!(status.status.success(), "{status:?}");
    assert!(lines(&status)[0].get("engine").is_some());
}

/// Moves are typed in SAN as readily as in UCI, read in the position on the board.
#[test]
fn moves_are_typed_in_san_or_uci() {
    let profile = tempfile::tempdir().expect("profile");
    let played = cli(
        profile.path(),
        &["local"],
        "Nf3\nd7d5\ng3\nNc6\nBg2\nBf5\nO-O\nNf3\npgn\nquit\n",
    );
    assert!(played.status.success(), "{played:?}");
    let mut states = lines(&played);
    // `pgn` prints the game so far, and nothing else.
    let pgn = states.pop().expect("the PGN");
    assert!(
        pgn["pgn"]
            .as_str()
            .is_some_and(|pgn| pgn.contains("1. Nf3 d5 2. g3 Nc6 3. Bg2 Bf5 4. O-O *")),
        "{pgn}"
    );
    let last = states.pop().expect("a state");
    assert_eq!(
        last["moves"],
        json!(["g1f3", "d7d5", "g2g3", "b8c6", "f1g2", "c8f5", "e1h1"])
    );
    // Black has no knight that reaches f3.
    assert!(String::from_utf8_lossy(&played.stderr).contains("Illegal move or game unavailable."));
}

#[test]
fn help_and_methods_need_no_core_and_bad_commands_fail() {
    let profile = tempfile::tempdir().expect("profile");
    let help = cli(profile.path(), &[], "");
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("KChess CLI"));
    let version = cli(profile.path(), &["--version"], "");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("kchess-cli {}", env!("CARGO_PKG_VERSION"))
    );
    let methods = cli(profile.path(), &["methods"], "");
    assert!(
        lines(&methods)[0]
            .as_array()
            .is_some_and(|names| names.contains(&json!("library")))
    );

    for (args, message) in [
        (vec!["bogus"], "Unknown command: bogus. Run --help."),
        (
            vec!["librar"],
            "Unknown command: librar. Did you mean library?",
        ),
        (
            vec!["positon"],
            "Unknown command: positon. Did you mean position?",
        ),
        (
            vec!["call", "nope"],
            "Unknown CoreApi method. Run methods to list them.",
        ),
        (
            vec!["call", "library", "{}"],
            "Arguments must be a JSON array.",
        ),
        (vec!["analyze"], "analyze requires a FEN or moves."),
        (
            vec!["analyze", "e4", "Ke7"],
            "Move 1 (Ke7) is not legal there.",
        ),
        (
            vec!["review"],
            "review requires a game: a Lichess link, a PGN file or moves.",
        ),
        (
            vec!["review", "1. e4 e5 2. Ke3"],
            "Move 2 (Ke3) is not legal there.",
        ),
        (
            vec!["pgn", "https://lichess.org/training/FPFg0"],
            "pgn takes a lichess.org game or study link.",
        ),
        (
            vec!["--browser", "never"],
            "--browser must be open or print.",
        ),
    ] {
        let failed = cli(profile.path(), &args, "");
        assert!(!failed.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&failed.stderr);
        assert!(
            stderr.contains(&format!("[cli] Command failed: {message}")),
            "{args:?}: {stderr}"
        );
    }
}

/// A generic call that opens a sign-in page gets its page: the CLI answers the core's host
/// requests while it waits, as `login` does.
#[test]
fn a_generic_call_answers_the_sign_in_page() {
    use std::io::{BufRead, BufReader};
    let profile = tempfile::tempdir().expect("a profile");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kchess-cli"))
        .args([
            "--embedded",
            "--no-credentials",
            "--browser",
            "print",
            "--data-dir",
        ])
        .arg(profile.path())
        .args(["call", "connectLichess", "[]"])
        .env("KCHESS_USER_DATA_DIR", profile.path())
        .env_remove("KCHESS_SERVICE_BINARY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kchess-cli starts");
    let stderr = BufReader::new(child.stderr.take().expect("stderr"));
    let (found, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if found.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut seen = Vec::new();
    let page = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) if line.starts_with("Open this page to continue: ") => break Some(line),
            Ok(line) => seen.push(line),
            Err(_) => break None,
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    let page = page.unwrap_or_else(|| panic!("no sign-in page was printed: {seen:?}"));
    assert!(page.contains("/oauth"), "{page}");
}
