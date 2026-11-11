//! The `cargo xtask` binary itself: its help, its refusal of an unknown command, `sweep` on a
//! scratch build folder and `ci-times`' refusal of a bad argument.

use std::process::Command;

fn xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .expect("xtask runs")
}

#[test]
fn help_is_printed_for_no_command_and_for_help() {
    for args in [&[][..], &["help"], &["--help"]] {
        let out = xtask(args);
        assert!(out.status.success());
        let help = String::from_utf8_lossy(&out.stdout);
        assert!(help.starts_with("cargo xtask <command>"), "{help}");
        assert!(help.contains("\n  check-fast "), "{help}");
    }
}

/// A library built twice into `profile`: the fingerprint of each build (Cargo's layout, its
/// fingerprint as little-endian hexadecimal) and its `.rlib`, the older build's 1.5 MB.
fn replaced_build(profile: &std::path::Path) -> std::path::PathBuf {
    let deps = profile.join("deps");
    std::fs::create_dir_all(&deps).unwrap();
    for (hash, own, seconds) in [
        ("0000000000000001", "0100000000000000", 0),
        ("0000000000000002", "0200000000000000", 10),
    ] {
        let folder = profile.join(".fingerprint").join(format!("old-{hash}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("lib-old.json"), "{\"deps\":[]}").unwrap();
        std::fs::write(folder.join("lib-old"), own).unwrap();
        let stamp = std::fs::File::create(folder.join("invoked.timestamp")).unwrap();
        stamp
            .set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_000_000 + seconds),
            )
            .unwrap();
    }
    std::fs::write(deps.join("libold-0000000000000002.rlib"), b"new").unwrap();
    let old = deps.join("libold-0000000000000001.rlib");
    std::fs::write(&old, vec![0; 1_500_000]).unwrap();
    old
}

#[test]
fn sweep_deletes_replaced_builds_and_on_macos_the_objects_no_program_uses() {
    let target = tempfile::tempdir().unwrap();
    // One in each profile's folder, and big enough to count in megabytes.
    let objects = ["debug", "llvm-cov-target/debug"].map(|profile| {
        let deps = target.path().join(profile).join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let object = deps.join("gone-0123456789abcdef.a1.x7.rcgu.o");
        std::fs::write(&object, vec![0; 1_500_000]).unwrap();
        object
    });
    let replaced = replaced_build(&target.path().join("debug"));
    let sweep = |dry_run: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
        command.arg("sweep").env("CARGO_TARGET_DIR", target.path());
        if dry_run {
            command.arg("--dry-run");
        }
        let out = command.output().expect("xtask runs");
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let said = sweep(true);
    assert!(
        said.contains("Would delete 1 build no current build uses (0.0 GB)."),
        "{said}"
    );
    assert!(replaced.exists() && objects.iter().all(|object| object.exists()));
    let said = sweep(false);
    assert!(!replaced.exists());
    assert!(
        said.contains("Deleted 1 build no current build uses (0.0 GB)."),
        "{said}"
    );
    if cfg!(target_os = "macos") {
        assert!(objects.iter().all(|object| !object.exists()));
        assert!(
            said.contains("Deleted 2 object files no program uses (3 MB)."),
            "{said}"
        );
    } else {
        assert!(objects.iter().all(|object| object.exists()));
        assert!(!said.contains("object file"), "{said}");
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
