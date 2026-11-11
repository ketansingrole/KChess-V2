//! Third-party attribution: THIRD_PARTY_NOTICES.md names every dependency that ships, and each
//! package carries the license texts of every crate compiled into it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::util::{Result, output, root, say};

use crate::members::{self, Role};

fn metadata(no_deps: bool) -> Result<Value> {
    let mut args = vec!["metadata", "--format-version", "1", "--locked"];
    if no_deps {
        args.push("--no-deps");
    }
    let text = output("cargo", &args)?;
    serde_json::from_str(&text).map_err(|cause| format!("cargo metadata: {cause}"))
}

/// Whether `notices` mentions `name` as a whole word (not inside another crate's name).
pub fn names(notices: &str, name: &str) -> bool {
    let lower = notices.to_lowercase();
    let name = name.to_lowercase();
    let word = |byte: Option<u8>| {
        byte.is_some_and(|b| {
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'@' || b == b'/'
        })
    };
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(found) = lower[from..].find(&name) {
        let start = from + found;
        let end = start + name.len();
        if !word(start.checked_sub(1).map(|at| bytes[at])) && !word(bytes.get(end).copied()) {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The registry and git dependencies that shipped workspace packages name directly, for their
/// programs or libraries (not their tests or build scripts), from `cargo metadata --no-deps`: the
/// shipped programs' and those of the workspace libraries they build on. Not the workspace hack's:
/// its own dependencies are linked only where a package uses them.
fn direct_dependencies(metadata: &Value) -> BTreeSet<String> {
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| Some((package["name"].as_str()?, package)))
        .collect();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<&str> = members::shipped().collect();
    let mut dependencies = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if members::role(name) == Role::Hack || !seen.insert(name) {
            continue;
        }
        let Some(package) = packages.get(name) else {
            continue;
        };
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if !dependency["kind"].is_null() {
                continue;
            }
            let dependency_name = dependency["name"].as_str().unwrap_or_default();
            if dependency["path"].is_null() {
                dependencies.insert(dependency_name.to_string());
            } else {
                queue.push(dependency_name);
            }
        }
    }
    dependencies
}

const NOTICES: &str = "THIRD_PARTY_NOTICES.md";

/// Whether `check_notices` reads `file`: the notices, and the manifests that name dependencies.
pub fn reads(file: &str) -> bool {
    file == NOTICES || crate::check::is_manifest(file)
}

