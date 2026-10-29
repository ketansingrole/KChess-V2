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

#[test]
fn help_and_methods_need_no_core_and_bad_commands_fail() {
    let profile = tempfile::tempdir().expect("profile");
    let help = cli(profile.path(), &[], "");
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("KChess CLI"));
    let methods = cli(profile.path(), &["methods"], "");
    assert!(
        lines(&methods)[0]
            .as_array()
            .is_some_and(|names| names.contains(&json!("library")))
    );

    for (args, message) in [
        (vec!["bogus"], "Unknown command: bogus. Run --help."),
        (
            vec!["call", "nope"],
            "Unknown CoreApi method. Run methods to list them.",
        ),
        (
            vec!["call", "library", "{}"],
            "Arguments must be a JSON array.",
        ),
        (vec!["analyze"], "analyze requires a FEN."),
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
