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

/// Coordinate labels in the corners of the edge squares, as a website draws them in the other
/// square's colour, are left out of the squares, a piece standing on a labelled square included.
#[test]
fn coordinate_labels_are_left_out() {
    let fen = "r3k2r/pppq1ppp/2n5/8/8/2N5/PPPQ1PPP/R3K2R w KQkq - 0 1";
    let board = fen
        .parse::<shakmaty::fen::Fen>()
        .unwrap()
        .into_setup()
        .board;
    let pieces = set("cburnett");
    let square = 60u32;
    let margin = 10;
    let mut image = picture(&board, &pieces, square as usize, true, margin);
    let light = image::Rgba([240, 217, 181, 255]);
    let dark = image::Rgba([181, 136, 99, 255]);
    // A small mark like a digit or a letter, `width` by `height` pixels at `(left, top)`.
    let mut mark = |left: u32, top: u32, width: u32, height: u32, row: u32, column: u32| {
        let colour = if (row + column) % 2 == 1 { light } else { dark };
        for y in top..top + height {
            for x in left..left + width {
                // A hollow box, as a digit's outline would leave the square showing inside.
                if x == left || x + 1 == left + width || y == top || y + 1 == top + height {
                    image.put_pixel(x, y, colour);
                }
            }
        }
    };
    for row in 0..8 {
        // The ranks down the left edge, in each square's top left corner.
        mark(margin + 3, margin + row * square + 3, 7, 11, row, 0);
    }
    for column in 0..8 {
        // The files along the bottom edge, in each square's bottom right corner.
        mark(
            margin + column * square + square - 10,
            margin + 7 * square + square - 12,
            7,
            9,
            7,
            column,
        );
    }
    let read = recogniser()
        .read_image(&image::DynamicImage::ImageRgba8(image))
        .unwrap();
    assert_eq!(read.fen, fen);
    assert!(read.white_at_bottom);
    assert!(read.doubtful.is_empty(), "{:?}", read.doubtful);
}

/// How a printed diagram is drawn: dark squares hatched with lines `spacing` pixels apart along
/// a row and `thickness` thick, rising (`/`) or falling (`\`), on squares of `square` pixels,
/// White's figurines outlined `outline` of a square thick; in black and white alone (a 1-bit
/// scan) or with grey edges.
#[derive(Clone, Copy, Debug)]
struct Print {
    square: usize,
    spacing: f32,
    thickness: f32,
    rising: bool,
    outline: f32,
    one_bit: bool,
}

/// How much finer than the picture a printed diagram is drawn, for its grey edges.
const FINE: usize = 4;

/// `piece` of `pieces` as a book's figurine at `size` pixels: which pixels are ink and which
/// are the piece at all. White's is an outline `outline` pixels wide filled white, Black's
/// solid ink.
fn figurine(
    pieces: &PieceSet,
    piece: Piece,
    size: usize,
    outline: usize,
) -> (Vec<bool>, Vec<bool>) {
    let svg = &pieces
        .pieces
        .iter()
        .find(|(drawn, _)| *drawn == piece)
        .unwrap()
        .1;
    let inside: Vec<bool> = draw(svg, size)
        .unwrap()
        .iter()
        .map(|p| p[3] > 127.)
        .collect();
    if piece.color == Color::Black {
        return (inside.clone(), inside);
    }
    // White's: the piece less itself shrunk by the outline's width.
    let shrink = |on: &[bool], across: bool| -> Vec<bool> {
        (0..size * size)
            .map(|index| {
                let (x, y) = (index % size, index / size);
                let (along, line) = if across { (x, y) } else { (y, x) };
                let low = along.saturating_sub(outline);
                let high = (along + outline).min(size - 1);
                (low..=high).all(|at| {
                    let (x, y) = if across { (at, line) } else { (line, at) };
                    on[y * size + x]
                }) && along >= outline
                    && along + outline < size
            })
            .collect()
    };
    let shrunk = shrink(&shrink(&inside, true), false);
    let ink = inside
        .iter()
        .zip(&shrunk)
        .map(|(on, filled)| *on && !filled)
        .collect();
    (ink, inside)
}

