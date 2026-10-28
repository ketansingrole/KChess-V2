//! `cargo xtask <command>`: KChess's own tooling, in Rust like the rest of the repository.

mod check;
mod icons;
mod licenses;
mod package;
mod release;
mod util;
mod voice;

use std::process::ExitCode;

const HELP: &str = "cargo xtask <command>

  check                 Everything a push to main must pass: format, lints, notices, release
                        metadata and the Rust tests (under the coverage gate when cargo-llvm-cov
                        is installed)
  check-fast            Format, lints and the tests of the packages changed since main
  pre-push              The Git pre-push hook (.githooks/pre-push)
  install-hooks         Point Git at .githooks
  release-version       Set the next YEAR.MONTH.COUNTER release in Cargo.toml (fetches tags)
  release-verify        Check the release version (and, on a tag build, the tag)
  attach-draft-release <dir>
                        Attach a tag build's packages to its draft release (CI)
  download-release-snap Fetch and verify the published release's Snap (CI)
  notices               Check THIRD_PARTY_NOTICES.md names every shipped dependency
  icons [--check]       Write the Lucide icons the app names (or check they match)
  licenses <file>       Write the license texts of every crate compiled into the app
  voice-runtime [profile]  Stage the speech helper with libvosk (target/voice-runtime)
  package [--final] [--skip-build] [--snap]
                        Package the app for this machine (apps/gpui/dist): .dmg and .zip,
                        the NSIS installer, or the AppImage and .deb; --snap makes the Snap
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("check") => check::full(),
        Some("check-fast") => check::fast(),
        Some("pre-push") => check::pre_push(),
        Some("install-hooks") => check::install_hooks(),
        Some("release-version") => release::next(),
        Some("release-verify") => release::verify(),
        Some("attach-draft-release") => release::attach_draft(std::path::Path::new(
            rest.first().map_or("packages", String::as_str),
        )),
        Some("download-release-snap") => release::download_snap(),
        Some("notices") => licenses::check_notices(),
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
