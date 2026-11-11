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

const MANIFEST: &str = "crates/kchess-hack/Cargo.toml";
const CONFIG: &str = ".config/hakari.toml";
const DEPENDABOT: &str = ".github/dependabot.yml";

/// Whether `check` reads `file`: the hack's settings, Dependabot's hold list, and the manifests
/// and lock it is written from.
pub fn reads(file: &str) -> bool {
    file == CONFIG || file == DEPENDABOT || crate::check::is_manifest(file)
}

/// How `.github/dependabot.yml` holds a package only the hack depends on to the hack's
/// requirement. Dependabot raises a requirement in whichever manifest names it, and hakari writes
/// the hack's from what the real crates resolve: it once raised the hack's zbus 5 to 6, which
/// nothing used, so the lockfile gained a second zbus and the hack no longer matched (2026-11-10).
/// Patch and security updates still come; a 0.x package's minor is its major.
const KEPT: &str = "update-types: [version-update:semver-major, version-update:semver-minor]";

/// The platforms the hack is written for: `platforms` in `.config/hakari.toml`.
fn platforms(config: &str) -> Vec<String> {
    config
        .lines()
        .skip_while(|line| !line.starts_with("platforms = ["))
        .skip(1)
        .take_while(|line| !line.starts_with(']'))
        .filter_map(|line| {
            line.trim()
                .trim_end_matches(',')
                .strip_prefix('"')?
                .strip_suffix('"')
        })
        .map(str::to_string)
        .collect()
}

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
    let read = |path: &str| {
        std::fs::read_to_string(crate::util::root().join(path))
            .map_err(|cause| format!("{path}: {cause}"))
    };
    let text = read(MANIFEST)?;
    let platforms = platforms(&read(CONFIG)?);
    if platforms.is_empty() {
        return Err(format!("{CONFIG} names no platforms."));
    }
    let members = crate::check::members()?;
    let mut headless = BTreeSet::new();
    for platform in &platforms {
        let mut args = vec!["tree", "--locked", "--prune", "kchess-hack"];
        for package in members
            .iter()
            .filter(|member| crate::members::headless(member))
        {
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
    if let extra @ [_, ..] = outside(&dependencies(&text), &headless).as_slice() {
        return Err(format!(
            "The hack makes the headless packages build {}: add {} to `[final-excludes]` in \
             .config/hakari.toml and run `cargo hakari generate`.",
            extra.join(", "),
            if extra.len() == 1 { "it" } else { "them" }
        ));
    }
    let metadata = output(
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
    )?;
    let metadata =
        serde_json::from_str(&metadata).map_err(|cause| format!("cargo metadata: {cause}"))?;
    unkept(&hack_only(&metadata), &kept(&read(DEPENDABOT)?))
}

/// The packages only the hack depends on, from `cargo metadata --no-deps`: no other manifest
/// names them, so Dependabot would raise the hack's requirement alone.
fn hack_only(metadata: &serde_json::Value) -> BTreeSet<String> {
    let (mut hack, mut real) = (BTreeSet::new(), BTreeSet::new());
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let names = if package["name"] == "kchess-hack" {
            &mut hack
        } else {
            &mut real
        };
        let dependencies = package["dependencies"].as_array().into_iter().flatten();
        names.extend(dependencies.filter_map(|dependency| dependency["name"].as_str()));
    }
    hack.difference(&real)
        .map(|name| name.to_string())
        .collect()
}

/// The packages `.github/dependabot.yml` holds to the hack's requirement, one line each.
fn kept(config: &str) -> BTreeSet<String> {
    let suffix = format!(", {KEPT} }}");
    config
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("- { dependency-name: ")?
                .strip_suffix(&suffix)
        })
        .map(str::to_string)
        .collect()
}

/// Whether Dependabot holds exactly the hack-only packages: a missing one can get the zbus raise,
/// a stale one no longer gets its updates in the manifest that names it now.
fn unkept(hack_only: &BTreeSet<String>, kept: &BTreeSet<String>) -> Result {
    let missing: Vec<_> = hack_only.difference(kept).collect();
    let stale: Vec<_> = kept.difference(hack_only).collect();
    if missing.is_empty() && stale.is_empty() {
        return Ok(());
    }
    let mut message = format!("{DEPENDABOT} must hold the packages only the hack depends on:");
    for name in missing {
        message.push_str(&format!(
            "\n  add `- {{ dependency-name: {name}, {KEPT} }}`"
        ));
    }
    for name in stale {
        message.push_str(&format!(
            "\n  remove {name}, which another package now depends on"
        ));
    }
    Err(message)
}

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
    fn only_packages_no_other_manifest_names_are_hack_only() {
        let metadata = serde_json::json!({ "packages": [
            { "name": "kchess-hack", "dependencies": [{ "name": "zbus" }, { "name": "tokio" }] },
            { "name": "kchess-core", "dependencies": [{ "name": "tokio" }, { "name": "serde" }] },
            { "name": "xtask", "dependencies": [] },
        ]});
        assert_eq!(hack_only(&metadata), set(&["zbus"]));
        assert_eq!(hack_only(&serde_json::json!({})), set(&[]));
    }

    #[test]
    fn dependabot_keeps_only_the_lines_holding_both_update_types() {
        let config = format!(
            "    ignore:\n      - dependency-name: gpui-kit\n      \
             - {{ dependency-name: zbus, {KEPT} }}\n      \
             - {{ dependency-name: log, update-types: [version-update:semver-major] }}\n"
        );
        assert_eq!(kept(&config), set(&["zbus"]));
    }

    #[test]
    fn a_missing_or_stale_package_is_refused_by_name() {
        assert_eq!(unkept(&set(&["zbus"]), &set(&["zbus"])), Ok(()));
        let error = unkept(&set(&["zbus"]), &set(&["log"])).unwrap_err();
        assert!(error.contains(&format!("add `- {{ dependency-name: zbus, {KEPT} }}`")));
        assert!(error.contains("remove log,"));
        assert!(unkept(&set(&["zbus"]), &set(&[])).is_err());
        assert!(unkept(&set(&[]), &set(&["log"])).is_err());
    }

    #[test]
    fn the_platforms_are_read_from_the_configuration() {
        let config = "resolver = \"2\"\nplatforms = [\n    \"aarch64-apple-darwin\",\n    \"x86_64-pc-windows-msvc\",\n]\n\n[final-excludes]\n";
        assert_eq!(
            platforms(config),
            ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
        );
        assert!(platforms("").is_empty());
    }

    #[test]
    fn a_tree_lists_each_package_once() {
        let tree =
            "kchess-core v2026.11.7 (/r/crates/kchess-core)\ntokio v1.54.1\n\ntokio v1.54.1\n";
        assert_eq!(packages(tree), set(&["kchess-core", "tokio"]));
    }
}
