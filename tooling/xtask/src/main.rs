//! `cargo xtask <command>`: KChess's own tooling, in Rust like the rest of the repository.

mod app;
mod app_icon;
mod bench;
mod check;
mod fake_lichess;
mod icons;
mod licenses;
mod package;
mod release;
mod util;
mod voice;

use std::process::ExitCode;

const HELP: &str = "cargo xtask <command>

  app [--profile <dir>] [--stop] [-- <app arguments>]
                        Build and start the app in the background on a scratch profile
                        (.data/gpui-profile) with a native Stockfish; print its pid
  bench --build <label>=<binary> [--build …] [--runs 5] [--seconds 30] [--scenarios a,b]
        [--out <file>]  The app's frame benchmarks for builds made with
                        `cargo build --release -p kchess-gpui --features bench`, alternating
                        run by run on fresh profiles; prints each scenario's medians
  check                 Everything a push to main must pass: format, lints, notices, release
                        metadata and the Rust tests (under the coverage gate when cargo-llvm-cov
                        is installed)
  check-fast            Format, lints and the tests of the packages changed since main
  pre-push              The Git pre-push hook (.githooks/pre-push)
  install-hooks         Point Git at .githooks
  release-version       Set the next YEAR.MONTH.COUNTER release in Cargo.toml (fetches tags)
  release-verify        Check the release version (and, on a tag build, the tag), and that CI
                        ran and passed every job for HEAD
  attach-draft-release <dir>
                        Attach a tag build's packages to its draft release (CI)
  download-release-snap Fetch and verify the published release's Snap (CI)
  notices               Check THIRD_PARTY_NOTICES.md names every shipped dependency
  app-icon              Draw the app icon in every size and format the app needs
  icons [--check]       Write the Lucide icons the app names (or check they match)
  licenses <file>       Write the license texts of every crate compiled into the app
  voice-runtime [profile]  Stage the speech helper with libvosk (target/voice-runtime)
  package [--skip-build] [--snap]
                        Package the app for this machine (apps/gpui/dist): .dmg and .zip,
                        the NSIS installer, or the AppImage and .deb; --snap makes the Snap
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("app") => app::run(rest),
        Some("bench") => bench::run(rest),
        Some("check") => check::full(),
        Some("check-fast") => check::fast(),
        Some("pre-push") => check::pre_push(),
        Some("install-hooks") => check::install_hooks(),
        Some("release-version") => release::next(),
        Some("release-verify") => release::verify_release(),
        Some("attach-draft-release") => release::attach_draft(std::path::Path::new(
            rest.first().map_or("packages", String::as_str),
        )),
        Some("download-release-snap") => release::download_snap(),
        Some("notices") => licenses::check_notices(),
        Some("app-icon") => app_icon::write(),
        Some("icons") if rest.iter().any(|arg| arg == "--check") => icons::check(),
        Some("icons") => icons::write(),
        Some("licenses") => match rest.first() {
            Some(path) => licenses::write(std::path::Path::new(path)),
            None => Err("licenses needs the file to write.".into()),
        },
        Some("voice-runtime") => {
            voice::stage(rest.first().map_or("release", String::as_str)).map(|_| ())
        }
        Some("package") => package::run(rest),
        None | Some("help" | "--help" | "-h") => {
            print!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!("Unknown command {other}. Run `cargo xtask help`.")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(cause) => {
            eprintln!("[xtask] {cause}");
            ExitCode::FAILURE
        }
    }
}
