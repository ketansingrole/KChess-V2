//! What the app draws and plays: the icons it names, GPUI Kit's component icons, its own icon and
//! startup logo, and the board's boards, piece sets and sounds (`kchess_board::assets`), embedded
//! once.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// The icons the app names (`icon`), at each size they are drawn for (`name@16.svg`), written
/// by `cargo xtask icons`.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
struct Icons;

/// An icon by its name (`kchess_ui::icon`), drawn from `Icons`.
pub use kchess_ui::icon;

/// The app icon and startup logo.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icon.png"]
#[include = "icon-splash.png"]
struct Own;

pub use kchess_board::assets::shared;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = Icons::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(file) = Own::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(bytes) = shared(path) {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = Icons::iter()
            .chain(Own::iter())
            .chain(kchess_board::assets::paths())
            .filter(|name| name.starts_with(path))
            .map(|name| SharedString::from(name.to_string()))
            .collect();
        names.extend(gpui_kit::assets::Assets.list(path)?);
        names.sort();
        names.dedup();
        Ok(names)
    }
}
