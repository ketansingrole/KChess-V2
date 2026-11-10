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
        "sweep",
    ] {
        assert!(help.contains(command), "help misses {command}");
    }
}

#[test]
fn sweep_deletes_the_objects_of_programs_no_longer_built_on_macos_only() {
    let target = tempfile::tempdir().unwrap();
    // One in each profile's folder, and big enough to count in megabytes.
    let objects = ["debug", "llvm-cov-target/debug"].map(|profile| {
        let deps = target.path().join(profile).join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let object = deps.join("gone-0123456789abcdef.a1.x7.rcgu.o");
        std::fs::write(&object, vec![0; 1_500_000]).unwrap();
        object
    });
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("sweep")
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("xtask runs");
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    if cfg!(target_os = "macos") {
        assert!(objects.iter().all(|object| !object.exists()));
        assert!(
            said.contains("Deleted 2 object files no program uses (3 MB)."),
            "{said}"
        );
    } else {
        assert!(objects.iter().all(|object| object.exists()));
        assert!(said.is_empty(), "{said}");
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
