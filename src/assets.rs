use std::{borrow::Cow, fs, path::PathBuf};

use gpui::{AssetSource, SharedString};

pub struct Assets {
    base: PathBuf,
}

impl Assets {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let full_path = self.base.join(path);
        if !full_path.exists() {
            return Ok(None);
        }

        let data = fs::read(full_path)?;
        Ok(Some(Cow::Owned(data)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let full_path = self.base.join(path);
        let entries = match fs::read_dir(full_path) {
            Ok(entries) => entries,
            Err(_) => return Ok(Vec::new()),
        };

        Ok(entries
            .filter_map(|entry| {
                entry
                    .ok()
                    .and_then(|entry| entry.file_name().into_string().ok())
                    .map(SharedString::from)
            })
            .collect())
    }
}