/// `board` printed as a book prints a diagram, White at the bottom: white light squares,
/// hatched dark ones, a thick frame, the files' and ranks' marks outside it, and the pieces of
/// `pieces` as figurines; on a white page.
fn printed(board: &Board, pieces: &PieceSet, print: Print) -> image::GrayImage {
    let Print {
        square, spacing, ..
    } = print;
    let margin = square;
    let side = 8 * square + 2 * margin;
    let fine = side * FINE;
    let mut ink = vec![false; fine * fine];
    let frame = (square / 12).max(2) as f32;
    let outline = ((square * FINE) as f32 * print.outline).ceil() as usize;
    let mut figurines = std::collections::HashMap::new();
    for y in 0..fine {
        for x in 0..fine {
            // Where this fine pixel's middle is, in the picture's pixels from the board's corner.
            let u = (x as f32 + 0.5) / FINE as f32 - margin as f32;
            let v = (y as f32 + 0.5) / FINE as f32 - margin as f32;
            let board_side = (8 * square) as f32;
            let on_board = (0. ..board_side).contains(&u) && (0. ..board_side).contains(&v);
            if !on_board {
                let framed = (-frame..board_side + frame).contains(&u)
                    && (-frame..board_side + frame).contains(&v);
                ink[y * fine + x] = framed;
                continue;
            }
            let (column, row) = ((u as usize) / square, (v as usize) / square);
            let at = Square::from_coords(
                shakmaty::File::new(column as u32),
                shakmaty::Rank::new(7 - row as u32),
            );
            if let Some(piece) = board.piece_at(at) {
                let size = square * FINE;
                let (drawn, whole) = figurines
                    .entry(piece)
                    .or_insert_with(|| figurine(pieces, piece, size, outline));
                let (fx, fy) = (
                    x - (margin + column * square) * FINE,
                    y - (margin + row * square) * FINE,
                );
                if whole[fy * size + fx] {
                    ink[y * fine + x] = drawn[fy * size + fx];
                    continue;
                }
            }
            if (row + column) % 2 == 1 {
                let along = if print.rising {
                    u + v
                } else {
                    u - v + board_side
                };
                ink[y * fine + x] = along % spacing < print.thickness;
            }
        }
    }
    let mut image = image::GrayImage::from_pixel(side as u32, side as u32, image::Luma([255]));
    for y in 0..side {
        for x in 0..side {
            let inked = (0..FINE * FINE)
                .filter(|k| ink[(y * FINE + k / FINE) * fine + x * FINE + k % FINE])
                .count();
            let mut grey = 255. * (1. - inked as f32 / (FINE * FINE) as f32);
            if print.one_bit {
                grey = if grey < 128. { 0. } else { 255. };
            }
            image.put_pixel(x as u32, y as u32, image::Luma([grey.round() as u8]));
        }
    }
    // The files under the frame and the ranks right of it: marks the size of a small letter.
    let (wide, high) = ((square / 4).max(3) as u32, (square / 3).max(4) as u32);
    let stroke = (square / 20).max(1) as u32;
    let mut mark = |left: u32, top: u32| {
        for y in top..top + high {
            for x in left..left + wide {
                if x < left + stroke || y < top + stroke || y + stroke >= top + high {
                    image.put_pixel(x, y, image::Luma([0]));
                }
            }
        }
    };
    let gap = (square / 4) as u32;
    let (margin, square) = (margin as u32, square as u32);
    for k in 0..8 {
        mark(
            margin + k * square + square / 2 - wide / 2,
            margin + 8 * square + gap,
        );
        mark(
            margin + 8 * square + gap,
            margin + k * square + square / 2 - high / 2,
        );
    }
    image
}

/// A random position from a random game of 4 to 70 moves, fewer if it ends.
fn random_position(rng: &mut Rng) -> Board {
    use shakmaty::Position as _;
    let mut position = shakmaty::Chess::default();
    for _ in 0..4 + rng.below(67) {
        let legal = position.legal_moves();
        let Some(chosen) = rng.pick(&legal).copied() else {
            break;
        };
        position.play_unchecked(chosen);
    }
    position.board().clone()
}

