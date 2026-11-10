//! The `cargo xtask` binary itself: its help and its refusal of an unknown command.

use std::process::Command;

fn xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .expect("xtask runs")
}

#[test]
fn help_names_the_commands() {
    let out = xtask(&["help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.starts_with("cargo xtask <command>"), "{help}");
    for command in [
        "check-fast",
        "ci-times [--days <n>]",
        "mutants [--plan] [--base <ref>]",
        "release-verify",
    ] {
        assert!(help.contains(command), "help misses {command}");
    }
}

#[test]
fn an_unknown_command_fails_and_says_why() {
    let out = xtask(&["no-such-command"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unknown command no-such-command"));
}

#[test]
fn ci_times_refuses_a_bad_number_of_days_before_asking_github() {
    let out = xtask(&["ci-times", "--days", "week"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--days needs a number of days, not week.")
    );
}
