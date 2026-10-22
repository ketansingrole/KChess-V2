//! What the app draws and plays: the Lucide icons it names, GPUI Kit's component icons, and the
//! Electron app's own boards, piece sets and sounds (`apps/desktop/app/assets`), embedded once so
//! both apps look and sound the same.

use std::borrow::Cow;

use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};

/// The Lucide icons the app names (`icon`), written by `tooling/gpui-icons.mjs` from the
/// Electron renderer's own icon set.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
struct Icons;

/// A Lucide icon by its Electron name (`i-lucide-<name>`). `tooling/gpui-icons.mjs` finds every
/// `icon("…")` in the sources, so the name must be written here as a literal.
pub fn icon(name: &'static str) -> Icon {
    Icon::empty().path(format!("icons/{name}.svg"))
}

/// The Electron renderer's boards, pieces and sounds.
#[derive(rust_embed::RustEmbed)]
#[folder = "../desktop/app/assets"]
#[include = "boards/*"]
#[include = "pieces/**/*.svg"]
#[include = "sounds/*.mp3"]
struct Shared;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = Icons::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(file) = Shared::get(path) {
            return Ok(Some(file.data));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = Icons::iter()
            .chain(Shared::iter())
            .filter(|name| name.starts_with(path))
            .map(|name| SharedString::from(name.to_string()))
            .collect();
        names.extend(gpui_kit::assets::Assets.list(path)?);
        names.sort();
        names.dedup();
        Ok(names)
    }
}
