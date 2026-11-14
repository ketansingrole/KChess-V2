//! The engines development, CI and packaging run: the Stockfish KChess ships, fetched by
//! kchess-core's own installer (its `fetch_stockfish` example), and Fairy-Stockfish, built from
//! its pinned source (`fairy`). `cargo xtask engines` gives both to CI's tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{Result, root, say};

/// The shipped Stockfish's program in `dir` when the folder already holds the pinned release
/// (the core's installer records its tag in `VERSION`).
fn fetched(dir: &Path) -> Option<PathBuf> {
    let program = dir.join(kchess_delta::installed::program(
        kchess_delta::installed::STOCKFISH,
    ));
    let version = std::fs::read_to_string(dir.join("VERSION")).ok()?;
    (program.is_file() && version.trim() == crate::pinned::stockfish_tag()).then_some(program)
}

/// The shipped Stockfish in `dir`, fetched there unless it already is; its path. Fetching builds
/// the core's installer, so a folder that holds the release is answered without it (CI's kept
/// engines). `release` builds the fetcher as packaging builds everything else, so it reuses that
/// build.
pub fn stockfish(dir: &Path, release: bool) -> Result<PathBuf> {
    if let Some(program) = fetched(dir) {
        return Ok(program);
    }
    say(&format!("Fetching Stockfish into {}…", dir.display()));
    let mut cargo = Command::new("cargo");
    cargo.arg("run");
    if release {
        cargo.arg("--release");
    }
    let out = cargo
        .args([
            "--locked",
            "-q",
            "-p",
            "kchess-core",
            "--example",
            "fetch_stockfish",
            "--",
        ])
        .arg(dir)
        .current_dir(root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|cause| format!("Could not fetch Stockfish: {cause}"))?;
    if !out.status.success() {
        return Err(format!("Fetching Stockfish failed ({}).", out.status));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn variables(stockfish: &Path, fairy: &Path) -> String {
    format!(
        "KCHESS_ENGINE_PATH={}\nKCHESS_VARIANT_ENGINE_PATH={}\n",
        stockfish.display(),
        fairy.display()
    )
}

/// `engines [--github-env] <stockfish folder>`: fetch Stockfish into the folder and build
/// Fairy-Stockfish, then print the variables that name them, or with `--github-env` add them to
/// the file `GITHUB_ENV` names, so the job's later steps have them.
pub fn run(args: &[String]) -> Result {
    let fetch = |dir: &Path| Ok((stockfish(dir, false)?, crate::fairy::build()?));
    if let Some(text) = run_with(args, fetch, std::env::var_os("GITHUB_ENV"))? {
        print!("{text}");
    }
    Ok(())
}

/// `run` with `fetch` making the engines for a Stockfish folder and `github_env` the job's
/// environment file; the text to print, when it goes to no file.
fn run_with(
    args: &[String],
    fetch: impl FnOnce(&Path) -> Result<(PathBuf, PathBuf)>,
    github_env: Option<std::ffi::OsString>,
) -> Result<Option<String>> {
    let to_file = args.iter().any(|arg| arg == "--github-env");
    let dir = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or("engines needs the folder to fetch Stockfish into.")?;
    let (stockfish, fairy) = fetch(Path::new(dir))?;
    let text = variables(&stockfish, &fairy);
    if !to_file {
        return Ok(Some(text));
    }
    let file = github_env.ok_or("--github-env needs GITHUB_ENV (CI).")?;
    append(Path::new(&file), &text)?;
    Ok(None)
}

fn append(file: &Path, text: &str) -> Result {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|cause| format!("{}: {cause}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder that holds the pinned release is used as it is; another release, or none, is
    /// fetched again.
    #[test]
    fn only_the_pinned_release_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(kchess_delta::installed::program(
            kchess_delta::installed::STOCKFISH,
        ));
        assert_eq!(fetched(dir.path()), None);
        std::fs::write(&program, "engine").unwrap();
        assert_eq!(fetched(dir.path()), None, "no VERSION");
        std::fs::write(dir.path().join("VERSION"), "sf_1\n").unwrap();
        assert_eq!(fetched(dir.path()), None, "another release");
        std::fs::write(
            dir.path().join("VERSION"),
            format!("{}\n", crate::pinned::stockfish_tag()),
        )
        .unwrap();
        assert_eq!(fetched(dir.path()), Some(program.clone()));
        assert_eq!(stockfish(dir.path(), false), Ok(program));
    }

    /// The job's environment file keeps what earlier steps wrote and gains both engines.
    #[test]
    fn the_engines_are_added_to_the_jobs_environment() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        std::fs::write(&file, "EARLIER=1\n").unwrap();
        let text = variables(
            Path::new("/sf/stockfish"),
            Path::new("/fsf/fairy-stockfish"),
        );
        append(&file, &text).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "EARLIER=1\nKCHESS_ENGINE_PATH=/sf/stockfish\nKCHESS_VARIANT_ENGINE_PATH=/fsf/fairy-stockfish\n"
        );
        assert!(run(&[]).unwrap_err().contains("folder"));
    }

    /// The engines are made for the folder named, whatever the flag's place, and their
    /// variables are printed, or with `--github-env` added to the job's file only.
    #[test]
    fn the_engines_go_to_the_terminal_or_the_jobs_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        let fetch = |folder: &Path| {
            assert_eq!(folder, Path::new("sf"));
            Ok((PathBuf::from("sf/stockfish"), PathBuf::from("fsf")))
        };
        let expected = "KCHESS_ENGINE_PATH=sf/stockfish\nKCHESS_VARIANT_ENGINE_PATH=fsf\n";
        assert_eq!(
            run_with(&args(&["sf"]), fetch, Some(file.clone().into())).unwrap(),
            Some(expected.to_string())
        );
        assert!(!file.exists());
        assert_eq!(
            run_with(
                &args(&["--github-env", "sf"]),
                fetch,
                Some(file.clone().into())
            )
            .unwrap(),
            None
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), expected);
        assert!(
            run_with(&args(&["sf", "--github-env"]), fetch, None)
                .unwrap_err()
                .contains("GITHUB_ENV")
        );
    }
}
