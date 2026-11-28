//! Boards and pieces (`crates/kchess-board/data/looks.json`): the
//! board images and their coordinate colors, and the piece sets, rasterized at the exact size they
//! are shown so they stay sharp at any board size.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock};

use gpui_kit::{
    App, AppContext as _, DevicePixels, EntityId, Global, RenderImage, SvgRenderer, SvgSize, size,
};
use serde::Deserialize;
use shakmaty::{Color, Piece, Role};

const LOOKS_JSON: &str = include_str!("../../data/looks.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardLook {
    pub id: String,
    pub name: String,
    /// How its squares are drawn.
    #[serde(flatten)]
    pub surface: Surface,
    /// Coordinate labels on dark squares.
    pub coord_white: String,
    /// Coordinate labels on light squares.
    pub coord_black: String,
}

/// A board's squares: two flat colours the board paints itself, sharp at any size and with no
/// texture to hold, or a textured image (`apps/gpui/assets/boards`) stretched under it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    Colors {
        #[serde(deserialize_with = "hex")]
        light: u32,
        #[serde(deserialize_with = "hex")]
        dark: u32,
    },
    Image(String),
}

fn hex<'de, D: serde::Deserializer<'de>>(from: D) -> Result<u32, D::Error> {
    let text = String::deserialize(from)?;
    text.strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .ok_or_else(|| serde::de::Error::custom(format!("{text} is not #rrggbb")))
}

impl Surface {
    /// The image's path among the assets (`boards/wood.jpg`), for a textured board.
    pub fn image_path(&self) -> Option<String> {
        match self {
            Self::Image(image) => Some(format!("boards/{image}")),
            Self::Colors { .. } => None,
        }
    }
}

/// Whether the square at `column`, `row` from the viewer's top left is dark: the corners at the
/// viewer's bottom left and top right are, from either side.
pub fn dark_square(column: u32, row: u32) -> bool {
    (column + row) % 2 == 1
}

/// The squares of a drawn board in `bounds`: the light colour over all of it, the dark squares
/// over that, each corner square taking the board's own `corners`, so they stay concentric with
/// what holds it. Edges fall on whole device pixels, so no square blurs into its neighbour.
pub fn paint_squares(
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    light: u32,
    dark: u32,
    corners: gpui_kit::Corners<gpui_kit::Pixels>,
    window: &mut gpui_kit::Window,
) {
    use gpui_kit::{fill, point, px, rgb};
    let side = bounds.size.width / 8.;
    let corners = gpui_kit::Corners {
        top_left: corners.top_left.min(side),
        top_right: corners.top_right.min(side),
        bottom_right: corners.bottom_right.min(side),
        bottom_left: corners.bottom_left.min(side),
    };
    window.paint_quad(fill(bounds, rgb(light)).corner_radii(corners));
    let scale = window.scale_factor();
    let snap = |at: gpui_kit::Pixels| px((f32::from(at) * scale).round() / scale);
    let edge = |origin: gpui_kit::Pixels, extent: gpui_kit::Pixels, index: u32| {
        snap(origin + extent * (index as f32 / 8.))
    };
    for (column, row, rounded) in dark_squares(corners) {
        let (left, right) = (
            edge(bounds.origin.x, bounds.size.width, column),
            edge(bounds.origin.x, bounds.size.width, column + 1),
        );
        let (top, bottom) = (
            edge(bounds.origin.y, bounds.size.height, row),
            edge(bounds.origin.y, bounds.size.height, row + 1),
        );
        let square = gpui_kit::Bounds::from_corners(point(left, top), point(right, bottom));
        window.paint_quad(fill(square, rgb(dark)).corner_radii(rounded));
    }
}

/// A board's squares filling what holds them, with the board's `corners`: drawn in one paint for
/// a flat board (no layout of its own beyond the one box, nothing to notify), or the image
/// stretched for a textured one.
pub fn surface(
    surface: &Surface,
    corners: gpui_kit::Corners<gpui_kit::Pixels>,
) -> gpui_kit::AnyElement {
    use gpui_kit::{
        IntoElement as _, ObjectFit, SharedString, Styled as _, StyledImage as _, canvas, img,
    };
    match *surface {
        Surface::Colors { light, dark } => canvas(
            |_, _, _| {},
            move |bounds, (), window, _| paint_squares(bounds, light, dark, corners, window),
        )
        .absolute()
        .size_full()
        .into_any_element(),
        Surface::Image(ref image) => img(SharedString::from(format!("boards/{image}")))
            .absolute()
            .size_full()
            .object_fit(ObjectFit::Fill)
            .rounded_tl(corners.top_left)
            .rounded_tr(corners.top_right)
            .rounded_br(corners.bottom_right)
            .rounded_bl(corners.bottom_left)
            .into_any_element(),
    }
}

