//! The engines development, CI and packaging run: the Stockfish KChess ships, fetched by
//! kchess-core's own installer (its `fetch_stockfish` example), and Fairy-Stockfish, built from
//! its pinned source (`fairy`). `cargo xtask engines` gives both to CI's tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{Result, root, say};

/// The shipped Stockfish in `dir`, fetched there unless it already is; its path. `release` builds
/// the fetcher as packaging builds everything else, so it reuses that build.
pub fn stockfish(dir: &Path, release: bool) -> Result<PathBuf> {
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

/// The variables that hand the engines to the tests and to a development run.
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
    let github_env = args.iter().any(|arg| arg == "--github-env");
    let dir = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or("engines needs the folder to fetch Stockfish into.")?;
    let text = variables(&stockfish(Path::new(dir), false)?, &crate::fairy::build()?);
    if !github_env {
        print!("{text}");
        return Ok(());
    }
    let file = std::env::var_os("GITHUB_ENV").ok_or("--github-env needs GITHUB_ENV (CI).")?;
    append(Path::new(&file), &text)
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
}
