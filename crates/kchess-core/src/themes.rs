//! The user's own app themes: `*.json` files in the profile's `themes` folder, each checked as
//! an `AppTheme`. Every KChess app on the profile reads the same files.

use kchess_contracts::check::check;
use kchess_contracts::types::{AppTheme, CustomThemeReport};
use kchess_domain::misc::value::J;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

const MAX_FILES: usize = 100;
const MAX_BYTES: u64 = 64 * 1024;

/// Written once so there is something to copy. Files starting with `_` are never loaded.
const TEMPLATE: &str = r##"{
  "id": "my-theme",
  "name": "My theme",
  "dark": {
    "bg": "#1b1d23",
    "text": "#e6e8ee",
    "primary": "#7aa2f7",
    "muted": "#20232b",
    "elevated": "#262a33",
    "accented": "#323744",
    "border": "#343946",
    "textMuted": "#9aa1b2",
    "success": "#7fd28a",
    "warning": "#e6b35a",
    "error": "#ef7b86",
    "info": "#6cb6ff"
  },
  "light": {
    "bg": "#fbfbfd",
    "text": "#22252e",
    "primary": "#3b5fd0"
  }
}
"##;

/// The folder custom theme files are read from.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("themes")
}

/// Every valid theme file; a bad file is reported, never fatal.
pub fn custom_themes(host: &dyn Host, data_dir: &Path) -> Result<CustomThemeReport> {
    let dir = dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::wrap("The themes folder", e))?;
    let template = dir.join("_template.json");
    if !template.exists() {
        host.log(
            Level::Debug,
            "themes",
            "Theme template is missing, creating it.",
        );
        std::fs::write(&template, TEMPLATE)
            .map_err(|e| CoreError::wrap("The theme template", e))?;
    }
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| CoreError::wrap("The themes folder", e))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json") && !name.starts_with('_') && !name.starts_with('.'))
        .collect();
    files.sort();
    let mut themes = Vec::new();
    let mut problems = Vec::new();
    if files.len() > MAX_FILES {
        problems.push(format!("Only the first {MAX_FILES} theme files are read."));
    }
    for name in files.iter().take(MAX_FILES) {
        match read_theme(&dir.join(name)) {
            Ok(theme) => themes.push(AppTheme {
                custom: Some(true),
                ..theme
            }),
            Err(cause) => {
                host.log(
                    Level::Warn,
                    "themes",
                    &format!("Skipping invalid theme file: {name} {}", cause.message),
                );
                problems.push(format!("{name}: {}", cause.message));
            }
        }
    }
    Ok(CustomThemeReport {
        themes,
        dir: dir.display().to_string(),
        problems,
    })
}

fn read_theme(path: &Path) -> Result<AppTheme> {
    let size = std::fs::metadata(path)
        .map_err(|_| CoreError::new("unreadable"))?
        .len();
    if size > MAX_BYTES {
        return Err(CoreError::new("The file is too large."));
    }
    let text = std::fs::read_to_string(path).map_err(|_| CoreError::new("unreadable"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| CoreError::new("The file is not valid JSON."))?;
    check::<AppTheme>(J::from(&value)).map_err(CoreError::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_contracts::CoreEvent;

    struct Quiet;
    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _event: CoreEvent) {}
    }

    #[test]
    fn valid_files_load_and_bad_ones_are_reported() {
        let profile = tempfile::tempdir().unwrap();
        let folder = dir(profile.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("b-mine.json"),
            r##"{ "id": "mine", "name": " Mine ", "dark": { "bg": "#000", "text": "#fff", "primary": "#f00" } }"##,
        )
        .unwrap();
        std::fs::write(folder.join("a-broken.json"), "{").unwrap();
        std::fs::write(folder.join("_draft.json"), "{").unwrap();
        let report = custom_themes(&Quiet, profile.path()).unwrap();
        assert_eq!(
            serde_json::to_string(&report.themes).unwrap(),
            r##"[{"id":"mine","name":"Mine","dark":{"bg":"#000","text":"#fff","primary":"#f00"},"custom":true}]"##
        );
        assert_eq!(
            report.problems,
            ["a-broken.json: The file is not valid JSON."]
        );
        // The template is written once and never loaded.
        assert!(folder.join("_template.json").exists());
        let template: Value = serde_json::from_str(TEMPLATE).unwrap();
        assert!(check::<AppTheme>(J::from(&template)).is_ok());
    }
}
