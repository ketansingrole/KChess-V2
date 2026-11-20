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

use crate::util::{Result, available, output, root, run, say};

const MANIFEST: &str = "crates/kchess-hack/Cargo.toml";
const CONFIG: &str = ".config/hakari.toml";
const DEPENDABOT: &str = ".github/dependabot.yml";
/// The fuzz targets' own workspace (`fuzz/`): it depends on `kchess-domain`, and so on the hack,
/// but Dependabot updates only the root's lock, so its lock missed each package the hack gained
/// (byteorder on 2026-11-18, socket2 on 11-20) and `check` refused it.
const FUZZ: &str = "fuzz/Cargo.toml";

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
                format!("{cause}\nThe hack is out of date: run `cargo xtask hack`.")
            })?;
    } else {
        say("cargo-hakari is not installed (`cargo install cargo-hakari`): CI checks the hack.");
    }
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
    let metadata = metadata()?;
    let source = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    if let unnamed @ [_, ..] = unnamed(&metadata, source).as_slice() {
        return Err(format!("{} {NAMED_HELP}", unnamed.join(", ")));
    }
    unkept(&hack_only(&metadata), &kept(&read(DEPENDABOT)?))
}

/// `hack`: writes everything `check` wants to follow the dependencies, so a crate update needs no
/// hand afterwards (`.github/workflows/dependabot-hack.yml` runs it on Dependabot's pull
/// requests): the hack and the lock (`cargo hakari generate`), each package's dependency on it
/// (`cargo hakari manage-deps`), Dependabot's hold list, and the fuzz targets' lock. Then checks
/// it all, since a dependency the headless packages lack needs a person's `[final-excludes]`.
pub fn write() -> Result {
    if !available("cargo", &["hakari", "--version"]) {
        return Err("`cargo xtask hack` needs cargo-hakari: `cargo install cargo-hakari`.".into());
    }
    run("cargo", &["hakari", "generate"])?;
    run("cargo", &["hakari", "manage-deps", "--yes"])?;
    let config = read(DEPENDABOT)?;
    let held = hold(&config, &hack_only(&metadata()?))?;
    if held != config {
        std::fs::write(root().join(DEPENDABOT), held)
            .map_err(|cause| format!("{DEPENDABOT}: {cause}"))?;
        say(&format!(
            "{DEPENDABOT} holds the packages only the hack depends on."
        ));
    }
    // Only what the manifests changed: the packages it already locks keep their versions.
    run("cargo", &["update", "--workspace", "--manifest-path", FUZZ])?;
    check()
}

fn read(path: &str) -> Result<String> {
    std::fs::read_to_string(root().join(path)).map_err(|cause| format!("{path}: {cause}"))
}

fn metadata() -> Result<serde_json::Value> {
    let metadata = output(
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
    )?;
    serde_json::from_str(&metadata).map_err(|cause| format!("cargo metadata: {cause}"))
}

/// How a package's code names the hack. Its code uses nothing of the hack, so without this line
/// Cargo's `unused_dependencies` lint (Rust 1.100) warns about the dependency every time it
/// compiles the package.
const NAMED: &str = "use kchess_hack as _;";
const NAMED_HELP: &str = "depend on the hack (`cargo xtask hack`) without naming it: add \
                          `use kchess_hack as _;` to the root of its library or program.";

