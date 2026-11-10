//! The command line as a person runs it: an embedded core over a scratch profile, a local game
//! played through stdin and saved to the library, and the errors of commands it cannot run.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

/// The CLI under test, with no engine from the environment: CI sets `KCHESS_ENGINE_PATH` to a
/// real Stockfish, which a test's `--engine` would not replace.
fn kchess_cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kchess-cli"));
    command
        .env_remove("KCHESS_ENGINE_PATH")
        .env_remove("KCHESS_VARIANT_ENGINE_PATH");
    command
}

/// Runs `kchess-cli --embedded --no-credentials --data-dir <profile> <args>` with `input` on stdin.
fn cli(profile: &std::path::Path, args: &[&str], input: &str) -> Output {
    let mut child = kchess_cli()
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

/// Naming an opening, describing a position and refusing an unknown command need no core: the
/// profile is left untouched.
#[test]
fn local_answers_open_no_core() {
    let profile = tempfile::tempdir().expect("profile");
    let opening = cli(
        profile.path(),
        &["opening", "1.", "e4", "e5", "2.", "Nf3", "Nc6", "3.", "Bb5"],
        "",
    );
    assert!(opening.status.success(), "{opening:?}");
    assert_eq!(lines(&opening)[0]["name"], "Ruy Lopez");
    let position = cli(profile.path(), &["position", "startpos"], "");
    assert!(position.status.success(), "{position:?}");
    assert_eq!(lines(&position)[0]["turn"], "white");
    let unknown = cli(profile.path(), &["bogus"], "");
    assert!(!unknown.status.success());
    let files: Vec<_> = std::fs::read_dir(profile.path())
        .expect("profile")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert!(files.is_empty(), "{files:?}");
}

/// A generic call that opens a sign-in page gets its page: the CLI answers the core's host
/// requests while it waits, as `login` does.
#[test]
fn a_generic_call_answers_the_sign_in_page() {
    use std::io::{BufRead, BufReader};
    let profile = tempfile::tempdir().expect("a profile");
    let mut child = kchess_cli()
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

/// A binary of another workspace package, built first: Cargo does nothing when it is fresh, and
/// a binary left over from before a change (a service of another protocol) is never run.
#[cfg(unix)]
fn workspace_binary(package: &str, name: &str) -> std::path::PathBuf {
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .expect("the test executable is in target/<profile>/deps");
    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "-q", "--locked", "-p", package, "--bin", name]);
    for key in cargo_test_vars() {
        build.env_remove(key);
    }
    // Into the target folder this test runs from, which `cargo llvm-cov` moves with a flag that
    // a nested build does not inherit.
    build
        .arg("--target-dir")
        .arg(target.parent().expect("the target folder"));
    if target.file_name().is_some_and(|name| name == "release") {
        build.arg("--release");
    }
    assert!(
        build.status().expect("cargo runs").success(),
        "{name} builds"
    );
    target.join(name)
}

/// The variables Cargo sets for this test's own package (`CARGO_MANIFEST_DIR`, `OUT_DIR`, `CARGO_PKG_*`…),
/// left out of a nested build: build scripts that watch one (ring's) would rebuild for it, and
/// the next outer build rebuild them back (2026-11-10: about 7 s for every test that built).
#[cfg(unix)]
fn cargo_test_vars() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_str().is_some_and(is_cargo_test_var))
        .collect()
}

#[cfg(unix)]
fn is_cargo_test_var(key: &str) -> bool {
    key.starts_with("CARGO_PKG_")
        || key.starts_with("CARGO_BIN_EXE_")
        || matches!(
            key,
            "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_CRATE_NAME"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_TARGET_TMPDIR"
                | "CARGO_BIN_NAME"
                | "OUT_DIR"
        )
}

