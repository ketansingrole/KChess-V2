//! Fetch the latest official Stockfish for this machine into a folder, verified exactly as the
//! app's own download is (`install_managed_engine`), and print the executable's path. Packaging
//! (`tooling/gpui-package.mjs`) ships it beside the GPUI app as its bundled engine. A folder
//! that already holds the latest release downloads nothing. `GH_TOKEN` or `GITHUB_TOKEN`, when
//! set, authenticates the release check (CI runners share GitHub's anonymous rate limit).
//!
//! cargo run --release -p kchess-core --example fetch_stockfish -- <folder>

use std::path::PathBuf;
use std::process::ExitCode;

use kchess_core::engine::managed::{ManagedLocation, install_managed_engine};
use kchess_core::host::{Host, Level};
use serde_json::Value;

struct Stderr;

impl Host for Stderr {
    fn log(&self, level: Level, scope: &str, message: &str) {
        if level != Level::Debug {
            eprintln!("[{scope}] {message}");
        }
    }

    fn emit(&self, _: &str, _: Value) {}
}

#[tokio::main]
async fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: fetch_stockfish <folder>");
        return ExitCode::FAILURE;
    };
    let mut location = ManagedLocation::new(dir);
    location.github_token = ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|token| !token.is_empty()));
    match install_managed_engine(&Stderr, &location).await {
        Ok(result) => {
            eprintln!(
                "Stockfish {}{}",
                result.version,
                if result.updated {
                    " downloaded"
                } else {
                    " already there"
                }
            );
            println!("{}", result.path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Could not fetch Stockfish: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
