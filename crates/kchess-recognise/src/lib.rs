//! A picture of a chess diagram to its position, on this computer alone: a screenshot of a 2D
//! board from a website, an app, an e-book or a video. No network, no model.
//!
//! 1. The board is found by its squares' edges (`grid`) and checked to be one: its 64 squares
//!    alternate between two colours (`squares::checkered`).
//! 2. Each square's own colour comes from a band inside its edges; what differs from it, filled
//!    in where an outline encloses it, is what stands there (`squares::silhouette`). Too little
//!    is an empty square.
//! 3. That silhouette, scaled to fit a small grid, and the brightness inside it are compared with
//!    every bundled piece set's twelve pieces, drawn at the square's size over the square's colour
//!    and read the same way (`Recogniser::references`). The nearest names the piece and its
//!    colour; one barely nearer than another piece is doubtful.
//! 4. The side at the bottom is the one whose pawns and king stand lower (`position`); a few
//!    rules then settle doubtful squares without inventing pieces: one king a side, no pawn on
//!    the first or last rank.
//!
//! What it measured on the app's own pictures is in `kchess-board`'s `picture` tests.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

mod grid;
mod picture;
mod position;
mod squares;
mod texture;

use std::sync::{Arc, Mutex};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use shakmaty::{Board, Color, Piece, Square};

use crate::picture::Picture;
use crate::squares::{Cell, Features};

/// Why no position came from a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a picture this can decode.
    Unreadable,
    /// A picture with no chessboard in it.
    NoBoard,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Unreadable => "That picture could not be read",
            Refusal::NoBoard => "No board found in the picture",
        })
    }
}

impl std::error::Error for Refusal {}

/// A piece set's twelve pieces as SVG, to compare pictures with.
pub struct PieceSet {
    pub name: String,
    pub pieces: Vec<(Piece, Vec<u8>)>,
}

/// The position read from a picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recognised {
    pub board: Board,
    /// White, unless Black is in check (then it must be Black's move).
    pub turn: Color,
    /// Castling where each king and rook stand on their starting squares.
    pub fen: String,
    /// Whether White's side was at the bottom of the picture.
    pub white_at_bottom: bool,
    /// Squares read with little certainty, to check.
    pub doubtful: Vec<Square>,
}

/// One bundled piece drawn at one size: premultiplied RGBA, 0 to 255.
struct Art {
    piece: Piece,
    rgba: Vec<[f32; 4]>,
}

/// One bundled piece as compared on one square colour.
struct Reference {
    piece: Piece,
    features: Features,
}

/// The sizes drawn last are kept, so pictures of the same size draw nothing again.
const KEPT_SIZES: usize = 3;
/// The largest a square is resampled to: more pixels tell no more about which piece it is.
const LARGEST_CELL: f32 = 72.;
/// The smallest: a thumbnail's squares are enlarged to this.
const SMALLEST_CELL: f32 = 24.;

/// Reads board pictures against a fixed list of piece sets.
pub struct Recogniser {
    sets: Vec<PieceSet>,
    drawn: Mutex<Vec<(usize, Arc<Vec<Art>>)>>,
}

impl Recogniser {
    pub fn new(sets: Vec<PieceSet>) -> Recogniser {
        Recogniser {
            sets,
            drawn: Mutex::new(Vec::new()),
        }
    }

    /// The names of the sets compared with, in order.
    pub fn set_names(&self) -> impl Iterator<Item = &str> {
        self.sets.iter().map(|set| set.name.as_str())
    }

    /// The position in the picture `bytes` (PNG, JPEG, GIF, WebP, BMP or TIFF).
    pub fn read(&self, bytes: &[u8]) -> Result<Recognised, Refusal> {
        let picture = Picture::decode(bytes).ok_or(Refusal::Unreadable)?;
        self.read_picture(&picture)
    }

    /// The position in a decoded picture.
    pub fn read_image(&self, image: &image::DynamicImage) -> Result<Recognised, Refusal> {
        self.read_picture(&Picture::of(image))
    }

