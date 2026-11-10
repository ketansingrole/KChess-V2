//! The workspace hack (`crates/kchess-hack`, `.config/hakari.toml`): every package depends on it,
//! so every build resolves the shared dependencies with one feature set.
//!
//! Without it Cargo unified features over the packages a command selected: `cargo test -p
//! kchess-core` built tokio without `signal`, `-p kchess-service` with it, and getrandom, serde and
//! 30 more differed against the whole workspace. Each feature set gave tokio and every crate above
//! it a new hash, so switching selection compiled the core and its tests again and `target/` kept
//! every copy (2026-11-10, one check-fast then `cargo test -p kchess-core`: 19 more units and
//! incremental caches, 2.0 GB, 65 s).

use std::collections::BTreeSet;

use crate::util::{Result, available, output, run, say};

/// The hack's manifest, from the repository root.
const MANIFEST: &str = "crates/kchess-hack/Cargo.toml";

/// The packages whose own dependencies may go in the hack: the headless ones, so that the hack
/// never makes them build the app's or xtask's dependencies.
const HEADLESS: [&str; 8] = [
    "kchess-domain",
    "kchess-contracts",
    "kchess-core",
    "kchess-games",
    "kchess-service",
    "kchess-cli",
    "kchess-voice",
    "kchess-delta",
];

/// The platforms the hack is written for (`.config/hakari.toml`).
const PLATFORMS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// The hack as `cargo hakari` writes it, and every package depending on it, when cargo-hakari is
/// installed (CI installs it); and the hack's dependencies all headless ones. Not `cargo hakari
/// verify`: it also wants one feature set for the app's own dependencies, which stay out.
pub fn check() -> Result {
    if available("cargo", &["hakari", "--version"]) {
        run("cargo", &["hakari", "generate", "--diff"])
            .and_then(|()| run("cargo", &["hakari", "manage-deps", "--dry-run"]))
            .map_err(|cause| {
                format!(
                    "{cause}\nThe hack is out of date: run `cargo hakari generate` and \
                     `cargo hakari manage-deps`."
                )
            })?;
    } else {
        say("cargo-hakari is not installed (`cargo install cargo-hakari`): CI checks the hack.");
    }
    let text = std::fs::read_to_string(crate::util::root().join(MANIFEST))
        .map_err(|cause| format!("{MANIFEST}: {cause}"))?;
    let mut headless = BTreeSet::new();
    for platform in PLATFORMS {
        let mut args = vec!["tree", "--locked", "--prune", "kchess-hack"];
        for package in HEADLESS {
            args.extend(["-p", package]);
        }
        args.extend([
            "-e",
            "normal,build,dev",
            "--target",
            platform,
            "--prefix",
            "none",
            "-f",
            "{p}",
        ]);
        headless.extend(packages(&output("cargo", &args)?));
    }
    match outside(&dependencies(&text), &headless).as_slice() {
        [] => Ok(()),
        extra => Err(format!(
            "The hack makes the headless packages build {}: add {} to `[final-excludes]` in \
             .config/hakari.toml and run `cargo hakari generate`.",
            extra.join(", "),
            if extra.len() == 1 { "it" } else { "them" }
        )),
    }
}

/// The package names `cargo tree --prefix none -f '{p}'` prints.
fn packages(tree: &str) -> BTreeSet<String> {
    tree.lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The packages the hack's manifest depends on, in its hakari section: each entry's key, or the
/// `package` it renames (`syn-7b89eefb6aaa9bf3 = { package = "syn", … }`).
fn dependencies(manifest: &str) -> BTreeSet<String> {
    manifest
        .lines()
        .skip_while(|line| !line.starts_with("### BEGIN HAKARI SECTION"))
        .take_while(|line| !line.starts_with("### END HAKARI SECTION"))
        .filter_map(|line| {
            let (key, value) = line.split_once(" = ")?;
            let renamed = value
                .split_once("package = \"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(name, _)| name);
            Some(renamed.unwrap_or(key.trim()).to_string())
        })
        .collect()
}

/// The hack's dependencies the headless packages do not build without it.
fn outside(hack: &BTreeSet<String>, headless: &BTreeSet<String>) -> Vec<String> {
    hack.difference(headless).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_hack_names_its_dependencies_and_what_they_rename() {
        let manifest = "[package]\nname = \"kchess-hack\"\n\n### BEGIN HAKARI SECTION\n\
                        [dependencies]\ntokio = { version = \"1\" }\n\
                        syn-7b89eefb6aaa9bf3 = { package = \"syn\", version = \"3\" }\n\n\
                        [target.x86_64-unknown-linux-gnu.dependencies]\n\
                        proc-macro2 = { version = \"1\" }\n### END HAKARI SECTION\n\
                        after = { version = \"1\" }\n";
        assert_eq!(
            dependencies(manifest),
            set(&["proc-macro2", "syn", "tokio"])
        );
        assert_eq!(dependencies("tokio = { version = \"1\" }\n"), set(&[]));
    }

    #[test]
    fn only_dependencies_the_headless_packages_lack_are_refused() {
        let hack = set(&["image", "serde", "tokio"]);
        assert_eq!(outside(&hack, &set(&["serde", "tokio", "zip"])), ["image"]);
        assert!(outside(&hack, &hack).is_empty());
    }

    #[test]
    fn a_tree_lists_each_package_once() {
        let tree =
            "kchess-core v2026.11.7 (/r/crates/kchess-core)\ntokio v1.54.1\n\ntokio v1.54.1\n";
        assert_eq!(packages(tree), set(&["kchess-core", "tokio"]));
    }

    #[test]
    fn the_checked_in_hack_holds_only_headless_dependencies() {
        check().unwrap();
    }
}
