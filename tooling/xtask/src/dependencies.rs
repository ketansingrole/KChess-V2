//! Every dependency a member's manifest lists is named in its Rust: a crate left behind when the
//! code using it moved still compiled and linked into every build (the app kept `resvg`, `image`
//! and `getrandom` after the code drawing a game's picture and drawing a Chess960 start had moved
//! to `kchess-board` and `kchess-games`; the command line kept `kchess-core`). `cargo machete`
//! finds the same from the same reading: a crate is used when its name is a path's first segment
//! (`serde::`, `use serde`) somewhere in the member's sources.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::util::root;

    /// A dependency named nowhere in its member's Rust that it still needs, and why.
    const KEPT: [(&str, &str, &str); 1] = [(
        "apps/gpui",
        "gpui-pre-apple",
        "its library is named `gpui_apple` (the screenshots' text system, `src/e2e/screens.rs`)",
    )];

    /// The `[workspace] members` of the root manifest.
    fn members(manifest: &str) -> Vec<String> {
        manifest
            .lines()
            .find(|line| line.starts_with("members = ["))
            .map(|line| {
                line.split('"')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The dependencies a manifest lists, in any dependencies section (a target's too).
    fn dependencies(manifest: &str) -> Vec<String> {
        let mut listed = Vec::new();
        let mut in_section = false;
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_section = line.ends_with("dependencies]");
                continue;
            }
            if !in_section || line.starts_with('#') || line.is_empty() {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let name = key.trim().trim_end_matches(".workspace").trim();
            if !name.is_empty() {
                listed.push(name.to_string());
            }
        }
        listed
    }

    /// Whether `text` names the crate `ident` as a path's first segment.
    fn names(text: &str, ident: &str) -> bool {
        let bytes = text.as_bytes();
        let mut from = 0;
        while let Some(found) = text[from..].find(ident) {
            let start = from + found;
            let end = start + ident.len();
            from = end;
            let before = start.checked_sub(1).map(|at| bytes[at]);
            if before
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b':')
            {
                continue;
            }
            let after = &text[end..];
            if after.starts_with("::") {
                return true;
            }
            let lead = text[..start].trim_end();
            if (lead.ends_with("use") || lead.ends_with("extern crate"))
                && after.starts_with([';', ' ', ':'])
            {
                return true;
            }
        }
        false
    }

    fn sources(dir: &Path, into: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "target") {
                    sources(&path, into);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                into.push_str(&text);
                into.push('\n');
            }
        }
    }

    #[test]
    fn every_dependency_is_named_in_its_members_rust() {
        let root = root();
        let workspace =
            std::fs::read_to_string(root.join("Cargo.toml")).expect("the root manifest");
        let mut unused = Vec::new();
        for member in members(&workspace) {
            let dir = root.join(&member);
            let manifest =
                std::fs::read_to_string(dir.join("Cargo.toml")).expect("a member's manifest");
            // The workspace hack names its dependencies only to fix their features.
            if member == "crates/kchess-hack" {
                continue;
            }
            let mut text = String::new();
            sources(&dir, &mut text);
            for dependency in dependencies(&manifest) {
                let ident = dependency.replace('-', "_");
                let kept = KEPT
                    .iter()
                    .any(|(kept_in, name, _)| *kept_in == member && *name == dependency);
                if dependency != "kchess-hack" && !kept && !names(&text, &ident) {
                    unused.push(format!("{member}: {dependency}"));
                }
            }
        }
        assert!(
            unused.is_empty(),
            "dependencies named nowhere in their member's Rust (remove them, or keep one in \
             `KEPT` with why): {unused:?}"
        );
    }

    #[test]
    fn a_crate_is_named_by_a_paths_first_segment() {
        assert!(names("use serde::Serialize;", "serde"));
        assert!(names("let x = serde_json::json!({});", "serde_json"));
        assert!(names("use futures;", "futures"));
        assert!(!names("use serde_json::Value;", "serde"));
        assert!(!names("kchess_board::image::draw()", "image"));
        assert!(!names("// an image of the board", "image"));
        assert_eq!(
            dependencies(
                "[dependencies]\nserde.workspace = true\n# a note\nimage = { version = \"0.25\" }\n\n[target.'cfg(windows)'.dependencies]\nwindows = \"0.61\"\n[lints]\nworkspace = true\n"
            ),
            ["serde", "image", "windows"]
        );
    }
}
