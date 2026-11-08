//! `cargo xtask <command>`: KChess's own tooling, in Rust like the rest of the repository.

mod app;
mod app_icon;
mod bench;
#[cfg(test)]
mod boundaries;
mod check;
mod electron_step;
mod fairy;
mod fake_lichess;
mod icons;
mod licenses;
mod mutants;
mod package;
// The pins it checks live with the Linux packaging, compiled on Unix.
#[cfg(unix)]
mod pins;
mod release;
mod update_key;
mod util;
mod voice;

use std::process::ExitCode;

const HELP: &str = "cargo xtask <command>

  app [--profile <dir>] [--watch] [--stop] [-- <app arguments>]
                        Build and start the app in the background on a scratch profile
                        (.data/gpui-profile) with a native Stockfish; print its pid. --watch
                        stays in the foreground and rebuilds and restarts it on every save,
                        back on the same page
  bench --build <label>=<binary> [--build …] [--runs 5] [--seconds 30] [--scenarios a,b]
        [--out <file>]  The app's frame benchmarks for builds made with
                        `cargo build --release -p kchess-gpui --features bench`, alternating
                        run by run on fresh profiles; prints each scenario's medians
  check                 Everything a push to main must pass: format, lints, notices, release
                        metadata and the Rust tests (under the coverage gate when cargo-llvm-cov
                        is installed)
  check-fast            Format, lints and the tests of the packages changed since main
  mutants [--base <ref>] [-- <cargo-mutants arguments>]
                        Mutation testing of the lines changed since main (or <ref>); fails
                        when a mutant survives (docs/mutation-testing.md)
  pre-push              The Git pre-push hook (.githooks/pre-push)
  install-hooks         Point Git at .githooks
  release-version       Set the next YEAR.MONTH.COUNTER release in Cargo.toml (fetches tags)
  release-verify        Check the release version (and, on a tag build, the tag), and that CI
                        ran and passed every job for HEAD
  attach-draft-release <dir>
                        Attach a tag build's packages to its draft release (CI)
  update-key <secret file>
                        Make the release key: the secret to <secret file>, the public half
                        to apps/gpui/update-key.pub
  sign-sums <file>      Sign a release's SHA256SUMS.txt into <file>.sig with
                        KCHESS_UPDATE_SIGNING_KEY (CI)
  download-release-snap Fetch and verify the published release's Snap (CI)
  electron-step [dir]   Check the frozen stepping-stone release's installers and write the
                        latest*.yml that send Electron installs to them (release CI)
  notices               Check THIRD_PARTY_NOTICES.md names every shipped dependency
  app-icon              Draw the app icon in every size and format the app needs
  icons [--check]       Write the icons the app names (or check they match)
  licenses <file>       Write the license texts of every crate compiled into the app
  voice-runtime [profile]  Stage the speech helper with libvosk (target/voice-runtime)
  fairy-stockfish       Build the pinned Fairy-Stockfish, the variant engine
                        (target/fairy-stockfish/bin)
  package [--skip-build] [--snap|--mas]
                        Package the app for this machine (apps/gpui/dist): .dmg and .zip,
                        the NSIS installer, or the AppImage and .deb; --snap makes the Snap,
                        --mas the Mac App Store's .pkg (apps/gpui/dist/mas)
  pins --check [--report <file>]
                        Fail when appimagetool or the AppImage runtime has a newer release than
                        its pin (package/linux.rs), naming both; the weekly Pins workflow
                        runs it (Unix)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("app") => app::run(rest),
        Some("bench") => bench::run(rest),
        Some("check") => check::full(),
        Some("check-fast") => check::fast(),
        Some("mutants") => mutants::run(rest),
        Some("pre-push") => check::pre_push(),
        Some("install-hooks") => check::install_hooks(),
        Some("release-version") => release::next(),
        Some("release-verify") => release::verify_release(),
        Some("attach-draft-release") => release::attach_draft(std::path::Path::new(
            rest.first().map_or("packages", String::as_str),
        )),
        Some("update-key") => match rest.first() {
            Some(path) => update_key::generate(std::path::Path::new(path)),
            None => Err("update-key needs the file to write the secret key to.".into()),
        },
        Some("sign-sums") => match rest.first() {
            Some(path) => update_key::sign(std::path::Path::new(path)),
            None => Err("sign-sums needs the checksums file.".into()),
        },
        Some("download-release-snap") => release::download_snap(),
        Some("electron-step") => electron_step::write(std::path::Path::new(
            rest.first().map_or("packages", String::as_str),
        )),
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
        Some("fairy-stockfish") => fairy::build().map(|_| ()),
        Some("package") => package::run(rest),
        #[cfg(unix)]
        Some("pins") => pins::run(rest),
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
