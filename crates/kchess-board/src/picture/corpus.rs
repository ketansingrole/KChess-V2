//! How well a board picture is read, measured on the app's own pictures: random legal positions
//! drawn by the image export (`game_image::position_png`) on every board theme with every
//! bundled piece set, at several sizes, from either side, with and without the last move
//! highlighted, with coordinate labels on the edge squares, on a page with a margin around it,
//! scaled, and saved as JPEG.
//!
//! `KCHESS_RECOGNISE_DUMP=<dir>` writes each picture read wrongly there, named after its case.

use image::{DynamicImage, Rgba, RgbaImage};
use kchess_testkit::random::Rng;
use shakmaty::{Board, CastlingMode, Chess, Color, EnPassantMode, Position as _};

use crate::board::looks;
use crate::game_image::{ImageOptions, position_png};

/// What is done to a picture after it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// As drawn.
    None,
    /// Coordinate labels in the corners of the edge squares, as many boards show them.
    Coordinates,
    /// On a page, a margin of its colour around it.
    Margin,
    /// Scaled to 85% or 125%, as a screenshot of a zoomed page is.
    Scaled,
    /// Saved as JPEG at quality 75.
    Jpeg,
}

pub(super) const CHANGES: [Change; 5] = [
    Change::None,
    Change::Coordinates,
    Change::Margin,
    Change::Scaled,
    Change::Jpeg,
];

/// One picture to read.
#[derive(Clone, Debug)]
pub(super) struct Case {
    pub set: String,
    pub board: String,
    pub size: u32,
    pub change: Change,
    pub seed: u64,
}

/// What one picture was read as.
pub(super) struct Outcome {
    pub case: Case,
    pub found: bool,
    pub squares_right: usize,
    pub position_right: bool,
    pub doubtful: usize,
    /// Doubtful squares among the ones read wrongly.
    pub wrong_and_doubtful: usize,
}

/// A random legal position: a random game of 4 to 70 moves (fewer if it ends), its moves as UCI
/// and the board after them.
fn random_game(rng: &mut Rng) -> (Vec<String>, Chess) {
    let mut position = Chess::default();
    let mut moves = Vec::new();
    let plies = 4 + rng.below(67);
    for _ in 0..plies {
        let legal = position.legal_moves();
        let Some(chosen) = rng.pick(&legal).copied() else {
            break;
        };
        moves.push(chosen.to_uci(CastlingMode::Standard).to_string());
        position.play_unchecked(chosen);
    }
    (moves, position)
}

fn hex(text: &str) -> [u8; 3] {
    let value = u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap_or(0);
    let [_, r, g, b] = value.to_be_bytes();
    [r, g, b]
}

/// Labels like a board's coordinates: a small letter-like mark in the bottom right corner of each
/// square on the bottom rank and the top left of each on the left file, in the board's
/// coordinate colours.
fn coordinates(image: &mut RgbaImage, top: u32, square: u32, board: &str) {
    let look = looks::board(board);
    let (on_dark, on_light) = (hex(&look.coord_white), hex(&look.coord_black));
    let stroke = (square / 24).max(1);
    let (wide, high) = ((square * 14 / 100).max(3), (square * 18 / 100).max(4));
    let mut label = |left: u32, label_top: u32, dark: bool| {
        let [r, g, b] = if dark { on_dark } else { on_light };
        for y in label_top..label_top + high {
            for x in left..left + wide {
                // An "L" and a bar: a letter's strokes, roughly.
                let edge =
                    x < left + stroke || y >= label_top + high - stroke || y < label_top + stroke;
                if edge && x < image.width() && y < image.height() {
                    image.put_pixel(x, y, Rgba([r, g, b, 255]));
                }
            }
        }
    };
    for column in 0..8 {
        let left = (column + 1) * square - wide - square * 6 / 100;
        let label_top = top + 8 * square - high - square * 5 / 100;
        label(left, label_top, looks::dark_square(column, 7));
    }
    for row in 0..8 {
        label(
            square * 6 / 100,
            top + row * square + square * 5 / 100,
            looks::dark_square(0, row),
        );
    }
}

/// `image` on a page of a random colour, with a random margin around it.
fn on_a_page(image: &RgbaImage, rng: &mut Rng) -> RgbaImage {
    let pages = [[255, 255, 255], [245, 245, 245], [22, 21, 18], [49, 46, 43]];
    let [r, g, b] = *rng.pick(&pages).unwrap_or(&pages[0]);
    let (left, top) = (8 + rng.below(60) as u32, 8 + rng.below(60) as u32);
    let (right, bottom) = (8 + rng.below(60) as u32, 8 + rng.below(60) as u32);
    let mut page = RgbaImage::from_pixel(
        image.width() + left + right,
        image.height() + top + bottom,
        Rgba([r, g, b, 255]),
    );
    image::imageops::overlay(&mut page, image, i64::from(left), i64::from(top));
    page
}