/// `notices`: every direct dependency of a shipped KChess crate is attributed.
pub fn check_notices() -> Result {
    let notices = std::fs::read_to_string(root().join(NOTICES))
        .map_err(|cause| format!("Cannot read {NOTICES}: {cause}"))?;
    let dependencies = direct_dependencies(&metadata(true)?);
    let missing: Vec<&String> = dependencies
        .iter()
        .filter(|name| !names(&notices, name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "THIRD_PARTY_NOTICES.md is missing entries for: {}. Add each shipped dependency with its license and what it is used for.",
            missing
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    say(&format!(
        "All {} runtime dependencies are attributed.",
        dependencies.len()
    ));
    Ok(())
}

const MIT: &str =
    "Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the \"Software\"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.";

/// Whether the SPDX expression `license` (`MIT OR Apache-2.0`, or the older `MIT/Apache-2.0`)
/// offers license `id`.
fn offers(license: &str, id: &str) -> bool {
    license
        .split(" OR ")
        .flat_map(|choice| choice.split('/'))
        .any(|choice| choice.trim_matches(['(', ')', ' ']) == id)
}

/// A crate's license files, or the standard text of its SPDX license when it ships none.
fn license_text(package: &Value) -> Result<String> {
    let manifest = package["manifest_path"].as_str().unwrap_or_default();
    let dir = Path::new(manifest).parent().unwrap_or(Path::new("."));
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map_err(|cause| cause.to_string())?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| {
            let upper = name.to_uppercase();
            upper.starts_with("LICENSE")
                || upper.starts_with("LICENCE")
                || upper.starts_with("COPYING")
                || upper.starts_with("NOTICE")
        })
        .collect();
    files.sort();
    if !files.is_empty() {
        let mut text = String::new();
        for name in files {
            let body = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
            text.push_str(&format!("--- {name} ---\n{}\n\n", body.trim()));
        }
        return Ok(text);
    }
    let license = package["license"].as_str().unwrap_or_default();
    let name = package["name"].as_str().unwrap_or_default();
    if license.contains("GPL-3.0") {
        let gpl =
            std::fs::read_to_string(root().join("LICENSE")).map_err(|cause| cause.to_string())?;
        return Ok(format!(
            "--- GPL-3.0 (the same license as KChess) ---\n{}\n",
            gpl.trim()
        ));
    }
    // `MIT OR …` (or the older `…/MIT`): the MIT option is elected, so its text applies.
    if offers(license, "MIT") {
        let authors: Vec<&str> = package["authors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let holders = if authors.is_empty() {
            format!("the {name} authors")
        } else {
            authors.join(", ")
        };
        return Ok(format!("--- MIT ---\nCopyright (c) {holders}\n\n{MIT}\n"));
    }
    // A crate published without its license file: the standard text of a license it offers.
    for (id, text) in [
        ("Apache-2.0", include_str!("../licenses/Apache-2.0.txt")),
        ("MPL-2.0", include_str!("../licenses/MPL-2.0.txt")),
        ("BSD-3-Clause", include_str!("../licenses/BSD-3-Clause.txt")),
        ("ISC", include_str!("../licenses/ISC.txt")),
        ("Zlib", include_str!("../licenses/Zlib.txt")),
        ("BSL-1.0", include_str!("../licenses/BSL-1.0.txt")),
        ("Unicode-3.0", include_str!("../licenses/Unicode-3.0.txt")),
        ("CC0-1.0", include_str!("../licenses/CC0-1.0.txt")),
    ] {
        if offers(license, id) {
            return Ok(format!("--- {id} ---\n{}\n", text.trim()));
        }
    }
    Err(format!(
        "{name} {} ships no license file for {license}.",
        package["version"].as_str().unwrap_or_default()
    ))
}

/// The third-party crates compiled into a shipped program (for any platform), sorted by name and
/// version, from `cargo metadata`'s resolved graph: their runtime dependencies, not proc-macros,
/// build scripts' or tests', nor what only the workspace hack names.
fn shipped_crates(metadata: &Value) -> Vec<&Value> {
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| Some((node["id"].as_str()?, node)))
        .collect();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<&str> = packages
        .iter()
        .filter(|(_, package)| {
            members::shipped().any(|name| package["name"].as_str() == Some(name))
        })
        .map(|(id, _)| *id)
        .collect();
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        for dep in nodes
            .get(id)
            .and_then(|node| node["deps"].as_array())
            .into_iter()
            .flatten()
        {
            let runtime = dep["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|kind| kind["kind"].is_null());
            let Some(target) = dep["pkg"].as_str() else {
                continue;
            };
            let proc_macro = packages.get(target).is_some_and(|package| {
                package["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| {
                        t["kind"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|kind| kind == "proc-macro")
                    })
            });
            let hack = packages.get(target).is_some_and(|package| {
                members::role(package["name"].as_str().unwrap_or_default()) == Role::Hack
            });
            if runtime && !proc_macro && !hack {
                queue.push(target);
            }
        }
    }
    let mut crates: Vec<&Value> = seen
        .iter()
        .filter_map(|id| packages.get(id).copied())
        // Workspace crates are KChess's own code.
        .filter(|package| !package["source"].is_null())
        .collect();
    crates.sort_by_key(|package| {
        (
            package["name"].as_str().unwrap_or_default().to_string(),
            package["version"].as_str().unwrap_or_default().to_string(),
        )
    });
    crates
}

