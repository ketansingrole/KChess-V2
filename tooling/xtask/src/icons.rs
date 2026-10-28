//! The Lucide icons the app names (`icon("name")` in apps/gpui/src), as SVG files in
//! `apps/gpui/assets/icons`, from the Iconify Lucide set the Electron app drew its icons with, so
//! glyphs never change. Aliases resolve as Iconify resolves them (`history` is
//! `rotate-ccw-clock`). Writing fetches the pinned set; `check` needs no network: every named icon
//! has its file and every file is named.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::util::{Result, root, say};

const SET: &str = "https://cdn.jsdelivr.net/npm/@iconify-json/lucide@1.2.147/icons.json";

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every name written as `icon("…")`.
pub fn named(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find("icon(\"") {
        let before = rest[..at].chars().next_back();
        rest = &rest[at + 6..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(end) = rest.find('"') {
            let name = &rest[..end];
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn wanted() -> BTreeSet<String> {
    let mut files = Vec::new();
    sources(&root().join("apps/gpui/src"), &mut files);
    files
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|text| named(&text))
        .collect()
}

fn folder() -> PathBuf {
    root().join("apps/gpui/assets/icons")
}

fn present() -> BTreeSet<String> {
    std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .strip_suffix(".svg")
                .map(str::to_string)
        })
        .collect()
}

/// `icons --check`: the files match the names in the sources.
pub fn check() -> Result {
    let (wanted, present) = (wanted(), present());
    let missing: Vec<&String> = wanted.difference(&present).collect();
    let unused: Vec<&String> = present.difference(&wanted).collect();
    if missing.is_empty() && unused.is_empty() {
        say(&format!("Icons: {} up to date.", wanted.len()));
        return Ok(());
    }
    Err(format!(
        "Icons are out of date (missing {missing:?}, unused {unused:?}). Run `cargo xtask icons`."
    ))
}

fn resolve<'a>(set: &'a Value, name: &str, depth: usize) -> Result<&'a Value> {
    if let Some(icon) = set["icons"].get(name) {
        return Ok(icon);
    }
    let parent = set["aliases"][name]["parent"].as_str();
    match parent {
        Some(parent) if depth < 8 => resolve(set, parent, depth + 1),
        _ => Err(format!("Lucide has no icon named {name}.")),
    }
}

/// `icons`: write the files the sources name, and remove the rest.
pub fn write() -> Result {
    let cache = root().join("target/lucide-1.2.147.json");
    if !cache.exists() {
        say("Downloading the Lucide icon set…");
        crate::util::download(SET, &cache)?;
    }
    let text = std::fs::read_to_string(&cache).map_err(|cause| cause.to_string())?;
    let set: Value = serde_json::from_str(&text)
        .map_err(|cause| format!("The icon set is unreadable: {cause}"))?;
    let (width, height) = (
        set["width"].as_u64().unwrap_or(24),
        set["height"].as_u64().unwrap_or(24),
    );
    let wanted = wanted();
    std::fs::create_dir_all(folder()).map_err(|cause| cause.to_string())?;
    for name in present().difference(&wanted) {
        std::fs::remove_file(folder().join(format!("{name}.svg")))
            .map_err(|cause| cause.to_string())?;
    }
    for name in &wanted {
        let icon = resolve(&set, name, 0)?;
        let (w, h) = (
            icon["width"].as_u64().unwrap_or(width),
            icon["height"].as_u64().unwrap_or(height),
        );
        let body = icon["body"].as_str().unwrap_or_default();
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">{body}</svg>\n"
        );
        std::fs::write(folder().join(format!("{name}.svg")), svg)
            .map_err(|cause| cause.to_string())?;
    }
    say(&format!("Icons: wrote {}.", wanted.len()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::named;

    #[test]
    fn finds_icon_names_written_as_literals() {
        let text =
            r#"icon("circle-dot"); crate::assets::icon("x2"); my_icon("not-this"); icon(name)"#;
        let names: Vec<String> = named(text).into_iter().collect();
        assert_eq!(names, ["circle-dot", "x2"]);
    }

    #[test]
    fn the_apps_icons_are_all_present() {
        super::check().unwrap();
    }
}
