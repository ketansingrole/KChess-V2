//! `cargo xtask <command>`: KChess's own tooling, in Rust like the rest of the repository.

mod app;
mod app_icon;
mod bench;
#[cfg(test)]
mod boundaries;
mod check;
mod ci_times;
mod deb;
mod deltas;
mod electron_step;
mod engines;
mod fairy;
mod fake_lichess;
mod hack;
mod icons;
mod licenses;
mod members;
mod mutants;
mod mutants_tree;
mod package;
mod pinned;
mod pins;
mod release;
mod store_notes;
mod sums;
mod superseded;
mod sweep;
mod update_key;
mod util;
mod voice;
#[cfg(test)]
mod workflows;

use std::path::Path;
use std::process::ExitCode;

type Run = fn(&[String]) -> util::Result;

/// One command: its name, each way to call it with what that does, and what runs it. The help
/// and the dispatch both read this table.
struct Command {
    name: &'static str,
    forms: &'static [(&'static str, &'static str)],
    run: Run,
}

fn first<'a>(rest: &'a [String], missing: &str) -> Result<&'a Path, String> {
    rest.first()
        .map(Path::new)
        .ok_or_else(|| missing.to_owned())
}

const COMMANDS: &[Command] = &[
    Command {
        name: "app",
        forms: &[(
            "[--profile <dir>] [--watch] [--stop] [-- <app arguments>]",
            "Build and start the app in the background on a scratch profile (.data/gpui-profile) \
             with a native Stockfish; print its pid. --watch stays in the foreground and rebuilds \
             and restarts it on every save, back on the same page",
        )],
        run: |rest| {
            sweep::build_folder(false);
            app::run(rest)
        },
    },
    Command {
        name: "bench",
        forms: &[(
            "--build <label>=<binary>… [--runs 5] [--seconds 30] [--scenarios a,b] [--out <file>]",
            "The app's frame benchmarks for builds made with \
             `cargo build --release -p kchess-gpui --features bench`, alternating run by run on \
             fresh profiles; prints each scenario's medians",
        )],
        run: bench::run,
    },
    Command {
        name: "check",
        forms: &[(
            "[--without-app-tests]",
            "Everything a push to main must pass: format, lints, notices, release metadata and \
             the Rust tests (under the coverage gate when cargo-llvm-cov is installed)",
        )],
        run: check::check,
    },
    Command {
        name: "check-fast",
        forms: &[(
            "",
            "Format, lints and the tests of the packages changed since main",
        )],
        run: |_| check::fast(),
    },
    Command {
        name: "scope",
        forms: &[(
            "<base>",
            "Print code= (more than documents changed) and packaging= (a file packages are made \
             from changed) for the commits since <base> (CI's changes job)",
        )],
        run: check::scope,
    },
    Command {
        name: "ci-times",
        forms: &[(
            "[--days <n>]",
            "How long CI took over the last n days (7), job by job, and the tests that failed and \
             then passed on the same commit, as Markdown (the weekly CI times workflow)",
        )],
        run: ci_times::run,
    },
    Command {
        name: "sweep",
        forms: &[(
            "[--dry-run]",
            "Delete what the build folder keeps of builds no current one uses, and on macOS the \
             object files no program uses (`check`, `check-fast` and `app` do it first), then \
             those of the checkout's mutants tree (the pre-push hook's, which sweeps it too); \
             --dry-run says how much would go. CARGO_TARGET_DIR picks the build",
        )],
        run: |rest| {
            sweep::build_folder(rest.iter().any(|arg| arg == "--dry-run"));
            Ok(())
        },
    },
    Command {
        name: "mutants",
        forms: &[(
            "[--plan|--warm] [--app] [--base <ref>|--all] [--in-place] [-- <cargo-mutants arguments>]",
            "Mutation testing of the lines changed since main (or <ref>), the app's left out \
             unless --app; fails when a mutant survives (docs/mutation-testing.md); with --plan, \
             only count them and print CI's shards; --warm builds what the mutants build on; --all \
             every file; --in-place mutates the checkout itself",
        )],
        run: mutants::run,
    },
    Command {
        name: "pre-push",
        forms: &[(
            "",
            "The Git pre-push hook (.githooks/pre-push): `check` before a push to main, the new \
             lines' mutants before a branch push",
        )],
        run: |_| check::pre_push(),
    },
    Command {
        name: "install-hooks",
        forms: &[("", "Point Git at .githooks")],
        run: |_| check::install_hooks(),
    },
    Command {
        name: "release-version",
        forms: &[(
            "",
            "Set the next YEAR.MONTH.COUNTER release in Cargo.toml (fetches tags)",
        )],
        run: |_| release::next(),
    },
    Command {
        name: "release-verify",
        forms: &[(
            "",
            "Check the release version (and, on a tag build, the tag), and that CI ran and passed \
             every job for HEAD",
        )],
        run: |_| release::verify_release(),
    },
    Command {
        name: "attach-draft-release",
        forms: &[(
            "<dir>",
            "Attach a tag build's packages to its draft release (CI)",
        )],
        run: |rest| {
            release::attach_draft(Path::new(rest.first().map_or("packages", String::as_str)))
        },
    },
    Command {
        name: "update-key",
        forms: &[(
            "<secret file>",
            "Make the release key: the secret to <secret file>, the public half to \
             apps/gpui/update-key.pub",
        )],
        run: |rest| {
            update_key::generate(first(
                rest,
                "update-key needs the file to write the secret key to.",
            )?)
        },
    },
    Command {
        name: "sums",
        forms: &[(
            "<dir>",
            "Write <dir>/SHA256SUMS.txt for every other file in <dir> (release CI)",
        )],
        run: |rest| {
            sums::run(first(
                rest,
                "sums needs the folder of the release's files.",
            )?)
        },
    },
    Command {
        name: "store-notes",
        forms: &[(
            "<in> <out>",
            "Write release notes <in> as the plain text App Store Connect takes (App Store CI)",
        )],
        run: store_notes::run,
    },
    Command {
        name: "sign-sums",
        forms: &[(
            "<file>",
            "Sign a release's SHA256SUMS.txt into SHA256SUMS.txt.sig beside it with \
             KCHESS_UPDATE_SIGNING_KEY (CI)",
        )],
        run: |rest| update_key::sign(first(rest, "sign-sums needs the checksums file.")?),
    },
    Command {
        name: "download-release-snap",
        forms: &[("", "Fetch and verify the published release's Snap (CI)")],
        run: |_| release::download_snap(),
    },
    Command {
        name: "electron-step",
        forms: &[(
            "[dir]",
            "Check the frozen stepping-stone release's installers and write the latest*.yml that \
             send Electron installs to them (release CI)",
        )],
        run: |rest| {
            electron_step::write(Path::new(rest.first().map_or("packages", String::as_str)))
        },
    },
    Command {
        name: "notices",
        forms: &[(
            "",
            "Check THIRD_PARTY_NOTICES.md names every shipped dependency",
        )],
        run: |_| licenses::check_notices(),
    },
    Command {
        name: "app-icon",
        forms: &[(
            "",
            "Draw the app icon in every size and format the app needs",
        )],
        run: |_| app_icon::write(),
    },
    Command {
        name: "icons",
        forms: &[(
            "[--check]",
            "Write the icons the app names (or check they match)",
        )],
        run: |rest| {
            if rest.iter().any(|arg| arg == "--check") {
                icons::check()
            } else {
                icons::write()
            }
        },
    },
    Command {
        name: "licenses",
        forms: &[(
            "<file>",
            "Write the license texts of every crate compiled into the app",
        )],
        run: |rest| licenses::write(first(rest, "licenses needs the file to write.")?),
    },
    Command {
        name: "voice-runtime",
        forms: &[(
            "[profile]",
            "Stage the speech helper with libvosk (target/voice-runtime)",
        )],
        run: |rest| voice::stage(rest.first().map_or("release", String::as_str)).map(|_| ()),
    },
    Command {
        name: "fairy-stockfish",
        forms: &[(
            "",
            "Build the pinned Fairy-Stockfish, the variant engine (target/fairy-stockfish/bin); \
             print its path",
        )],
        run: |_| fairy::build().map(|path| println!("{}", path.display())),
    },
    Command {
        name: "engines",
        forms: &[(
            "[--github-env] <stockfish folder>",
            "Fetch Stockfish into the folder and build Fairy-Stockfish, unless both are there; \
             print KCHESS_ENGINE_PATH and KCHESS_VARIANT_ENGINE_PATH, or add them to $GITHUB_ENV \
             (CI)",
        )],
        run: engines::run,
    },
    Command {
        name: "package",
        forms: &[(
            "[--skip-build|--build-only] [--snap|--mas]",
            "Package the app for this machine (apps/gpui/dist): .dmg and .zip, the NSIS \
             installer, or the AppImage and .deb; --snap adds the Snap, --mas the Mac App Store's \
             .pkg (apps/gpui/dist/mas); --build-only only compiles what packaging ships (CI's \
             package cache)",
        )],
        run: package::run,
    },
    Command {
        name: "deltas",
        forms: &[
            (
                "[dir]",
                "Write the update patches to this release's packages in [dir] (apps/gpui/dist) \
                 from the latest published releases (release CI)",
            ),
            (
                "--from <old package> <new package> [--stage <out>]",
                "Write the one patch between two packages, beside the new one; --stage then \
                 stages a Windows patch over the old installer's files as --apply does (the \
                 Windows upgrade test)",
            ),
            (
                "--apply <patch> <installed folder> <out>",
                "Stage a Windows patch with the updater's own code: <out>/files and the \
                 installer beside them",
            ),
        ],
        run: deltas::run,
    },
    Command {
        name: "pins",
        forms: &[(
            "--check [--report <file>]",
            "Fail when a pinned download (tooling/xtask/src/pinned.rs) has a newer release than its pin, naming \
             both; the weekly Pins workflow runs it",
        )],
        run: pins::run,
    },
];