/// The picture `case` names, and the position on it.
pub(super) fn draw(case: &Case) -> (DynamicImage, Board) {
    let mut rng = Rng::new(case.seed);
    let (moves, position) = random_game(&mut rng);
    let highlight = rng.chance(50);
    let orientation = if rng.chance(50) {
        Color::White
    } else {
        Color::Black
    };
    let fen = shakmaty::fen::Fen::from_position(&position, EnPassantMode::Legal).to_string();
    let options = ImageOptions {
        setup: if highlight {
            kchess_domain::variant::Setup::standard()
        } else {
            kchess_domain::variant::Setup::standard_at(fen)
        },
        moves: if highlight { moves.clone() } else { Vec::new() },
        orientation,
        board_theme: case.board.clone(),
        piece_set: case.set.clone(),
        white: String::new(),
        black: String::new(),
        size: case.size,
    };
    let png = position_png(&options, options.moves.len()).expect("the picture is drawn");
    let mut image = image::load_from_memory(&png)
        .expect("the export decodes")
        .to_rgba8();
    match case.change {
        Change::None => {}
        Change::Coordinates => coordinates(&mut image, 28, case.size / 8, &case.board),
        Change::Margin => image = on_a_page(&image, &mut rng),
        Change::Scaled => {
            let factor = if rng.chance(50) { 0.85 } else { 1.25 };
            image = image::imageops::resize(
                &image,
                (image.width() as f32 * factor).round() as u32,
                (image.height() as f32 * factor).round() as u32,
                image::imageops::FilterType::Triangle,
            );
        }
        Change::Jpeg => {
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 75)
                .encode_image(&DynamicImage::ImageRgba8(image.clone()).to_rgb8())
                .expect("the JPEG is written");
            image = image::load_from_memory(&jpeg)
                .expect("the JPEG decodes")
                .to_rgba8();
        }
    }
    (DynamicImage::ImageRgba8(image), position.board().clone())
}

/// `case` drawn and read.
pub(super) fn run(case: Case) -> Outcome {
    let (image, expected) = draw(&case);
    let read = super::RECOGNISER.read_image(&image);
    let outcome = match &read {
        Ok(recognised) => {
            let squares_right = shakmaty::Square::ALL
                .iter()
                .filter(|&&square| recognised.board.piece_at(square) == expected.piece_at(square))
                .count();
            let wrong_and_doubtful = recognised
                .doubtful
                .iter()
                .filter(|&&square| recognised.board.piece_at(square) != expected.piece_at(square))
                .count();
            Outcome {
                found: true,
                squares_right,
                position_right: recognised.board == expected,
                doubtful: recognised.doubtful.len(),
                wrong_and_doubtful,
                case,
            }
        }
        Err(_) => Outcome {
            found: false,
            squares_right: 0,
            position_right: false,
            doubtful: 0,
            wrong_and_doubtful: 0,
            case,
        },
    };
    if !outcome.position_right
        && let Some(dir) = kchess_host::env::var("KCHESS_RECOGNISE_DUMP")
    {
        let case = &outcome.case;
        let name = format!(
            "{}-{}-{}-{:?}-{}.png",
            case.set, case.board, case.size, case.change, case.seed
        );
        image
            .save(std::path::Path::new(&dir).join(name))
            .expect("the picture is saved");
        if let Ok(recognised) = &read {
            let wrong: Vec<String> = shakmaty::Square::ALL
                .iter()
                .filter(|&&square| recognised.board.piece_at(square) != expected.piece_at(square))
                .map(|&square| {
                    let name = |piece: Option<shakmaty::Piece>| piece.map_or('.', |p| p.char());
                    format!(
                        "{square}:{}>{}",
                        name(expected.piece_at(square)),
                        name(recognised.board.piece_at(square))
                    )
                })
                .collect();
            eprintln!(
                "{} {} {} {:?} {}: {}",
                case.set,
                case.board,
                case.size,
                case.change,
                case.seed,
                wrong.join(" ")
            );
        }
    }
    outcome
}

/// Every case read, on as many threads as there are cores.
pub(super) fn run_all(cases: Vec<Case>) -> Vec<Outcome> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = cases.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let running: Vec<_> = cases
            .chunks(chunk)
            .map(|cases| scope.spawn(move || cases.iter().cloned().map(run).collect::<Vec<_>>()))
            .collect();
        running
            .into_iter()
            .flat_map(|thread| thread.join().expect("a reading thread ends"))
            .collect()
    })
}

/// The totals of `outcomes` grouped by `key`: pictures, boards found, squares and positions read
/// right, doubtful squares.
pub(super) fn table(outcomes: &[Outcome], title: &str, key: impl Fn(&Case) -> String) -> String {
    let mut groups: Vec<(String, Vec<&Outcome>)> = Vec::new();
    for outcome in outcomes {
        let name = key(&outcome.case);
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, list)) => list.push(outcome),
            None => groups.push((name, vec![outcome])),
        }
    }
    let mut text = format!(
        "{title:<14} {:>6} {:>7} {:>9} {:>10} {:>9} {:>10}\n",
        "images", "found", "squares", "positions", "doubtful", "wrong/dbt"
    );
    for (name, list) in groups {
        let (found, squares, positions, doubtful, wrong_doubtful, wrong) =
            list.iter().fold((0, 0, 0, 0, 0, 0), |sum, o| {
                (
                    sum.0 + usize::from(o.found),
                    sum.1 + o.squares_right,
                    sum.2 + usize::from(o.position_right),
                    sum.3 + o.doubtful,
                    sum.4 + o.wrong_and_doubtful,
                    sum.5 + (64 - o.squares_right),
                )
            });
        let total = list.len();
        text.push_str(&format!(
            "{name:<14} {total:>6} {found:>7} {:>8.2}% {:>9.1}% {:>9.2}% {:>5}/{:<4}\n",
            100. * squares as f64 / (64 * total) as f64,
            100. * positions as f64 / total as f64,
            100. * doubtful as f64 / (64 * total) as f64,
            wrong_doubtful,
            wrong,
        ));
    }
    text
}
