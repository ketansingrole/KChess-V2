//! What the board draws and plays: its boards, piece sets and sounds (`apps/gpui/assets`, the
//! app's assets folder), embedded once. The app's asset source serves them with its own icons.

use std::borrow::Cow;

/// The boards, pieces and sounds.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../apps/gpui/assets"]
#[include = "boards/*"]
#[include = "pieces/**/*.svg"]
#[include = "sounds/*.mp3"]
struct Files;

/// A board image or piece, by its path (`boards/brown.png`).
pub fn shared(path: &str) -> Option<Cow<'static, [u8]>> {
    Files::get(path).map(|file| file.data)
}

/// A sound clip by its file name in `sounds/`.
pub fn sound(name: &str) -> Option<Cow<'static, [u8]>> {
    shared(&format!("sounds/{name}"))
}

/// Every embedded file's path.
pub fn paths() -> impl Iterator<Item = Cow<'static, str>> {
    Files::iter()
}
