//! The checked-in `core/src/contracts/generated/*.ts` must be what the Rust contracts render.

use std::fs;
use std::path::PathBuf;

fn generated_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core/src/contracts/generated")
}

#[test]
fn generated_files_match_the_rust_contracts() {
    let files = kchess_contracts::render_all();
    let dir = generated_dir();
    for (name, text) in &files {
        let on_disk = fs::read_to_string(dir.join(name)).unwrap_or_else(|error| {
            panic!("{name} is missing ({error}); run pnpm run build:native")
        });
        assert!(
            on_disk == *text,
            "{name} is stale; run pnpm run build:native to regenerate it"
        );
    }
}

#[test]
fn no_generated_file_is_left_behind() {
    let files = kchess_contracts::render_all();
    let dir = generated_dir();
    for entry in fs::read_dir(&dir).expect("generated directory exists") {
        let name = entry
            .expect("readable entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.ends_with(".ts") {
            assert!(
                files.contains_key(&name),
                "{name} is no longer generated; remove it"
            );
        }
    }
}

#[test]
fn core_methods_are_the_api_methods_in_order() {
    let methods = kchess_contracts::core_api::methods();
    let names: Vec<&str> = methods.iter().map(|m| m.name).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "method names are unique");
    let core = &kchess_contracts::render_all()["core.ts"];
    for name in &names {
        assert!(
            core.contains(&format!("'{name}',")),
            "{name} is listed in CORE_METHODS"
        );
    }
}
