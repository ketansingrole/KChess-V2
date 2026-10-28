//! Boards and pieces as the Electron app draws them (`crates/kchess-domain/data/looks.json`): the
//! board images and their coordinate colors, and the piece sets, rasterized at the exact size they
//! are shown so they stay sharp at any board size.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock};

use gpui_kit::{
    App, AssetSource as _, DevicePixels, Global, Hsla, RenderImage, Rgba, SvgSize, rgb, size,
};
use serde::Deserialize;
use shakmaty::{Color, Piece, Role};

const LOOKS_JSON: &str = include_str!("../../../../crates/kchess-domain/data/looks.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardLook {
    pub id: String,
    pub name: String,
    /// The image in `apps/desktop/app/assets/boards`.
    pub image: String,
    /// Coordinate labels on dark squares.
    pub coord_white: String,
    /// Coordinate labels on light squares.
    pub coord_black: String,
}

#[derive(Deserialize)]
pub struct PieceSet {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Looks {
    boards: Vec<BoardLook>,
    piece_sets: Vec<PieceSet>,
    default_piece_set: String,
    default_board: String,
    animation_ms: std::collections::HashMap<String, f64>,
}

static LOOKS: LazyLock<Looks> = LazyLock::new(|| match serde_json::from_str(LOOKS_JSON) {
    Ok(looks) => looks,
    // `looks.json` is checked in and read by this crate's tests.
    Err(cause) => panic!("looks.json does not decode: {cause}"),
});

/// How long a piece slides for the Settings choice (`animationMs`): `none`, `fast`, `normal`,
/// `slow`.
pub fn animation_ms(choice: &str) -> f64 {
    LOOKS.animation_ms.get(choice).copied().unwrap_or(180.)
}

/// Every board look, in the order Settings shows them.
pub fn boards() -> &'static [BoardLook] {
    &LOOKS.boards
}

/// Every piece set, in the order Settings shows them.
pub fn piece_sets() -> &'static [PieceSet] {
    &LOOKS.piece_sets
}

/// The board look `id`, or the default one.
pub fn board(id: &str) -> &'static BoardLook {
    LOOKS
        .boards
        .iter()
        .find(|board| board.id == id)
        .or_else(|| {
            LOOKS
                .boards
                .iter()
                .find(|board| board.id == LOOKS.default_board)
        })
        .unwrap_or(&LOOKS.boards[0])
}

/// The piece set `id`, or the default one.
pub fn piece_set(id: &str) -> &'static str {
    LOOKS
        .piece_sets
        .iter()
        .find(|set| set.id == id)
        .map_or(LOOKS.default_piece_set.as_str(), |set| set.id.as_str())
}

/// `#rrggbb` as a color; transparent when it is not one.
pub fn hex(text: &str) -> Hsla {
    u32::from_str_radix(text.trim_start_matches('#'), 16)
        .map(|value| rgb(value).into())
        .unwrap_or_else(|_| Rgba::default().into())
}

/// The image of a piece in a set (`pieces/cburnett/wK.svg`).
pub fn piece_path(set: &str, piece: Piece) -> String {
    let color = match piece.color {
        Color::White => 'w',
        Color::Black => 'b',
    };
    let role = match piece.role {
        Role::Pawn => 'P',
        Role::Knight => 'N',
        Role::Bishop => 'B',
        Role::Rook => 'R',
        Role::Queen => 'Q',
        Role::King => 'K',
    };
    format!("pieces/{}/{color}{role}.svg", piece_set(set))
}

/// Pieces rasterized per set and size; the oldest sizes are dropped while a window resizes.
#[derive(Default)]
struct PieceImages {
    images: HashMap<(String, u32), Arc<RenderImage>>,
    order: VecDeque<(String, u32)>,
}

impl Global for PieceImages {}

/// The most images kept: two piece sets at four sizes.
const MAX_IMAGES: usize = 12 * 8;

/// A piece of set `set`, `pixels` device pixels square.
pub fn piece_image(set: &str, piece: Piece, pixels: u32, cx: &mut App) -> Option<Arc<RenderImage>> {
    let key = (piece_path(set, piece), pixels.max(1));
    if let Some(image) = cx.default_global::<PieceImages>().images.get(&key) {
        return Some(Arc::clone(image));
    }
    let bytes = crate::assets::Assets.load(&key.0).ok()??;
    let renderer = cx.svg_renderer();
    let parsed = renderer.parse_svg(&bytes).ok()?;
    let side = DevicePixels(key.1 as i32);
    let image = renderer
        .render_parsed(&parsed, SvgSize::ExactSize(size(side, side)))
        .ok()?;
    let cache = cx.default_global::<PieceImages>();
    cache.images.insert(key.clone(), Arc::clone(&image));
    cache.order.push_back(key);
    while cache.order.len() > MAX_IMAGES {
        if let Some(old) = cache.order.pop_front() {
            cache.images.remove(&old);
        }
    }
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_image_is_embedded() {
        for set in &LOOKS.piece_sets {
            for color in Color::ALL {
                for role in Role::ALL {
                    let path = piece_path(&set.id, Piece { color, role });
                    assert!(
                        crate::assets::Assets.load(&path).ok().flatten().is_some(),
                        "{path}"
                    );
                }
            }
        }
        for board in &LOOKS.boards {
            let path = format!("boards/{}", board.image);
            assert!(
                crate::assets::Assets.load(&path).ok().flatten().is_some(),
                "{path}"
            );
        }
    }

    #[test]
    fn unknown_names_fall_back_to_the_defaults() {
        assert_eq!(board("gone").id, "brown");
        assert_eq!(piece_set("gone"), "cburnett");
    }
}