/// A nested build leaves out only what Cargo set for this package: its own configuration
/// (`CARGO_HOME`, `CARGO_INCREMENTAL`, `CARGO_BUILD_*`) still applies.
#[cfg(unix)]
#[test]
fn a_nested_build_drops_only_this_packages_variables() {
    for key in [
        "CARGO_MANIFEST_DIR",
        "CARGO_MANIFEST_PATH",
        "CARGO_PKG_NAME",
        "CARGO_BIN_EXE_kchess-cli",
        "CARGO_CRATE_NAME",
        "CARGO_PRIMARY_PACKAGE",
        "CARGO_TARGET_TMPDIR",
        "CARGO_BIN_NAME",
        "OUT_DIR",
    ] {
        assert!(is_cargo_test_var(key), "{key}");
    }
    for key in [
        "CARGO",
        "CARGO_HOME",
        "CARGO_INCREMENTAL",
        "CARGO_BUILD_JOBS",
        "PATH",
    ] {
        assert!(!is_cargo_test_var(key), "{key}");
    }
    assert!(
        cargo_test_vars()
            .iter()
            .any(|key| key == "CARGO_MANIFEST_DIR")
    );
}

/// A stale binary on disk is rebuilt before use, not run as it is (2026-11-09: a service built
/// before a contracts change refused the CLI of the same release).
#[cfg(unix)]
#[test]
fn a_stale_workspace_binary_is_rebuilt_before_use() {
    use std::os::unix::fs::MetadataExt as _;
    let fresh = workspace_binary("kchess-core", "kchess-fake-uci");
    let stale = put_stale_copy(&fresh);
    let used = workspace_binary("kchess-core", "kchess-fake-uci");
    assert_eq!(used, fresh);
    assert_ne!(
        std::fs::metadata(&used).expect("the scripted engine").ino(),
        stale,
        "the stale binary was used as it was"
    );
}

/// Puts a file that is not Cargo's output in place of `binary`, and answers its inode. Tests in
/// other binaries run this one at once (kchess-core's run it as their engine), and the core
/// takes an executable of another length or time for another engine: it restarted a warm engine
/// between two moves of `consecutive_moves_reuse_the_warm_engine_and_send_only_changed_options`
/// (2026-11-11) when this put a longer copy here. So the copy is the same bytes and time, put in
/// place under another name first so nobody reads half of it, and renamed over the file rather
/// than written into it, which Cargo hard-links to its own copy.
#[cfg(unix)]
fn put_stale_copy(binary: &std::path::Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    let built = std::fs::metadata(binary).expect("the binary");
    let partial = binary.with_file_name(format!(
        "{}-stale-{}",
        binary.file_name().expect("a file").to_string_lossy(),
        std::process::id()
    ));
    std::fs::copy(binary, &partial).expect("stale copy");
    let copy = std::fs::File::options()
        .write(true)
        .open(&partial)
        .expect("the stale copy");
    copy.set_modified(built.modified().expect("modified"))
        .expect("the binary's time");
    drop(copy);
    let stale = std::fs::metadata(&partial).expect("the stale copy");
    assert_eq!(
        (stale.len(), stale.modified().ok()),
        (built.len(), built.modified().ok()),
        "the copy looks like the binary to the tests running it"
    );
    std::fs::rename(&partial, binary).expect("stale binary in place");
    stale.ino()
}

/// The scripted engine (`kchess-fake-uci`, built first) as the copy `engine-quick`: its
/// streaming `search` mode, with every timed search cut to a few milliseconds.
#[cfg(unix)]
fn scripted_engine() -> std::path::PathBuf {
    scripted_engine_in("quick")
}

/// The scripted engine in `mode` (`search`, `quick`, `review`), chosen by its copy's name.
#[cfg(unix)]
fn scripted_engine_in(mode: &str) -> std::path::PathBuf {
    let fake = workspace_binary("kchess-core", "kchess-fake-uci");
    let target = fake.parent().expect("target folder").to_path_buf();
    // A copy of this call's own, so tests running at once never share a half-written file.
    // One folder per call: the tests of this binary run at once, in one process.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let folder = target.join(format!("cli-engine-{mode}-{}-{call}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("engine folder");
    let copy = folder.join(format!("engine-{mode}"));
    std::fs::copy(&fake, &copy).expect("copy the scripted engine");
    copy
}