const COLUMN: usize = 24;
const WIDTH: usize = 100;

fn help() -> String {
    let mut out = String::from("cargo xtask <command>\n\n");
    for command in COMMANDS {
        for (usage, about) in command.forms {
            let call = format!("  {} {usage}", command.name);
            let call = call.trim_end();
            let mut line = if call.len() < COLUMN {
                format!("{call:COLUMN$}")
            } else {
                out.push_str(call);
                out.push('\n');
                " ".repeat(COLUMN)
            };
            let mut empty = true;
            for word in about.split_whitespace() {
                if !empty && line.len() + 1 + word.len() > WIDTH {
                    out.push_str(&line);
                    out.push('\n');
                    line = " ".repeat(COLUMN);
                    empty = true;
                }
                if !empty {
                    line.push(' ');
                }
                line.push_str(word);
                empty = false;
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// The variables `cargo run` sets for xtask's own package (`CARGO_MANIFEST_DIR`, `CARGO_PKG_*`…),
/// which every Cargo xtask runs would inherit. ring's build script reruns when one changes, so a
/// build under xtask and a plain one (a CLI test's nested build of the scripted engine) rebuilt
/// ring, kchess-core and its binaries back and forth: the scripted engine kchess-core's tests
/// were running changed on disk under them, and the warm engine restarted for the new
/// executable (2026-11-11, `consecutive_moves_reuse_the_warm_engine_and_send_only_changed_options`
/// under `check-fast`). `CARGO`, the Cargo that ran xtask, stays.
fn is_package_variable(key: &str) -> bool {
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

/// Removes xtask's package variables (`is_package_variable`) from its environment, so what it runs
/// builds as a plain `cargo` does.
#[allow(unsafe_code)]
fn forget_package_variables() {
    let keys: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_str().is_some_and(is_package_variable))
        .collect();
    for key in keys {
        // SAFETY: `main` calls this first, before xtask starts any thread, so nothing else reads
        // or writes the environment meanwhile.
        unsafe { std::env::remove_var(key) };
    }
}

fn main() -> ExitCode {
    forget_package_variables();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            print!("{}", help());
            Ok(())
        }
        Some(name) => match COMMANDS.iter().find(|command| command.name == name) {
            Some(command) => (command.run)(rest),
            None => Err(format!("Unknown command {name}. Run `cargo xtask help`.")),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(cause) => {
            eprintln!("[xtask] {cause}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_shows_every_command_once_within_its_width() {
        let help = help();
        for command in COMMANDS {
            assert_eq!(
                COMMANDS
                    .iter()
                    .filter(|other| other.name == command.name)
                    .count(),
                1,
                "{} is in the table twice",
                command.name
            );
            assert!(
                help.lines()
                    .any(|line| line.starts_with(&format!("  {} ", command.name))
                        || line == format!("  {}", command.name)),
                "help misses {}",
                command.name
            );
        }
        assert!(help.lines().all(|line| line.len() <= WIDTH), "{help}");
    }

    #[test]
    fn only_the_package_variables_of_cargo_run_are_forgotten() {
        for key in [
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION_MAJOR",
            "CARGO_BIN_EXE_xtask",
            "CARGO_CRATE_NAME",
            "CARGO_PRIMARY_PACKAGE",
            "CARGO_TARGET_TMPDIR",
            "CARGO_BIN_NAME",
            "OUT_DIR",
        ] {
            assert!(is_package_variable(key), "{key}");
        }
        for key in [
            "CARGO",
            "CARGO_HOME",
            "CARGO_TARGET_DIR",
            "CARGO_INCREMENTAL",
            "CARGO_BUILD_JOBS",
            "PATH",
        ] {
            assert!(!is_package_variable(key), "{key}");
        }
    }
}
