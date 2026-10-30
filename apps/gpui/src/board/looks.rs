//! Boards and pieces as the Electron app draws them (`crates/kchess-domain/data/looks.json`): the
//! board images and their coordinate colors, and the piece sets, rasterized at the exact size they
//! are shown so they stay sharp at any board size.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock};

use gpui_kit::{
    App, AppContext as _, AssetSource as _, DevicePixels, EntityId, Global, Hsla, RenderImage,
    Rgba, SvgRenderer, SvgSize, rgb, size,
};
use serde::Deserialize;
use shakmaty::{Color, Piece, Role};

const LOOKS_JSON: &str = include_str!("../../../../crates/kchess-domain/data/looks.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardLook {
    pub id: String,
    pub name: String,
    /// The image in `apps/gpui/assets/boards`.
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
    /// Sets and sizes being rasterized in the background (`rasterize_later`), each with the
    /// views to draw again once it is done.
    pending: HashMap<(String, u32), Vec<EntityId>>,
    /// Views that asked for a size while `MOST_PENDING` were already being rasterized: they are
    /// drawn again when one finishes, and ask again.
    deferred: Vec<EntityId>,
}

impl Global for PieceImages {}

impl PieceImages {
    fn keep(&mut self, key: (String, u32), image: Arc<RenderImage>) {
        if self.images.insert(key.clone(), image).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_IMAGES {
            if let Some(old) = self.order.pop_front() {
                self.images.remove(&old);
            }
        }
    }
}

/// The most images kept: two piece sets at four sizes.
const MAX_IMAGES: usize = 12 * 8;

/// The most sizes rasterized in the background at once; a window being resized asks for a new
/// size every frame.
const MOST_PENDING: usize = 2;

/// A piece of set `set`, `pixels` device pixels square, as `viewer` draws it.
///
/// A size not rasterized yet is rasterized in the background, the set's twelve pieces at once,
/// and `viewer` is drawn again once they are; until then the piece's nearest size stands in,
/// scaled. A piece with no size at all is rasterized here and now.
pub fn piece_image(
    set: &str,
    piece: Piece,
    pixels: u32,
    viewer: EntityId,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let path = piece_path(set, piece);
    let pixels = pixels.max(1);
    let images = cx.default_global::<PieceImages>();
    if let Some(image) = images.images.get(&(path.clone(), pixels)) {
        return Some(Arc::clone(image));
    }
    let nearest = images
        .images
        .iter()
        .filter(|((drawn, _), _)| *drawn == path)
        .min_by_key(|((_, size), _)| size.abs_diff(pixels))
        .map(|(_, image)| Arc::clone(image));
    let Some(nearest) = nearest else {
        let image = rasterize(&path, pixels, &cx.svg_renderer())?;
        cx.default_global::<PieceImages>()
            .keep((path, pixels), Arc::clone(&image));
        return Some(image);
    };
    rasterize_later(piece_set(set), pixels, viewer, cx);
    Some(nearest)
}

fn rasterize(path: &str, pixels: u32, renderer: &SvgRenderer) -> Option<Arc<RenderImage>> {
    let bytes = crate::assets::Assets.load(path).ok()??;
    let parsed = renderer.parse_svg(&bytes).ok()?;
    let side = DevicePixels(pixels as i32);
    renderer
        .render_parsed(&parsed, SvgSize::ExactSize(size(side, side)))
        .ok()
}

/// Rasterize every piece of `set` at `pixels` on a background thread, then draw `viewer` again.
fn rasterize_later(set: &str, pixels: u32, viewer: EntityId, cx: &mut App) {
    let key = (set.to_string(), pixels);
    let images = cx.default_global::<PieceImages>();
    if let Some(waiting) = images.pending.get_mut(&key) {
        if !waiting.contains(&viewer) {
            waiting.push(viewer);
        }
        return;
    }
    if images.pending.len() >= MOST_PENDING {
        if !images.deferred.contains(&viewer) {
            images.deferred.push(viewer);
        }
        return;
    }
    images.pending.insert(key.clone(), vec![viewer]);
    let paths: Vec<String> = Color::ALL
        .into_iter()
        .flat_map(|color| Role::ALL.map(|role| piece_path(set, Piece { color, role })))
        .collect();
    let renderer = cx.svg_renderer();
    let rasterized = cx.background_spawn(async move {
        paths
            .into_iter()
            .filter_map(|path| Some((path.clone(), rasterize(&path, pixels, &renderer)?)))
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let rasterized = rasterized.await;
        cx.update(|cx| {
            let images = cx.default_global::<PieceImages>();
            for (path, image) in rasterized {
                images.keep((path, pixels), image);
            }
            let mut waiting = images.pending.remove(&key).unwrap_or_default();
            waiting.append(&mut images.deferred);
            for view in waiting {
                cx.notify(view);
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};

    use super::*;

    struct Board;

    fn width(image: &RenderImage) -> i32 {
        image.size(0).width.0
    }

    /// Every new board size rasterized its twelve pieces on the main thread, in the frame that
    /// showed it (4-10 ms when a board page opened, and again at each size while a window was
    /// resized).
    #[gpui_kit::test]
    fn a_new_size_is_rasterized_in_the_background(cx: &mut TestAppContext) {
        let board: Entity<Board> = cx.new(|_| Board);
        let viewer = board.entity_id();
        let drawn_again = Rc::new(Cell::new(0));
        let _observing = cx.update(|cx| {
            let drawn_again = drawn_again.clone();
            cx.observe(&board, move |_, _| drawn_again.set(drawn_again.get() + 1))
        });
        let knight = Piece {
            color: Color::White,
            role: Role::Knight,
        };
        let image = |pixels, cx: &mut TestAppContext| {
            cx.update(|cx| piece_image("cburnett", knight, pixels, viewer, cx))
                .expect("the knight")
        };

        // No size of the piece yet: made at once.
        let first = image(40, cx);
        assert_eq!(width(&first), 40);
        // Another size: the nearest stands in while the set is made in the background.
        assert!(Arc::ptr_eq(&image(64, cx), &first));
        assert!(Arc::ptr_eq(&image(48, cx), &first));
        // A third size waits for one of those two to finish.
        assert!(Arc::ptr_eq(&image(72, cx), &first));
        cx.run_until_parked();
        assert_eq!(width(&image(64, cx)), 64);
        assert_eq!(width(&image(48, cx)), 48);
        let bishop = Piece {
            color: Color::Black,
            role: Role::Bishop,
        };
        let made = cx.update(|cx| piece_image("cburnett", bishop, 64, viewer, cx));
        assert_eq!(made.map(|image| width(&image)), Some(64), "the whole set");
        assert!(drawn_again.get() >= 1, "the board is drawn again");
        // The deferred size, asked again when the board is drawn again.
        assert_eq!(width(&image(72, cx)), 64);
        cx.run_until_parked();
        assert_eq!(width(&image(72, cx)), 72);
    }

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
