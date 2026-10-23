//! Fetch the latest official Stockfish for this machine into a folder, verified exactly as the
//! app's own download is (`install_managed_engine`), and print the executable's path. Packaging
//! (`tooling/gpui-package.mjs`) ships it beside the GPUI app as its bundled engine. A folder
//! that already holds the latest release downloads nothing.
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
    match install_managed_engine(&Stderr, &ManagedLocation::new(dir)).await {
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