    fn read_picture(&self, original: &Picture) -> Result<Recognised, Refusal> {
        if original.width < 8 * grid::MIN_SQUARE as usize
            || original.height < 8 * grid::MIN_SQUARE as usize
        {
            return Err(Refusal::NoBoard);
        }
        // A printed diagram's textured squares are read through their tone: the picture
        // averaged over the texture's period.
        let period = texture::period(&original.luma(), original.width, original.height);
        let averaged;
        let (picture, window) = match period {
            Some(period) => {
                let window = (period.round() as usize).max(2);
                averaged = texture::smooth(original, window);
                (&averaged, window)
            }
            None => (original, 0),
        };
        let (grid, colours) = grid::candidates(picture, 1 + window / 2)
            .into_iter()
            .take(6)
            .find_map(|grid| {
                let probe = cell_size(&grid);
                let cells = squares::cells(picture, &grid, probe, 0);
                squares::checkered(&cells).map(|colours| (grid, colours))
            })
            .ok_or(Refusal::NoBoard)?;
        let size = cell_size(&grid);
        // The texture's period in the cells' pixels.
        let blur = period.map_or(0, |period| {
            (period * size as f32 / ((grid.width + grid.height) / 2.)).round() as usize
        });
        let mut cells = squares::cells(picture, &grid, size, blur);
        // Paper and ink, on a textured picture: what is lighter than halfway between the light
        // squares and the texture's tone is paper.
        let paper = (picture::luma(colours[0]) + picture::luma(colours[1])) / 2.;
        if blur > 0 {
            squares::even_out(&mut cells, colours);
            for (cell, drawn) in cells
                .iter_mut()
                .zip(squares::cells(original, &grid, size, 0))
            {
                cell.crisp = drawn
                    .rgb
                    .iter()
                    .map(|&colour| picture::luma(colour))
                    .collect();
                cell.paper = paper;
            }
        }
        let reads = self.read_squares(&cells, size, colours, blur, paper);
        Ok(position::assemble(&reads))
    }

    /// The bundled pieces drawn at `size`, drawn once per size.
    fn art(&self, size: usize) -> Arc<Vec<Art>> {
        if let Ok(drawn) = self.drawn.lock()
            && let Some((_, art)) = drawn.iter().find(|(at, _)| *at == size)
        {
            return art.clone();
        }
        let art: Arc<Vec<Art>> = Arc::new(
            self.sets
                .iter()
                .flat_map(|set| {
                    set.pieces.iter().filter_map(move |(piece, svg)| {
                        draw(svg, size).map(|rgba| Art {
                            piece: *piece,
                            rgba,
                        })
                    })
                })
                .collect(),
        );
        if let Ok(mut drawn) = self.drawn.lock() {
            drawn.retain(|(at, _)| *at != size);
            drawn.push((size, art.clone()));
            if drawn.len() > KEPT_SIZES {
                drawn.remove(0);
            }
        }
        art
    }

    /// Every bundled piece over `background`, read as a square of the picture with that colour
    /// and `spread` is: on a textured picture (`blur` > 0), averaged as the picture was, its
    /// paper lighter than `paper`.
    fn references(
        &self,
        art: &[Art],
        size: usize,
        background: [f32; 3],
        spread: f32,
        blur: usize,
        paper: f32,
    ) -> Vec<Reference> {
        art.iter()
            .filter_map(|art| {
                let mut rgb: Vec<[f32; 3]> = art
                    .rgba
                    .iter()
                    .map(|&[r, g, b, a]| {
                        let under = 1. - a / 255.;
                        [
                            r + under * background[0],
                            g + under * background[1],
                            b + under * background[2],
                        ]
                    })
                    .collect();
                let mut crisp = Vec::new();
                if blur > 0 {
                    crisp = rgb.iter().map(|&c| picture::luma(c)).collect();
                    let drawn = picture::Picture {
                        width: size,
                        height: size,
                        rgb,
                    };
                    rgb = texture::smooth(&drawn, blur).rgb;
                }
                let cell = Cell {
                    rgb,
                    background,
                    spread,
                    blur,
                    crisp,
                    paper,
                };
                let shape = squares::silhouette(size, &cell, &[]).ok()?;
                Some(Reference {
                    piece: art.piece,
                    features: squares::features(&shape, &cell.rgb),
                })
            })
            .collect()
    }

