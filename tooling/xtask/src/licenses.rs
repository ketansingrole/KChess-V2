//! Third-party attribution: THIRD_PARTY_NOTICES.md names every dependency that ships, and each
//! package carries the license texts of every crate compiled into it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::util::{Result, output, root, say};

/// The programs a package ships, whose runtime crates it must attribute.
const SHIPPED: [&str; 4] = [
    "kchess-gpui",
    "kchess-service",
    "kchess-voice",
    "kchess-cli",
];

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

/// `notices`: every direct dependency of a shipped KChess crate is attributed.
pub fn check_notices() -> Result {
    let notices = std::fs::read_to_string(root().join("THIRD_PARTY_NOTICES.md"))
        .map_err(|cause| format!("Cannot read THIRD_PARTY_NOTICES.md: {cause}"))?;
    let metadata = metadata(true)?;
    let mut dependencies = BTreeSet::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let name = package["name"].as_str().unwrap_or_default();
        if name == "xtask" {
            continue;
        }
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if dependency["kind"].is_null() && dependency["path"].is_null() {
                dependencies.insert(dependency["name"].as_str().unwrap_or_default().to_string());
            }
        }
    }
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
    if license
        .split(" OR ")
        .flat_map(|choice| choice.split('/'))
        .any(|choice| choice.trim_matches(['(', ')', ' ']) == "MIT")
    {
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
        if license
            .split(" OR ")
            .flat_map(|choice| choice.split('/'))
            .any(|choice| choice.trim_matches(['(', ')', ' ']) == id)
        {
            return Ok(format!("--- {id} ---\n{}\n", text.trim()));
        }
    }
    Err(format!(
        "{name} {} ships no license file for {license}.",
        package["version"].as_str().unwrap_or_default()
    ))
}

/// `licenses <file>`: the license texts of every crate compiled into a shipped program (for any
/// platform); proc-macros and build scripts run only at compile time and are left out.
pub fn write(path: &Path) -> Result {
    let metadata = metadata(false)?;
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
        .filter(|(_, package)| SHIPPED.contains(&package["name"].as_str().unwrap_or_default()))
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
            if runtime && !proc_macro {
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
    use super::names;

    #[test]
    fn names_are_whole_words() {
        let notices = "| serde_json | MIT | JSON |\n| tokio-util | MIT |";
        assert!(names(notices, "serde_json"));
        assert!(names(notices, "tokio-util"));
        assert!(!names(notices, "serde"), "inside serde_json");
        assert!(!names(notices, "tokio"), "inside tokio-util");
    }
}