/// The packages that depend on the hack but name it in none of their libraries' or programs'
/// roots (`src_path` in `cargo metadata --no-deps`), read by `read`.
fn unnamed(metadata: &serde_json::Value, read: impl Fn(&str) -> String) -> Vec<String> {
    let packages = metadata["packages"].as_array().into_iter().flatten();
    packages
        .filter(|package| {
            let mut dependencies = package["dependencies"].as_array().into_iter().flatten();
            dependencies.any(|dependency| dependency["name"] == "kchess-hack")
        })
        .filter(|package| {
            let targets = package["targets"].as_array().into_iter().flatten();
            !targets
                .filter(|target| {
                    let mut kinds = target["kind"].as_array().into_iter().flatten();
                    kinds.any(|kind| kind == "lib" || kind == "bin")
                })
                .filter_map(|target| target["src_path"].as_str())
                .any(|root| read(root).lines().any(|line| line.trim() == NAMED))
        })
        .filter_map(|package| package["name"].as_str().map(str::to_string))
        .collect()
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

/// `config` holding exactly `names`: its hold lines replaced by one per name, in order, where
/// the first of them was.
fn hold(config: &str, names: &BTreeSet<String>) -> Result<String> {
    let mut lines = Vec::new();
    let mut first = None;
    for line in config.lines() {
        let trimmed = line.trim_start();
        if kept(line).is_empty() {
            lines.push(line.to_string());
        } else if first.is_none() {
            first = Some((lines.len(), &line[..line.len() - trimmed.len()]));
        }
    }
    let Some((at, indent)) = first else {
        return Err(format!(
            "{DEPENDABOT} has no `- {{ dependency-name: <name>, {KEPT} }}` line to write the \
             hack's packages beside."
        ));
    };
    lines.splice(
        at..at,
        names
            .iter()
            .map(|name| format!("{indent}- {{ dependency-name: {name}, {KEPT} }}")),
    );
    let mut text = lines.join("\n");
    if config.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
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

    /// Rust 1.100's `unused_dependencies` lint warned about the hack in every package that
    /// compiled (2026-11-20): only a library's or program's root naming it counts as using it.
    #[test]
    fn a_package_depending_on_the_hack_names_it_in_a_library_or_program() {
        let package = |name: &str, dependency: &str, targets: &[(&str, &str)]| {
            let targets: Vec<_> = targets
                .iter()
                .map(|(kind, path)| serde_json::json!({ "kind": [kind], "src_path": path }))
                .collect();
            serde_json::json!({
                "name": name, "dependencies": [{ "name": dependency }], "targets": targets,
            })
        };
        let metadata = serde_json::json!({ "packages": [
            package("named-lib", "kchess-hack", &[("lib", "/named"), ("bin", "/bare")]),
            package("named-bin", "kchess-hack", &[("lib", "/bare"), ("bin", "/named")]),
            package("in-a-test", "kchess-hack", &[("lib", "/bare"), ("test", "/named")]),
            package("in-a-comment", "kchess-hack", &[("lib", "/commented")]),
            package("unnamed", "kchess-hack", &[("bin", "/bare")]),
            package("no-hack", "serde", &[("lib", "/bare")]),
        ]});
        let read = |path: &str| match path {
            "/named" => format!("//! Docs.\n\n    {NAMED}\nmod x;\n"),
            "/commented" => format!("// {NAMED}\n"),
            _ => "mod x;\n".to_string(),
        };
        assert_eq!(
            unnamed(&metadata, read),
            ["in-a-test", "in-a-comment", "unnamed"]
        );
        assert!(unnamed(&serde_json::json!({}), read).is_empty());
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
    fn the_hold_list_is_rewritten_in_place_with_exactly_the_names() {
        let line = |name: &str| format!("      - {{ dependency-name: {name}, {KEPT} }}\n");
        let config = format!(
            "    ignore:\n      - dependency-name: gpui-kit\n      # held\n{}{}    groups:\n",
            line("log"),
            line("zbus")
        );
        let held = hold(&config, &set(&["socket2", "zbus", "aho-corasick"])).unwrap();
        assert_eq!(
            held,
            format!(
                "    ignore:\n      - dependency-name: gpui-kit\n      # held\n{}{}{}    groups:\n",
                line("aho-corasick"),
                line("socket2"),
                line("zbus")
            )
        );
        assert_eq!(kept(&held), set(&["aho-corasick", "socket2", "zbus"]));
        // Already held: unchanged, to the byte; without a final newline, none is added.
        assert_eq!(hold(&held, &kept(&held)).unwrap(), held);
        assert_eq!(
            hold(line("log").trim_end(), &set(&["x"])).unwrap(),
            line("x").trim_end()
        );
        // Nothing held: an empty list in its place.
        assert_eq!(
            hold(&config, &set(&[])).unwrap(),
            "    ignore:\n      - dependency-name: gpui-kit\n      # held\n    groups:\n"
        );
        // No hold line to write beside.
        let error = hold("    ignore:\n", &set(&["zbus"])).unwrap_err();
        assert!(error.contains("no `- { dependency-name: <name>,"));
    }

    #[test]
    fn the_platforms_are_read_from_the_configuration() {
        let config = "resolver = \"2\"\nplatforms = [\n    \"aarch64-apple-darwin\",\n    \"x86_64-pc-windows-msvc\",\n]\n\n[final-excludes]\n";
        assert_eq!(
            platforms(config),
            ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
        );
        assert!(platforms("").is_empty());
        // Only the platforms' list is read, not a list before it.
        let config = "hakari-package = \"kchess-hack\"\nexcludes = [\n    \"x\",\n]\nplatforms = [\n    \"aarch64-apple-darwin\",\n]\n";
        assert_eq!(platforms(config), ["aarch64-apple-darwin"]);
    }

    #[test]
    fn a_tree_lists_each_package_once() {
        let tree =
            "kchess-core v2026.11.7 (/r/crates/kchess-core)\ntokio v1.54.1\n\ntokio v1.54.1\n";
        assert_eq!(packages(tree), set(&["kchess-core", "tokio"]));
    }
}
