//! The helper as the core starts it, up to loading libvosk (which these tests do not ship).

use std::process::Command;

fn helper(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_kchess-voice"))
        .args(args)
        .output()
        .expect("ran");
    let stdout = String::from_utf8(output.stdout).expect("text");
    (output.status.code(), stdout)
}

#[test]
fn without_a_model_it_answers_its_usage_and_fails() {
    let (code, stdout) = helper(&[]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        "{\"error\":\"usage: kchess-voice <model folder> [--library <libvosk>]\"}\n"
    );
    assert_eq!(helper(&["model", "another"]).0, Some(1));
}

/// A library that cannot be loaded is reported on stdout, named as it was given.
#[test]
fn a_missing_library_is_reported_and_fails() {
    let (code, stdout) = helper(&["model", "--library", "/nonexistent/libvosk.so"]);
    assert_eq!(code, Some(1));
    let answer: serde_json::Value = serde_json::from_str(&stdout).expect("one JSON line");
    let error = answer["error"].as_str().expect("an error");
    assert!(
        error.starts_with("Could not load /nonexistent/libvosk.so"),
        "{error}"
    );
}
