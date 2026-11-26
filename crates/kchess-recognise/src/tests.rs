//! Pictures drawn here with bundled sets, read whole. How well the app's own pictures are read,
//! on every set and board, is measured in `kchess-board`'s `picture` tests.

use super::*;
use kchess_testkit::random::Rng;
use shakmaty::Role;

/// A bundled set, read from the app's assets.
fn set(name: &str) -> PieceSet {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/gpui/assets/pieces")
        .join(name);
    let pieces = Color::ALL
        .into_iter()
        .flat_map(|color| Role::ALL.into_iter().map(move |role| Piece { color, role }))
        .map(|piece| {
            let color = if piece.color == Color::White {
                'w'
            } else {
                'b'
            };
            let role = piece.role.upper_char();
            let svg = std::fs::read(folder.join(format!("{color}{role}.svg"))).unwrap();
            (piece, svg)
        })
        .collect();
    PieceSet {
        name: name.into(),
        pieces,
    }
}

fn recogniser() -> &'static Recogniser {
    static RECOGNISER: std::sync::LazyLock<Recogniser> = std::sync::LazyLock::new(|| {
        Recogniser::new(vec![set("cburnett"), set("merida"), set("letter")])
    });
    &RECOGNISER
}

/// `board` drawn with `pieces` on light and dark squares of `square` pixels, from White's side
/// or Black's, with `margin` pixels of white page around it.
fn picture(
    board: &Board,
    pieces: &PieceSet,
    square: usize,
    white_at_bottom: bool,
    margin: u32,
) -> image::RgbaImage {
    let side = (square * 8) as u32;
    let mut image =
        image::RgbaImage::from_pixel(side + 2 * margin, side + 2 * margin, image::Rgba([255; 4]));
    for row in 0..8u32 {
        for column in 0..8u32 {
            let (file, rank) = if white_at_bottom {
                (column, 7 - row)
            } else {
                (7 - column, row)
            };
            let colour = if (row + column) % 2 == 1 {
                [181, 136, 99, 255]
            } else {
                [240, 217, 181, 255]
            };
            let size = square as u32;
            let (left, top) = (margin + column * size, margin + row * size);
            for y in 0..size {
                for x in 0..size {
                    image.put_pixel(left + x, top + y, image::Rgba(colour));
                }
            }
            let at = Square::from_coords(shakmaty::File::new(file), shakmaty::Rank::new(rank));
            let Some(piece) = board.piece_at(at) else {
                continue;
            };
            let svg = &pieces
                .pieces
                .iter()
                .find(|(drawn, _)| *drawn == piece)
                .unwrap()
                .1;
            for (index, [r, g, b, a]) in draw(svg, square).unwrap().into_iter().enumerate() {
                let (x, y) = (
                    left + (index % square) as u32,
                    top + (index / square) as u32,
                );
                let under = image.get_pixel(x, y).0;
                let over = |c: f32, u: u8| (c + (1. - a / 255.) * f32::from(u)).round() as u8;
                image.put_pixel(
                    x,
                    y,
                    image::Rgba([over(r, under[0]), over(g, under[1]), over(b, under[2]), 255]),
                );
            }
        }
    }
    image
}

fn png(image: &image::RgbaImage) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A position drawn with a bundled set is read square for square, from either side, with
/// castling where king and rook stand at home.
#[test]
fn a_drawn_position_is_read_from_either_side() {
    let fen = "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 0 1";
    let board = fen
        .parse::<shakmaty::fen::Fen>()
        .unwrap()
        .into_setup()
        .board;
    for (name, square, white_at_bottom) in [
        ("cburnett", 50, true),
        ("merida", 40, false),
        ("letter", 64, true),
    ] {
        let pieces = set(name);
        let read = recogniser()
            .read(&png(&picture(&board, &pieces, square, white_at_bottom, 17)))
            .unwrap();
        assert_eq!(read.fen, fen, "{name}");
        assert_eq!(read.white_at_bottom, white_at_bottom, "{name}");
        assert_eq!(read.turn, Color::White);
        assert!(read.doubtful.is_empty(), "{name}: {:?}", read.doubtful);
    }
    assert_eq!(
        recogniser().set_names().collect::<Vec<_>>(),
        ["cburnett", "merida", "letter"]
    );
}

/// A blank page, photo-like pictures of soft colour and grain, a picture too small for a board
/// and bytes that are no picture are refused.
#[test]
fn pictures_without_a_board_are_refused() {
    let blank = image::RgbaImage::from_pixel(400, 300, image::Rgba([250, 250, 250, 255]));
    assert_eq!(recogniser().read(&png(&blank)), Err(Refusal::NoBoard));
    let tiny = image::RgbaImage::from_pixel(40, 40, image::Rgba([0, 0, 0, 255]));
    assert_eq!(recogniser().read(&png(&tiny)), Err(Refusal::NoBoard));
    assert_eq!(
        recogniser().read(b"GIF89a, then nothing"),
        Err(Refusal::Unreadable)
    );
    kchess_testkit::random::check(3, |rng: &mut Rng| {
        // Soft patches of colour (a coarse grid of random colours, blended) and grain.
        let grid: Vec<[f32; 3]> = (0..36)
            .map(|_| [0, 1, 2].map(|_| rng.below(256) as f32))
            .collect();
        let at = |x: usize, y: usize| grid[y.min(5) * 6 + x.min(5)];
        let mut photo = image::RgbaImage::new(500, 500);
        for y in 0..500u32 {
            for x in 0..500u32 {
                let (fx, fy) = (x as f32 / 100., y as f32 / 100.);
                let (cx, cy, tx, ty) = (fx as usize, fy as usize, fx.fract(), fy.fract());
                let mut pixel = [0u8, 0, 0, 255];
                for (c, channel) in pixel.iter_mut().take(3).enumerate() {
                    let top = at(cx, cy)[c] * (1. - tx) + at(cx + 1, cy)[c] * tx;
                    let bottom = at(cx, cy + 1)[c] * (1. - tx) + at(cx + 1, cy + 1)[c] * tx;
                    let grain = rng.below(31) as f32 - 15.;
                    *channel = (top * (1. - ty) + bottom * ty + grain).clamp(0., 255.) as u8;
                }
                photo.put_pixel(x, y, image::Rgba(pixel));
            }
        }
        assert_eq!(recogniser().read(&png(&photo)), Err(Refusal::NoBoard));
    });
    assert_eq!(
        Refusal::NoBoard.to_string(),
        "No board found in the picture"
    );
    assert_eq!(
        Refusal::Unreadable.to_string(),
        "That picture could not be read"
    );
}