/// `licenses <file>`: the license texts of every crate compiled into a shipped program (for any
/// platform); proc-macros and build scripts run only at compile time and are left out.
pub fn write(path: &Path) -> Result {
    let metadata = metadata(false)?;
    let crates = shipped_crates(&metadata);
    let mut texts: Vec<(String, Vec<String>)> = Vec::new();
    for package in &crates {
        let text = license_text(package)?;
        let label = format!(
            "{} {}",
            package["name"].as_str().unwrap_or_default(),
            package["version"].as_str().unwrap_or_default()
        );
        match texts.iter_mut().find(|(known, _)| *known == text) {
            Some((_, names)) => names.push(label),
            None => texts.push((text, vec![label])),
        }
    }
    let index: Vec<String> = crates
        .iter()
        .map(|package| {
            format!(
                "- {} {} ({}) {}",
                package["name"].as_str().unwrap_or_default(),
                package["version"].as_str().unwrap_or_default(),
                package["license"].as_str().unwrap_or_default(),
                package["repository"].as_str().unwrap_or_default()
            )
            .trim_end()
            .to_string()
        })
        .collect();
    let rule = "=".repeat(78);
    let sections: Vec<String> = texts
        .iter()
        .map(|(text, names)| format!("{rule}\nApplies to: {}\n{rule}\n\n{text}", names.join(", ")))
        .collect();
    let body = format!(
        "Rust crates compiled into KChess, its core service, its command line and its speech helper:\n\n{}\n\n{}",
        index.join("\n"),
        sections.join("\n")
    );
    std::fs::write(path, body).map_err(|cause| cause.to_string())?;
    say(&format!(
        "Collected licenses for {} Rust crates.",
        crates.len()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MIT, direct_dependencies, license_text, names, offers, shipped_crates};
    use std::collections::BTreeSet;

    /// A crate's own license files come first; without them, the text of a license it offers.
    #[test]
    fn a_crate_is_attributed_with_its_own_files_or_its_licenses_text() {
        let dir = tempfile::tempdir().unwrap();
        let package = |license: &str, authors: &[&str]| {
            serde_json::json!({
                "name": "demo",
                "version": "1.0.0",
                "license": license,
                "authors": authors,
                "manifest_path": dir.path().join("Cargo.toml"),
            })
        };
        assert_eq!(
            license_text(&package("MIT OR Apache-2.0", &["Ada"])).unwrap(),
            format!("--- MIT ---\nCopyright (c) Ada\n\n{MIT}\n")
        );
        assert!(
            license_text(&package("MIT", &[]))
                .unwrap()
                .contains("Copyright (c) the demo authors\n")
        );
        assert!(
            license_text(&package("Apache-2.0", &[]))
                .unwrap()
                .starts_with("--- Apache-2.0 ---\n")
        );
        assert!(
            license_text(&package("GPL-3.0-or-later", &[]))
                .unwrap()
                .starts_with("--- GPL-3.0 (the same license as KChess) ---\n")
        );
        assert!(
            license_text(&package("LicenseRef-Unknown", &[]))
                .unwrap_err()
                .contains("ships no license file")
        );
        std::fs::write(dir.path().join("LICENSE-MIT"), "  the crate's own  \n").unwrap();
        assert_eq!(
            license_text(&package("MIT", &[])).unwrap(),
            "--- LICENSE-MIT ---\nthe crate's own\n\n"
        );
    }

    #[test]
    fn a_license_is_offered_by_any_choice_of_its_expression() {
        assert!(offers("MIT OR Apache-2.0", "Apache-2.0"));
        assert!(offers("(MIT OR Apache-2.0) AND Unicode-3.0", "MIT"));
        assert!(offers("Apache-2.0/MIT", "MIT"));
        assert!(!offers("MIT-0", "MIT"));
        assert!(!offers("GPL-3.0-or-later", "MIT"));
    }

    #[test]
    fn notices_name_what_shipped_packages_use_directly() {
        let metadata = serde_json::json!({ "packages": [
            { "name": "kchess-cli", "dependencies": [
                { "name": "kchess-core", "kind": null, "path": "/r/crates/kchess-core" },
                { "name": "kchess-hack", "kind": null, "path": "/r/crates/kchess-hack" },
            ] },
            { "name": "kchess-core", "dependencies": [
                { "name": "tokio", "kind": null, "path": null },
                { "name": "tempfile", "kind": "dev", "path": null },
                { "name": "cc", "kind": "build", "path": null },
                { "name": "kchess-domain", "kind": null, "path": "/r/crates/kchess-domain" },
            ] },
            { "name": "kchess-domain", "dependencies": [
                { "name": "shakmaty", "kind": null, "path": null },
            ] },
            { "name": "kchess-hack", "dependencies": [
                { "name": "proptest", "kind": null, "path": null },
            ] },
            { "name": "xtask", "dependencies": [
                { "name": "image", "kind": null, "path": null },
            ] },
            // A member nothing shipped builds on, such as a benchmark.
            { "name": "kchess-bench", "dependencies": [
                { "name": "criterion", "kind": null, "path": null },
            ] },
        ] });
        assert_eq!(
            direct_dependencies(&metadata),
            BTreeSet::from(["shakmaty".to_string(), "tokio".to_string()])
        );
    }

    #[test]
    fn the_license_file_follows_what_shipped_programs_link() {
        let package = |id: &str, name: &str, source: Option<&str>, kind: &str| {
            serde_json::json!({ "id": id, "name": name, "version": "1.0.0", "source": source,
                "targets": [{ "kind": [kind] }] })
        };
        let registry = Some("registry+https://github.com/rust-lang/crates.io-index");
        let dep = |pkg: &str, kind: Option<&str>| serde_json::json!({ "pkg": pkg, "dep_kinds": [{ "kind": kind }] });
        let metadata = serde_json::json!({
            "packages": [
                package("cli", "kchess-cli", None, "bin"),
                package("hack", "kchess-hack", None, "lib"),
                package("tokio", "tokio", registry, "lib"),
                package("bytes", "bytes", registry, "lib"),
                package("macros", "tokio-macros", registry, "proc-macro"),
                package("cc", "cc", registry, "lib"),
                package("tempfile", "tempfile", registry, "lib"),
                package("proptest", "proptest", registry, "lib"),
                package("unused", "unused", registry, "lib"),
            ],
            "resolve": { "nodes": [
                { "id": "cli", "deps": [
                    dep("tokio", None), dep("hack", None), dep("cc", Some("build")),
                    dep("tempfile", Some("dev")), dep("macros", None),
                ] },
                { "id": "tokio", "deps": [dep("bytes", None)] },
                { "id": "hack", "deps": [dep("proptest", None)] },
            ] },
        });
        let names: Vec<&str> = shipped_crates(&metadata)
            .iter()
            .map(|package| package["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["bytes", "tokio"]);
    }

    #[test]
    fn names_are_whole_words() {
        let notices = "| serde_json | MIT | JSON |\n| tokio-util | MIT |";
        assert!(names(notices, "serde_json"));
        assert!(names(notices, "tokio-util"));
        assert!(!names(notices, "serde"), "inside serde_json");
        assert!(!names(notices, "tokio"), "inside tokio-util");
    }
}