/// The 32 dark squares by column and row, each with the board's corners it rounds: the dark
/// squares at the top right and bottom left take those corners, the rest none.
pub fn dark_squares(
    board: gpui_kit::Corners<gpui_kit::Pixels>,
) -> impl Iterator<Item = (u32, u32, gpui_kit::Corners<gpui_kit::Pixels>)> {
    (0..8u32).flat_map(move |row| {
        (0..8u32)
            .filter(move |&column| dark_square(column, row))
            .map(move |column| {
                let at = |on: bool, radius| if on { radius } else { gpui_kit::px(0.) };
                (
                    column,
                    row,
                    gpui_kit::Corners {
                        top_left: at(column == 0 && row == 0, board.top_left),
                        top_right: at(column == 7 && row == 0, board.top_right),
                        bottom_right: at(column == 7 && row == 7, board.bottom_right),
                        bottom_left: at(column == 0 && row == 7, board.bottom_left),
                    },
                )
            })
    })
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
    LOOKS.animation_ms.get(choice).copied().unwrap_or(250.)
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
    /// Keeps `image`; returns the images it pushed out, whose textures the caller releases.
    #[must_use]
    fn keep(&mut self, key: (String, u32), image: Arc<RenderImage>) -> Vec<Arc<RenderImage>> {
        let mut dropped = Vec::new();
        if let Some(replaced) = self.images.insert(key.clone(), image) {
            dropped.push(replaced);
        } else {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_IMAGES {
            if let Some(old) = self.order.pop_front()
                && let Some(image) = self.images.remove(&old)
            {
                dropped.push(image);
            }
        }
        dropped
    }
}

/// Releases the textures of pieces no longer kept: GPUI keeps a drawn image's texture until it
/// is dropped, so every size a resize passed through stayed in the GPU atlas. Deferred, since
/// the window being drawn is not among the app's windows until it has been drawn.
fn release(dropped: Vec<Arc<RenderImage>>, cx: &mut App) {
    if dropped.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for image in dropped {
            cx.drop_image(image, None);
        }
    });
}

/// The piece images kept now, of `pixels` device pixels.
#[cfg(any(test, feature = "test-support"))]
pub fn kept_images(pixels: u32, cx: &mut App) -> Vec<Arc<RenderImage>> {
    cx.default_global::<PieceImages>()
        .images
        .iter()
        .filter(|((_, size), _)| *size == pixels)
        .map(|(_, image)| Arc::clone(image))
        .collect()
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
        let dropped = cx
            .default_global::<PieceImages>()
            .keep((path, pixels), Arc::clone(&image));
        release(dropped, cx);
        return Some(image);
    };
    rasterize_later(piece_set(set), pixels, viewer, cx);
    Some(nearest)
}

fn rasterize(path: &str, pixels: u32, renderer: &SvgRenderer) -> Option<Arc<RenderImage>> {
    let bytes = crate::assets::shared(path)?;
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
            let mut dropped = Vec::new();
            for (path, image) in rasterized {
                dropped.extend(images.keep((path, pixels), image));
            }
            let mut waiting = images.pending.remove(&key).unwrap_or_default();
            waiting.append(&mut images.deferred);
            release(dropped, cx);
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
                    assert!(crate::assets::shared(&path).is_some(), "{path}");
                }
            }
        }
        for board in &LOOKS.boards {
            if let Some(path) = board.surface.image_path() {
                assert!(crate::assets::shared(&path).is_some(), "{path}");
            }
        }
        // Every embedded board image is one a board shows.
        for path in crate::assets::paths().filter(|path| path.starts_with("boards/")) {
            assert!(
                LOOKS
                    .boards
                    .iter()
                    .any(|board| board.surface.image_path().as_deref() == Some(&*path)),
                "{path} is embedded but no board shows it"
            );
        }
    }

    /// The drawn boards' colours, read from the pixels of the Lichess images they replaced
    /// (2048² two-colour PNGs; the light square is the top left one).
    #[test]
    fn drawn_boards_keep_the_colours_of_their_old_images() {
        let old_pixels = [
            ("brown", 0xf0d9b5, 0xb58863),
            ("blue", 0xdee3e6, 0x8ca2ad),
            ("green", 0xffffdd, 0x86a666),
            ("purple", 0x9f90b0, 0x7d4a8d),
            ("ic", 0xececec, 0xc1c18e),
        ];
        for (id, light, dark) in old_pixels {
            assert_eq!(board(id).surface, Surface::Colors { light, dark }, "{id}");
        }
        let drawn = LOOKS
            .boards
            .iter()
            .filter(|board| matches!(board.surface, Surface::Colors { .. }))
            .count();
        assert_eq!(drawn, old_pixels.len());
        // The patterned purple keeps its image.
        assert_eq!(
            board("purple-diag").surface.image_path().as_deref(),
            Some("boards/purple-diag.png")
        );
    }

    /// Only the four corner squares are rounded, each on its outer corner: two dark ones here,
    /// and the light ones by the board's own rounded fill.
    #[test]
    fn a_drawn_board_rounds_its_corner_squares() {
        // A card's picture rounds only its top: each dark corner square takes its own corner.
        let board = gpui_kit::Corners {
            top_left: gpui_kit::px(8.),
            top_right: gpui_kit::px(8.),
            bottom_right: gpui_kit::px(0.),
            bottom_left: gpui_kit::px(6.),
        };
        let squares: Vec<_> = dark_squares(board).collect();
        assert_eq!(squares.len(), 32);
        for (column, row, corners) in squares {
            let expected = match (column, row) {
                (7, 0) => gpui_kit::Corners {
                    top_right: board.top_right,
                    ..Default::default()
                },
                (0, 7) => gpui_kit::Corners {
                    bottom_left: board.bottom_left,
                    ..Default::default()
                },
                _ => gpui_kit::Corners::default(),
            };
            assert_eq!(corners, expected, "{column},{row}");
        }
        assert!(
            !dark_square(0, 0) && !dark_square(7, 7),
            "the light corners"
        );
    }

    #[test]
    fn unknown_names_fall_back_to_the_defaults() {
        assert_eq!(board("gone").id, "brown");
        assert_eq!(piece_set("gone"), "cburnett");
    }
}