    /// What stands on each of `cells`, the board's colours `colours` (light, dark).
    fn read_squares(
        &self,
        cells: &[Cell],
        size: usize,
        colours: [[f32; 3]; 2],
        blur: usize,
        paper: f32,
    ) -> Vec<position::Read> {
        let art = self.art(size);
        // The pieces over each colour the squares show, and with each grain: the board's two
        // colours, then others (a highlighted move, a textured board's patches) as they come.
        let mut palette: Vec<([f32; 3], f32, Vec<Reference>)> = colours
            .iter()
            .map(|&colour| {
                (
                    colour,
                    0.,
                    self.references(&art, size, colour, 0., blur, paper),
                )
            })
            .collect();
        let labels = squares::labels(cells, size);
        let mut reads = Vec::with_capacity(cells.len());
        for cell in cells {
            let shape = match squares::silhouette(size, cell, &labels[reads.len()]) {
                Ok(shape) => shape,
                Err(mark) => {
                    reads.push(position::Read::Empty {
                        doubtful: mark >= squares::PIECE_AREA * 0.6,
                    });
                    continue;
                }
            };
            let features = squares::features(&shape, &cell.rgb);
            let nearest = palette
                .iter()
                .enumerate()
                .filter(|(_, (_, spread, _))| (spread - cell.spread).abs() <= 1.5)
                .map(|(index, (colour, ..))| (index, picture::distance(*colour, cell.background)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let index = match nearest {
                Some((index, apart)) if apart <= 24. => index,
                _ => {
                    let references =
                        self.references(&art, size, cell.background, cell.spread, blur, paper);
                    palette.push((cell.background, cell.spread, references));
                    palette.len() - 1
                }
            };
            // Each piece's nearest drawing among the sets.
            let mut best = [f32::MAX; 12];
            for reference in &palette[index].2 {
                let slot = piece_index(reference.piece);
                best[slot] = best[slot].min(squares::unlike(&features, &reference.features));
            }
            let mut ranked: Vec<(Piece, f32)> = best
                .iter()
                .enumerate()
                .filter(|(_, apart)| **apart < f32::MAX)
                .map(|(slot, apart)| (piece_of(slot), *apart))
                .collect();
            ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
            if ranked.is_empty() {
                reads.push(position::Read::Empty { doubtful: true });
                continue;
            }
            let doubtful =
                shape.area < squares::PIECE_AREA * 1.5 || position::doubtful_ranking(&ranked);
            reads.push(position::Read::Piece { ranked, doubtful });
        }
        reads
    }
}

/// The size a square of `grid` is resampled to.
fn cell_size(grid: &grid::Grid) -> usize {
    ((grid.width + grid.height) / 2.)
        .clamp(SMALLEST_CELL, LARGEST_CELL)
        .round() as usize
}

/// `svg` drawn `size` pixels square: premultiplied RGBA.
fn draw(svg: &[u8], size: usize) -> Option<Vec<[f32; 4]>> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(size as u32, size as u32)?;
    let drawn = tree.size();
    resvg::render(
        &tree,
        Transform::from_scale(size as f32 / drawn.width(), size as f32 / drawn.height()),
        &mut pixmap.as_mut(),
    );
    Some(
        pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                [
                    f32::from(p[0]),
                    f32::from(p[1]),
                    f32::from(p[2]),
                    f32::from(p[3]),
                ]
            })
            .collect(),
    )
}

/// The twelve pieces in a fixed order: White's then Black's, pawn to king.
pub(crate) fn piece_of(index: usize) -> Piece {
    Piece {
        color: if index < 6 {
            Color::White
        } else {
            Color::Black
        },
        role: shakmaty::Role::ALL[index % 6],
    }
}

pub(crate) fn piece_index(piece: Piece) -> usize {
    let role = shakmaty::Role::ALL
        .iter()
        .position(|role| *role == piece.role)
        .unwrap_or(0);
    role + if piece.color == Color::White { 0 } else { 6 }
}

#[cfg(test)]
mod tests;