/// `analyze` streams the engine's lines until Ctrl-C, then stops the search and exits cleanly.
#[cfg(unix)]
#[test]
fn analyze_streams_until_interrupted() {
    use std::io::BufRead;
    let profile = tempfile::tempdir().expect("profile");
    let engine = scripted_engine();
    let mut child = kchess_cli()
        .args(["--embedded", "--no-credentials", "--data-dir"])
        .arg(profile.path())
        .arg("--engine")
        .arg(&engine)
        .args(["analyze", "startpos"])
        .env("KCHESS_USER_DATA_DIR", profile.path())
        .env_remove("KCHESS_SERVICE_BINARY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("kchess-cli starts");
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
    let mut first = String::new();
    stdout.read_line(&mut first).expect("a line");
    let update: Value = serde_json::from_str(&first).expect("an analysis update");
    assert_eq!(update["lines"][0]["pv"][0], "e2e4", "{update}");
    // Still streaming: the command waits for Ctrl-C.
    assert!(child.try_wait().expect("status").is_none());
    let interrupted = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(interrupted.success());
    let status = child.wait().expect("kchess-cli finishes");
    assert!(status.success(), "{status:?}");
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// Without `--embedded`, the CLI joins the profile's shared core, starting the service; with it,
/// the core runs in the CLI alone.
#[cfg(unix)]
#[test]
fn the_cli_shares_the_profiles_core_unless_embedded() {
    let service = workspace_binary("kchess-service", "kchess-service");
    let runtime = tempfile::tempdir().expect("runtime");
    let run = |profile: &std::path::Path, embedded: bool| {
        let mut command = kchess_cli();
        if embedded {
            command.arg("--embedded");
        }
        command
            .args(["--no-credentials", "--data-dir"])
            .arg(profile)
            .arg("library")
            .env("KCHESS_USER_DATA_DIR", profile)
            .env("KCHESS_SERVICE_BINARY", &service)
            .env("KCHESS_RUNTIME_DIR", runtime.path())
            .stdin(Stdio::null())
            .output()
            .expect("kchess-cli runs")
    };
    let embedded = tempfile::tempdir().expect("profile");
    assert!(run(embedded.path(), true).status.success());
    assert!(!embedded.path().join("core.json").exists(), "no service");

    let shared = tempfile::tempdir().expect("profile");
    let output = run(shared.path(), false);
    assert!(output.status.success(), "{output:?}");
    // The service outlives the command, until its idle period ends: stopped here instead.
    let info: Value = serde_json::from_slice(
        &std::fs::read(shared.path().join("core.json")).expect("the service runs"),
    )
    .expect("core.json");
    let pid = info["pid"].as_u64().expect("pid").to_string();
    assert!(
        Command::new("kill")
            .arg(&pid)
            .status()
            .expect("kill")
            .success()
    );
}

/// A computer game against the scripted engine, which always answers 1. e4: its PGN names who
/// played which side.
#[cfg(unix)]
#[test]
fn a_computer_game_names_its_players() {
    let engine = scripted_engine();
    let pgn = |color: &str| {
        let profile = tempfile::tempdir().expect("profile");
        let mut child = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile.path())
            .arg("--engine")
            .arg(&engine)
            .args(["play", color])
            .env("KCHESS_USER_DATA_DIR", profile.path())
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
            .write_all(b"pgn\nquit\n")
            .expect("input");
        let played = child.wait_with_output().expect("kchess-cli finishes");
        assert!(played.status.success(), "{played:?}");
        let stdout = String::from_utf8_lossy(&played.stdout).to_string();
        // The engine's moves are announced as text between the JSON lines.
        assert_eq!(
            stdout.contains("Stockfish: e4"),
            color == "black",
            "{stdout}"
        );
        let last = stdout.lines().last().expect("the PGN");
        let pgn: Value = serde_json::from_str(last).expect("a JSON line");
        pgn["pgn"].as_str().expect("text").to_string()
    };
    let black = pgn("black");
    for header in [
        "[Event \"Computer game (KChess)\"]",
        "[White \"Stockfish\"]",
        "[Black \"You\"]",
    ] {
        assert!(black.contains(header), "{header}: {black}");
    }
    assert!(black.contains("1. e4 *"), "{black}");
    let white = pgn("white");
    assert!(white.contains("[White \"You\"]"), "{white}");
    assert!(white.contains("[Black \"Stockfish\"]"), "{white}");
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// `play` against the scripted engine: it moves first for Black, a takeback and a resignation
/// are carried out, `new` starts again, and the game in progress is resumed next time, where it
/// keeps its colour.
#[cfg(unix)]
#[test]
fn a_computer_game_is_played_saved_and_resumed() {
    let engine = scripted_engine();
    let profile = tempfile::tempdir().expect("profile");
    let play = |args: &[&str], input: &str| {
        let mut child = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile.path())
            .arg("--engine")
            .arg(&engine)
            .args(args)
            .env("KCHESS_USER_DATA_DIR", profile.path())
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
        let output = child.wait_with_output().expect("kchess-cli finishes");
        let states: Vec<Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        (output, states)
    };
    let (played, states) = play(
        &["play", "black"],
        "e7e5\nundo\nresign\nstatus\nnew\nquit\n",
    );
    assert!(played.status.success(), "{played:?}");
    let moves: Vec<Value> = states.iter().map(|state| state["moves"].clone()).collect();
    assert_eq!(
        moves,
        [
            json!([]),
            json!(["e2e4", "e7e5"]),
            // Undo takes back the player's move.
            json!(["e2e4"]),
            json!(["e2e4"]),
            json!(["e2e4"]),
            // A new game, where the engine opens again.
            json!(["e2e4"]),
        ]
    );
    assert_eq!(states[3]["result"]["detail"], "You resigned");
    assert_eq!(states[4]["result"], states[3]["result"]);
    assert!(states[5]["result"].is_null());
    let stdout = String::from_utf8_lossy(&played.stdout);
    assert_eq!(stdout.matches("Stockfish: e4").count(), 2, "{stdout}");

    assert!(
        !String::from_utf8_lossy(&played.stderr).contains("Illegal move"),
        "{played:?}"
    );

    // Resumed with its move, at another level; Black's reply is the only new move, and a
    // move that is not legal is said to be.
    let (resumed, states) = play(&["play", "club"], "e7e5\ne1e8\nquit\n");
    assert!(resumed.status.success(), "{resumed:?}");
    assert_eq!(states[0]["moves"], json!(["e2e4"]));
    assert_eq!(states[1]["moves"], json!(["e2e4", "e7e5"]));
    let stderr = String::from_utf8_lossy(&resumed.stderr);
    assert_eq!(stderr.matches("Illegal move").count(), 1, "{stderr}");

    // The game in progress keeps its colour, and an unknown level is refused.
    for (args, message) in [
        (
            vec!["play", "white"],
            "A game is in progress: finish it, or type new, before choosing a colour.",
        ),
        (vec!["play", "99"], "Unknown level 99."),
    ] {
        let (refused, _) = play(&args, "quit\n");
        assert!(!refused.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    // The same colour carries on with the game.
    let (kept, states) = play(&["play", "black"], "quit\n");
    assert!(kept.status.success(), "{kept:?}");
    assert_eq!(states[0]["moves"], json!(["e2e4", "e7e5"]));
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// A board game's variant and start position are chosen only between games, and a resignation
/// on White's move is Black's win.
#[test]
fn a_board_game_chooses_its_variant_and_position_between_games() {
    let profile = tempfile::tempdir().expect("profile");
    let local = |args: &[&str], input: &str| {
        let mut command = vec!["local"];
        command.extend_from_slice(args);
        let output = cli(profile.path(), &command, input);
        let states = lines(&output);
        (output, states)
    };
    let (resigned, states) = local(&[], "resign\npgn\nquit\n");
    assert!(resigned.status.success(), "{resigned:?}");
    assert_eq!(states[1]["result"]["winner"], "black");
    let pgn = states[2]["pgn"].as_str().expect("the PGN");
    assert!(pgn.contains("[Result \"0-1\"]"), "{pgn}");

    // Over: a new game in the variant asked for.
    let (crazyhouse, states) = local(&["crazyhouse"], "e2e4\nquit\n");
    assert!(crazyhouse.status.success(), "{crazyhouse:?}");
    assert!(
        states[0]["fen"].as_str().expect("fen").contains("[]"),
        "{states:?}"
    );
    assert_eq!(states[1]["moves"], json!(["e2e4"]));

    // In progress: the same variant carries on; another, or a position, is refused.
    let (kept, states) = local(&["crazyhouse"], "quit\n");
    assert!(kept.status.success(), "{kept:?}");
    assert_eq!(states[0]["moves"], json!(["e2e4"]));
    let rook = "4k3/8/8/8/8/8/8/4K2R w K - 0 1";
    for args in [vec!["atomic"], vec!["--fen", rook]] {
        let (refused, _) = local(&args, "quit\n");
        assert!(!refused.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.contains("A game is in progress"),
            "{args:?}: {stderr}"
        );
    }

    // Finished, it starts from the position given, if it is one a game can start from.
    let (_, states) = local(&[], "resign\nquit\n");
    assert!(states[1]["result"].is_object());
    let (bad, _) = local(&["--fen", "8/8/8/8/8/8/k7/K7 w - - 0 1"], "quit\n");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Not a playable position"));
    let (from_fen, states) = local(&["--fen", rook], "quit\n");
    assert!(from_fen.status.success(), "{from_fen:?}");
    assert_eq!(states[0]["fen"], rook);
}

/// `review` reports each side's figures and the faulted moves, numbered as in the game: the
/// scripted engine scores White winning from the sixth move on, so Black's Nf6 is the blunder.
#[cfg(unix)]
#[test]
fn review_numbers_the_faults_as_the_game_does() {
    let engine = scripted_engine_in("review");
    let review = |game: &str| {
        let profile = tempfile::tempdir().expect("profile");
        let output = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile.path())
            .arg("--engine")
            .arg(&engine)
            .args(["review", game])
            .env("KCHESS_USER_DATA_DIR", profile.path())
            .env_remove("KCHESS_SERVICE_BINARY")
            .stdin(Stdio::null())
            .output()
            .expect("kchess-cli runs");
        assert!(output.status.success(), "{output:?}");
        lines(&output).pop().expect("the report")
    };
    let report = review("1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7#");
    assert_eq!(report["white"]["blunder"], 0);
    assert_eq!(report["black"]["blunder"], 1);
    assert_eq!(
        report["faults"],
        json!([{ "move": 3, "color": "black", "san": "Nf6", "judgment": "blunder" }])
    );
    // From a position at move 10, the same blunder is move 12's.
    let later = review(
        "[FEN \"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 10\"]\n\n\
         10. e4 e5 11. Bc4 Nc6 12. Qh5 Nf6 13. Qxf7#",
    );
    assert_eq!(later["faults"][0]["move"], 12, "{later}");
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// A local stand-in for lichess.org: answers every request with what `reply` gives for its path
/// (a content type and a body), and records the paths asked for.
struct Lichess {
    base: String,
    paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Lichess {
    fn serve(reply: impl Fn(&str) -> (&'static str, String) + Send + 'static) -> Lichess {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&paths);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().expect("stream"));
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
                    header.clear();
                }
                seen.lock().expect("paths").push(path.clone());
                let (kind, body) = reply(&path);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Lichess { base, paths }
    }

    /// `kchess-cli <args>` over a scratch profile, reaching this stand-in for Lichess.
    fn cli(&self, profile: &std::path::Path, args: &[&str], input: &str) -> Output {
        let mut child = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile)
            .args(args)
            .env("KCHESS_USER_DATA_DIR", profile)
            .env("KCHESS_STORE_DEBUG", "1")
            .env("KCHESS_TEST_LICHESS_BASE", &self.base)
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

    fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("paths").clone()
    }
}

/// `puzzle` solves Lichess's daily puzzle, or the one a link names, typed move by move.
#[test]
fn a_puzzle_is_solved_move_by_move() {
    let puzzle = json!({
        "game": { "pgn": "e4 e5 Nf3 Nc6 Bc4 Nf6" },
        "puzzle": {
            "id": "abc12",
            "rating": 1500,
            "solution": ["f3g5", "d7d5", "e4d5"],
            "themes": ["fork"],
            "initialPly": 5
        }
    });
    let lichess = Lichess::serve(move |_| ("application/json", puzzle.to_string()));
    let profile = tempfile::tempdir().expect("profile");
    let solved = lichess.cli(
        profile.path(),
        &["puzzle"],
        "hint\nf3g5\nd2d4\ne4d5\nquit\n",
    );
    assert!(solved.status.success(), "{solved:?}");
    let shown = lines(&solved);
    assert_eq!(shown[0]["puzzle"], "abc12");
    assert_eq!(shown[0]["toMove"], "white");
    assert_eq!(shown[1], json!({ "hint": "f3" }));
    assert_eq!(shown[2]["move"], "Ng5");
    assert_eq!(shown[2]["reply"], "d5");
    assert_eq!(shown[3], json!({ "move": "d4", "correct": false }));
    assert_eq!(shown[4]["solved"], true);

    let given_up = lichess.cli(
        profile.path(),
        &["puzzle", "https://lichess.org/training/abc12"],
        "solution\nquit\n",
    );
    assert!(given_up.status.success(), "{given_up:?}");
    assert_eq!(
        lines(&given_up)[1]["solution"],
        json!(["Ng5", "d5", "exd5"])
    );
    assert_eq!(lichess.paths(), ["/api/puzzle/daily", "/api/puzzle/abc12"]);
}

/// `watch` prints each position of the watched game as a JSON line until Ctrl-C.
#[cfg(unix)]
#[test]
fn watch_prints_the_game_until_interrupted() {
    use std::io::BufRead;
    let italian = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 0 1";
    let records = [
        json!({
            "id": "VuAqDosj",
            "variant": { "key": "standard", "name": "Standard" },
            "speed": "rapid", "rated": true, "source": "arena", "initialFen": italian,
            "players": {
                "white": { "user": { "name": "Thot_x_Thor" }, "rating": 2155 },
                "black": { "user": { "name": "stefanoschor" }, "rating": 1766 }
            }
        }),
        json!({ "fen": italian, "wc": 600, "bc": 600 }),
    ]
    .map(|record| format!("{record}\n"))
    .concat();
    let lichess = Lichess::serve(move |path| {
        if path == "/api/stream/game/VuAqDosj" {
            ("application/x-ndjson", records.clone())
        } else {
            ("application/json", "{}".into())
        }
    });
    let profile = tempfile::tempdir().expect("profile");
    let mut child = kchess_cli()
        .args(["--embedded", "--no-credentials", "--data-dir"])
        .arg(profile.path())
        .args(["watch", "https://lichess.org/VuAqDosj"])
        .env("KCHESS_USER_DATA_DIR", profile.path())
        .env("KCHESS_STORE_DEBUG", "1")
        .env("KCHESS_TEST_LICHESS_BASE", &lichess.base)
        .env_remove("KCHESS_SERVICE_BINARY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("kchess-cli starts");
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
    let mut first = String::new();
    stdout.read_line(&mut first).expect("a line");
    let frame: Value = serde_json::from_str(&first).expect("a frame");
    assert_eq!(frame["gameId"], "VuAqDosj", "{frame}");
    assert!(
        child.try_wait().expect("status").is_none(),
        "still watching"
    );
    let interrupted = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(interrupted.success());
    let status = child.wait().expect("kchess-cli finishes");
    assert!(status.success(), "{status:?}");
}

/// `pgn` prints a Lichess game's or a study chapter's PGN, and `review` reviews either.
#[cfg(unix)]
#[test]
fn linked_games_and_studies_are_read_from_lichess() {
    let game =
        "[Event \"Casual\"]\n[Result \"1-0\"]\n\n1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7# 1-0\n";
    let lichess = Lichess::serve(move |_| ("application/x-chess-pgn", game.to_string()));
    let profile = tempfile::tempdir().expect("profile");
    for link in [
        "https://lichess.org/abcdefgh",
        "https://lichess.org/study/abcdefgh/ijklmnop",
    ] {
        let printed = lichess.cli(profile.path(), &["pgn", link], "");
        assert!(printed.status.success(), "{link}: {printed:?}");
        let stdout = String::from_utf8_lossy(&printed.stdout);
        assert!(stdout.contains("Qxf7#"), "{link}: {stdout}");
    }
    let export = "/api/games/export/_ids?clocks=true&evals=false&opening=true";
    let chapter = "/api/study/abcdefgh/ijklmnop.pgn?comments=true&variations=true&clocks=false&orientation=true";
    assert_eq!(lichess.paths(), [export, chapter]);
    let engine = scripted_engine_in("review");
    for link in [
        "https://lichess.org/abcdefgh",
        "https://lichess.org/study/abcdefgh/ijklmnop",
    ] {
        let fresh = tempfile::tempdir().expect("profile");
        let reviewed = lichess.cli(
            fresh.path(),
            &["--engine", engine.to_str().expect("path"), "review", link],
            "",
        );
        assert!(reviewed.status.success(), "{link}: {reviewed:?}");
        assert_eq!(lines(&reviewed)[0]["faults"][0]["san"], "Nf6", "{link}");
    }
    assert_eq!(lichess.paths(), [export, chapter, export, chapter]);
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// The core's warnings reach stderr with their scope, beside the command's own error.
#[test]
fn the_cores_warnings_go_to_stderr() {
    let profile = tempfile::tempdir().expect("profile");
    let failed = cli(
        profile.path(),
        &["--engine", "/nonexistent/stockfish", "review", "1. e4 e5"],
        "",
    );
    assert!(!failed.status.success());
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("[review] Review failed:"), "{stderr}");
    assert!(
        stderr.contains("[cli] Command failed: The review failed"),
        "{stderr}"
    );
    assert!(failed.stdout.is_empty());
}

/// A board game enters Game history once it has a move, not before; a Chess960 game starts from
/// one of its own positions.
#[test]
fn game_history_keeps_played_games_only() {
    let profile = tempfile::tempdir().expect("profile");
    let games = || lines(&cli(profile.path(), &["library"], ""))[0]["games"].clone();
    assert!(cli(profile.path(), &["local"], "quit\n").status.success());
    assert_eq!(games(), json!([]));
    assert!(
        cli(profile.path(), &["local"], "e2e4\nquit\n")
            .status
            .success()
    );
    let kept = games();
    assert_eq!(kept.as_array().map(Vec::len), Some(1), "{kept}");
    assert_eq!(kept[0]["moves"], json!(["e2e4"]));
    // Dated by the wall clock, in milliseconds.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_millis() as f64;
    for field in ["startedAt", "updatedAt"] {
        let at = kept[0][field].as_f64().expect(field);
        assert!((now - at).abs() < 600_000.0, "{field}: {at} against {now}");
    }

    assert!(
        cli(profile.path(), &["local"], "resign\nquit\n")
            .status
            .success()
    );
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    // One position in 960 is the usual one: two draws in a row are not.
    let fens: Vec<Value> = (0..2)
        .map(|_| {
            let started = cli(profile.path(), &["local", "chess960"], "resign\nquit\n");
            lines(&started)[0]["fen"].clone()
        })
        .collect();
    assert!(fens.iter().any(|fen| fen != start), "{fens:?}");
}

/// `play random` picks either colour.
#[cfg(unix)]
#[test]
fn a_random_colour_is_either() {
    let engine = scripted_engine();
    let mut seen = std::collections::HashSet::new();
    // Both colours within 40 draws, unless the draw is broken (one in 2^39 otherwise).
    for _ in 0..40 {
        let profile = tempfile::tempdir().expect("profile");
        let output = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile.path())
            .arg("--engine")
            .arg(&engine)
            .args(["play", "random"])
            .env("KCHESS_USER_DATA_DIR", profile.path())
            .env_remove("KCHESS_SERVICE_BINARY")
            .stdin(Stdio::null())
            .output()
            .expect("kchess-cli runs");
        assert!(output.status.success(), "{output:?}");
        // The engine opens when the player is Black.
        seen.insert(String::from_utf8_lossy(&output.stdout).contains("Stockfish: e4"));
        if seen.len() == 2 {
            break;
        }
    }
    assert_eq!(seen.len(), 2, "only one colour was drawn");
    let _ = std::fs::remove_dir_all(engine.parent().expect("engine folder"));
}

/// A timed board game the app saved resumes paused; once resumed, the side to move loses on
/// time when its clock runs out.
#[test]
fn a_resumed_clock_runs_out() {
    let profile = tempfile::tempdir().expect("profile");
    let session = json!(["local", {
        "setup": {
            "variant": "standard",
            "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        },
        "moves": ["e2e4"],
        "clock": {
            "white": { "minutes": 1, "increment": 0 },
            "black": { "minutes": 1, "increment": 0 }
        },
        "times": { "white": 60000, "black": 300 }
    }]);
    let saved = cli(
        profile.path(),
        &["call", "saveSession", &session.to_string()],
        "",
    );
    assert!(saved.status.success(), "{saved:?}");
    let mut child = kchess_cli()
        .args(["--embedded", "--no-credentials", "--data-dir"])
        .arg(profile.path())
        .arg("local")
        .env("KCHESS_USER_DATA_DIR", profile.path())
        .env_remove("KCHESS_SERVICE_BINARY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kchess-cli starts");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"pause\n").expect("resume");
    stdin.flush().expect("flush");
    // Black's 0.3 s run out well within this.
    std::thread::sleep(std::time::Duration::from_secs(2));
    stdin.write_all(b"quit\n").expect("quit");
    drop(stdin);
    let played = child.wait_with_output().expect("kchess-cli finishes");
    assert!(played.status.success(), "{played:?}");
    let states = lines(&played);
    assert!(states[1]["result"].is_null(), "{states:?}");
    assert_eq!(
        states.last().expect("a state")["result"],
        json!({ "winner": "white", "reason": "Time out" })
    );
}

/// `play` needs Stockfish for a standard game, and says so when it cannot start.
#[test]
fn play_needs_an_engine() {
    // No engine the person installed either: their home is the scratch profile.
    for args in [&["play"][..], &["play", "standard"]] {
        let profile = tempfile::tempdir().expect("profile");
        let refused = kchess_cli()
            .args(["--embedded", "--no-credentials", "--data-dir"])
            .arg(profile.path())
            .args(["--engine", "/nonexistent/stockfish"])
            .args(args)
            .env("KCHESS_USER_DATA_DIR", profile.path())
            .env("HOME", profile.path())
            .env("USERPROFILE", profile.path())
            .env_remove("KCHESS_SERVICE_BINARY")
            .stdin(Stdio::null())
            .output()
            .expect("kchess-cli runs");
        assert!(!refused.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.contains("Stockfish is unavailable. Install an engine or pass --engine."),
            "{args:?}: {stderr}"
        );
    }
}
