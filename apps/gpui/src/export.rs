//! Saving an export (`saveExport`): the person picks where, starting in Downloads, and the text is
//! written there. `None` when they cancel.

use std::future::Future;
use std::path::PathBuf;

use gpui_kit::App;

/// Ask where to save `text` as `name`, then write it; the outcome, or `None` when cancelled.
pub fn save_text(
    name: &str,
    text: String,
    cx: &mut App,
) -> impl Future<Output = Option<Result<(), String>>> + 'static + use<> {
    let folder = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let chosen = cx.prompt_for_new_path(&folder, Some(name));
    async move {
        match chosen.await {
            Ok(Ok(Some(path))) => {
                Some(std::fs::write(&path, text).map_err(|cause| cause.to_string()))
            }
            Ok(Ok(None)) | Err(_) => None,
            Ok(Err(cause)) => Some(Err(cause.to_string())),
        }
    }
}

/// A file name from the players and today's date (`ExportGame`'s): safe characters only.
pub fn game_file_name(white: &str, black: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    format!("{white} vs {black} {today}")
        .chars()
        .filter(|c| c.is_alphanumeric() || " _.()+-".contains(*c))
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::game_file_name;

    #[test]
    fn file_names_keep_safe_characters() {
        let name = game_file_name("You", "Stockfish (Club Player)/x");
        assert!(name.starts_with("You vs Stockfish (Club Player)x "));
        assert!(!name.contains('/'));
    }
}
