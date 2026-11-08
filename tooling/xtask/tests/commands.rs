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
    for command in ["check-fast", "mutants [--base <ref>]", "release-verify"] {
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
fn mutants_refuses_a_base_it_was_not_given() {
    // Without cargo-mutants it says how to install it; with it, that `--base` needs a value.
    let out = xtask(&["mutants", "--base"]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("--base needs a commit") || said.contains("cargo install cargo-mutants"),
        "{said}"
    );
}