/// Printed diagrams, as books print them: hatched dark squares at several spacings and line
/// widths, a frame with the coordinates outside it and figurines no bundled set draws (White's
/// an outline filled white, Black's solid), in black and white or with grey edges, on squares of
/// 22 to 64 pixels. Every board is found. The squares read wrongly are on hatched squares and
/// mostly in black and white alone: there each line is drawn a whole number of pixels thick,
/// so the hatching, averaged, stays uneven (by a third of its tone on 4-pixel lines), and only
/// what differs more than that is read, which rounds off small parts of a piece such as a
/// queen's points. A scanned book's hatching is uneven at random instead, and reads better.
#[test]
fn printed_diagrams_are_read() {
    // Measured 2026-11-26 over these 48 pictures: 95.57% of squares, 25.0% of positions, 88% of
    // the wrong squares doubtful.
    const SQUARES: f32 = 95.;
    const POSITIONS: f32 = 20.;
    const DOUBTED: f32 = 85.;
    let sets = [set("cburnett"), set("merida")];
    #[derive(Default)]
    struct Tally {
        pictures: usize,
        found: usize,
        right: usize,
        positions: usize,
        wrong: usize,
        doubted: usize,
    }
    let tally = std::cell::RefCell::new(Tally::default());
    kchess_testkit::random::check(48, |rng: &mut Rng| {
        let square = 22 + rng.below(43);
        let one_bit = rng.chance(50);
        // Lines closer than 3.5 pixels in black and white alone are not a tone: each is one
        // pixel or two thick, and which slowly changes across the board, so the dark squares
        // of one part of it are twice as dark as another's.
        let spacing =
            (square as f32 / (6. + rng.below(50) as f32 / 10.)).max(if one_bit { 3.5 } else { 2. });
        let print = Print {
            square,
            spacing,
            thickness: (spacing * (0.3 + rng.below(30) as f32 / 100.)).max(1.),
            rising: rng.chance(70),
            outline: 1. / (16 + rng.below(11)) as f32,
            one_bit,
        };
        let board = random_position(rng);
        let pieces = rng.pick(&sets).unwrap();
        let image = printed(&board, pieces, print);
        let found = recogniser().read_image(&image::DynamicImage::ImageLuma8(image));
        let mut tally = tally.borrow_mut();
        tally.pictures += 1;
        let Ok(found) = found else {
            eprintln!("{print:?} {}: no board", pieces.name);
            return;
        };
        tally.found += 1;
        let wrong: Vec<Square> = Square::ALL
            .into_iter()
            .filter(|&at| found.board.piece_at(at) != board.piece_at(at))
            .collect();
        tally.right += 64 - wrong.len();
        tally.positions += usize::from(wrong.is_empty());
        tally.wrong += wrong.len();
        tally.doubted += wrong
            .iter()
            .filter(|at| found.doubtful.contains(at))
            .count();
        eprintln!(
            "{print:?} {}: {} of 64 {}",
            pieces.name,
            64 - wrong.len(),
            found.fen
        );
    });
    let tally = tally.into_inner();
    let squares = 100. * tally.right as f32 / (64 * tally.pictures) as f32;
    let positions = 100. * tally.positions as f32 / tally.pictures as f32;
    let doubted = 100. * tally.doubted as f32 / tally.wrong.max(1) as f32;
    eprintln!(
        "{} of {} boards found, {squares:.2}% of squares, {positions:.1}% of positions, \
         {doubted:.0}% of wrong squares doubtful",
        tally.found, tally.pictures
    );
    assert_eq!(tally.found, tally.pictures, "a board was not found");
    assert!(squares >= SQUARES, "{squares:.2}% of squares");
    assert!(positions >= POSITIONS, "{positions:.1}% of positions");
    assert!(
        doubted >= DOUBTED,
        "{doubted:.0}% of wrong squares doubtful"
    );
}
