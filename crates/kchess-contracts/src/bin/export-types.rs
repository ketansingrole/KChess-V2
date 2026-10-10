//! Writes `core/src/contracts/generated/*.ts` from the Rust contracts; `--check` only verifies.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().any(|arg| arg == "--check");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core/src/contracts/generated");
    let files = kchess_contracts::render_all();

    if check {
        let mut stale: Vec<String> = Vec::new();
        for (name, text) in &files {
            if fs::read_to_string(dir.join(name)).ok().as_deref() != Some(text.as_str()) {
                stale.push(name.clone());
            }
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".ts") && !files.contains_key(&name) {
                    stale.push(format!("{name} (no longer generated)"));
                }
            }
        }
        if stale.is_empty() {
            println!("contracts: {} generated files are up to date", files.len());
            return ExitCode::SUCCESS;
        }
        eprintln!("contracts: generated files are stale: {}", stale.join(", "));
        eprintln!("contracts: run `pnpm run build:native` to regenerate them");
        return ExitCode::FAILURE;
    }

    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("contracts: cannot create {}: {error}", dir.display());
        return ExitCode::FAILURE;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".ts") && !files.contains_key(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    for (name, text) in &files {
        if let Err(error) = fs::write(dir.join(name), text) {
            eprintln!("contracts: cannot write {name}: {error}");
            return ExitCode::FAILURE;
        }
    }
    println!(
        "contracts: wrote {} files to {}",
        files.len(),
        dir.display()
    );
    ExitCode::SUCCESS
}
