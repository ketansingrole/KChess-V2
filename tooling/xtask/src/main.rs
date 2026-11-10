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
mod mutants;
mod package;
// The pins it checks live with the Linux packaging, compiled on Unix.
#[cfg(unix)]
mod pins;
mod release;
mod store_notes;
mod sums;
mod sweep;
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
  check [--without-app-tests]
                        Everything a push to main must pass: format, lints, notices, release
                        metadata and the Rust tests (under the coverage gate when cargo-llvm-cov
                        is installed)
  check-fast            Format, lints and the tests of the packages changed since main
  ci-times [--days <n>]  How long CI took over the last n days (7), job by job, and the tests
                        that failed and then passed on the same commit, as Markdown (the
                        weekly CI times workflow)
  sweep                 Delete the build's object files no program uses (macOS; `check`,
                        `check-fast` and `app` do it first). CARGO_TARGET_DIR picks the build
  mutants [--plan] [--base <ref>] [-- <cargo-mutants arguments>]
                        Mutation testing of the lines changed since main (or <ref>); fails
                        when a mutant survives (docs/mutation-testing.md); with --plan, only
                        count them and print CI's shards
  pre-push              The Git pre-push hook (.githooks/pre-push): `check` before a push to
                        main, the new lines' mutants before a branch push
  install-hooks         Point Git at .githooks
  release-version       Set the next YEAR.MONTH.COUNTER release in Cargo.toml (fetches tags)
  release-verify        Check the release version (and, on a tag build, the tag), and that CI
                        ran and passed every job for HEAD
  attach-draft-release <dir>
                        Attach a tag build's packages to its draft release (CI)
  update-key <secret file>
                        Make the release key: the secret to <secret file>, the public half
                        to apps/gpui/update-key.pub
  sums <dir>            Write <dir>/SHA256SUMS.txt for every other file in <dir> (release CI)
  store-notes <in> <out>
                        Write release notes <in> as the plain text App Store Connect takes
                        (App Store CI)
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
                        (target/fairy-stockfish/bin); print its path
  engines [--github-env] <stockfish folder>
                        Fetch Stockfish into the folder and build Fairy-Stockfish; print
                        KCHESS_ENGINE_PATH and KCHESS_VARIANT_ENGINE_PATH, or add them to
                        $GITHUB_ENV (CI)
  package [--skip-build] [--snap|--mas]
                        Package the app for this machine (apps/gpui/dist): .dmg and .zip,
                        the NSIS installer, or the AppImage and .deb; --snap makes the Snap,
                        --mas the Mac App Store's .pkg (apps/gpui/dist/mas)
  deltas [dir]          Write the update patches to this release's packages in [dir]
                        (apps/gpui/dist) from the latest published releases (release CI)
  deltas --from <old package> <new package> [--stage <out>]
                        Write the one patch between two packages, beside the new one;
                        --stage then stages a Windows patch over the old installer's files
                        as --apply does (the Windows upgrade test)
  deltas --apply <patch> <installed folder> <out>
                        Stage a Windows patch as the updater does: <out>/files and
                        <out>/KChess-update.exe
  pins --check [--report <file>]
                        Fail when appimagetool or the AppImage runtime has a newer release than
                        its pin (package/linux.rs), naming both; the weekly Pins workflow
                        runs it (Unix)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("app") => {
            sweep::objects();
            app::run(rest)
        }
        Some("bench") => bench::run(rest),
        Some("check") => check::check(rest),
        Some("check-fast") => check::fast(),
        Some("ci-times") => ci_times::run(rest),
        Some("sweep") => {
            sweep::objects();
            Ok(())
        }
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
        Some("sums") => match rest.first() {
            Some(dir) => sums::run(std::path::Path::new(dir)),
            None => Err("sums needs the folder of the release's files.".into()),
        },
        Some("store-notes") => store_notes::run(rest),
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
        Some("fairy-stockfish") => fairy::build().map(|path| println!("{}", path.display())),
        Some("engines") => engines::run(rest),
        Some("package") => package::run(rest),
        Some("deltas") => deltas::run(rest),
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
