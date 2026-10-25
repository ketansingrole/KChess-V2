//! Pictures of a game (`utils/gameImage.ts`): a PNG of one position, or an animated GIF of the
//! whole game, on the player's board and pieces with the players' names above and below.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageEncoder as _, RgbaImage};
use resvg::tiny_skia::{Color as Paint, IntSize, Pixmap, PixmapPaint, Rect, Transform};
use resvg::usvg;
use shakmaty::{Board, Color, Piece, Position as _, Square};

use crate::board::looks;

/// The bars above and below the board that carry the players' names.
const BAR: u32 = 28;
/// The longest game a GIF is made of (300 moves).
const MAX_FRAMES: usize = 600;

/// What to draw (`ImageOptions`).
#[derive(Clone)]
pub struct ImageOptions {
    pub variant: String,
    pub fen: String,
    pub moves: Vec<String>,
    pub orientation: Color,
    pub board_theme: String,
    pub piece_set: String,
    pub white: String,
    pub black: String,
    /// Board width in pixels.
    pub size: u32,
}

/// The system's fonts, read once, for the players' names.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

fn escape(text: &str) -> String {
    text.chars()
        .take(48)
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// An SVG drawn `width` × `height` pixels.
fn rasterize(svg: &[u8], width: u32, height: u32) -> Result<Pixmap, String> {
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(svg, &options).map_err(|cause| cause.to_string())?;
    let mut pixmap = Pixmap::new(width, height).ok_or("The image could not be made.")?;
    let scale = tree.size();
    resvg::render(
        &tree,
        Transform::from_scale(width as f32 / scale.width(), height as f32 / scale.height()),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

struct Painter {
    /// The board and the names, drawn once.
    base: Pixmap,
    pieces: HashMap<Piece, Pixmap>,
    square: u32,
    flipped: bool,
}

fn painter(options: &ImageOptions) -> Result<Painter, String> {
    let size = (options.size / 8).max(8) * 8;
    let square = size / 8;
    let look = looks::board(&options.board_theme);
    let board = crate::assets::shared(&format!("boards/{}", look.image))
        .or_else(|| crate::assets::shared("boards/brown.png"))
        .ok_or("No board image is available.")?;
    let board = image::load_from_memory(&board)
        .map_err(|_| "An image for the export could not be loaded.")?
        .resize_exact(size, size, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let board = Pixmap::from_vec(
        board.into_raw(),
        IntSize::from_wh(size, size).ok_or("The image could not be made.")?,
    )
    .ok_or("The image could not be made.")?;
    let flipped = options.orientation == Color::Black;
    let (top, bottom) = if flipped {
        (&options.white, &options.black)
    } else {
        (&options.black, &options.white)
    };
    let height = size + 2 * BAR;
    let font = BAR / 2;
    let baseline = |bar_top: u32| bar_top + BAR / 2 + font * 7 / 20;
    let names = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{size}' height='{height}'>\
         <rect width='{size}' height='{height}' fill='#262421'/>\
         <g font-family='Helvetica, Arial, sans-serif' font-weight='600' font-size='{font}' fill='#e8e6e3'>\
         <text x='8' y='{}'>{}</text><text x='8' y='{}'>{}</text></g></svg>",
        baseline(0),
        escape(top),
        baseline(size + BAR),
        escape(bottom),
    );
    let mut base = rasterize(names.as_bytes(), size, height)?;
    base.draw_pixmap(
        0,
        BAR as i32,
        board.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    let mut pieces = HashMap::new();
    for color in Color::ALL {
        for role in shakmaty::Role::ALL {
            let piece = Piece { color, role };
            let svg = crate::assets::shared(&looks::piece_path(&options.piece_set, piece))
                .ok_or("An image for the export could not be loaded.")?;
            pieces.insert(piece, rasterize(&svg, square, square)?);
        }
    }
    Ok(Painter {
        base,
        pieces,
        square,
        flipped,
    })
}

impl Painter {
    fn at(&self, square: Square) -> (u32, u32) {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        if self.flipped {
            ((7 - file) * self.square, rank * self.square + BAR)
        } else {
            (file * self.square, (7 - rank) * self.square + BAR)
        }
    }

    fn draw(&self, board: &Board, last_move: Option<&str>) -> Pixmap {
        let mut frame = self.base.clone();
        let squares = last_move
            .into_iter()
            .flat_map(|uci| [uci.get(0..2), uci.get(2..4)])
            .flatten()
            .filter_map(|name| name.parse::<Square>().ok());
        let mut highlight = resvg::tiny_skia::Paint::default();
        highlight.set_color(Paint::from_rgba8(155, 199, 0, 105));
        for square in squares {
            let (x, y) = self.at(square);
            if let Some(rect) =
                Rect::from_xywh(x as f32, y as f32, self.square as f32, self.square as f32)
            {
                frame.fill_rect(rect, &highlight, Transform::identity(), None);
            }
        }
        for (square, piece) in board.clone() {
            let (x, y) = self.at(square);
            if let Some(image) = self.pieces.get(&piece) {
                frame.draw_pixmap(
                    x as i32,
                    y as i32,
                    image.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        frame
    }
}

/// Each position of the game from the start, with the move that led to it; `up_to` stops after
/// that many moves. The line stops at the first illegal move.
fn positions(options: &ImageOptions, up_to: usize) -> Result<Vec<(Board, Option<String>)>, String> {
    let mut position = kchess_domain::rules::setup_start(&options.variant, &options.fen)
        .ok_or("That game cannot be replayed.")?;
    let mut list = vec![(position.board().clone(), None)];
    for uci in options.moves.iter().take(up_to) {
        let Some(chosen) = kchess_domain::rules::parse_uci(uci.trim())
            .and_then(|parsed| kchess_domain::rules::legal_move(&position, parsed))
        else {
            break;
        };
        position.play_unchecked(chosen);
        list.push((position.board().clone(), Some(uci.clone())));
    }
    Ok(list)
}

fn rgba(frame: Pixmap) -> Result<RgbaImage, String> {
    let (width, height) = (frame.width(), frame.height());
    // The frames are opaque, so their premultiplied pixels are the plain ones.
    RgbaImage::from_raw(width, height, frame.take())
        .ok_or_else(|| "The image could not be made.".into())
}

/// One line of rotated text: what it says, its size and weight, and how strongly it shows.
pub struct TextLine {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub opacity: f32,
}

/// Lines of text, centred and turned `rotation` degrees (a multiple of 90), as a PNG `scale` times
/// its size: GPUI cannot rotate text, so a turned clock face is shown as a picture of it.
pub fn rotated_text_png(
    lines: &[TextLine],
    color: &str,
    rotation: u16,
    scale: f32,
) -> Result<Vec<u8>, String> {
    const GAP: f32 = 12.;
    const WIDTH: f32 = 560.;
    let height: f32 = lines.iter().map(|line| line.size).sum::<f32>()
        + GAP * lines.len().saturating_sub(1) as f32;
    let sideways = rotation % 180 == 90;
    let (outer_w, outer_h) = if sideways {
        (height, WIDTH)
    } else {
        (WIDTH, height)
    };
    let mut y = 0.;
    let mut body = String::new();
    for line in lines {
        y += line.size;
        body.push_str(&format!(
            "<text x='{x}' y='{y}' font-size='{size}' font-weight='{weight}' opacity='{opacity}' text-anchor='middle'>{text}</text>",
            x = WIDTH / 2.,
            y = y - line.size * 0.18,
            size = line.size,
            weight = if line.bold { 700 } else { 500 },
            opacity = line.opacity,
            text = escape(&line.text),
        ));
        y += GAP;
    }
    // Turned about the block's centre, then moved to the middle of the (maybe swapped) canvas.
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{outer_w}' height='{outer_h}'>\
         <g font-family='-apple-system, Segoe UI, Helvetica, Arial, sans-serif' fill='{color}' \
         transform='translate({cx} {cy}) rotate({rotation}) translate({bx} {by})'>{body}</g></svg>",
        cx = outer_w / 2.,
        cy = outer_h / 2.,
        bx = -WIDTH / 2.,
        by = -height / 2.,
    );
    let pixmap = rasterize(
        svg.as_bytes(),
        (outer_w * scale).round() as u32,
        (outer_h * scale).round() as u32,
    )?;
    pixmap.encode_png().map_err(|cause| cause.to_string())
}

/// A PNG of the position after `ply` moves.
pub fn position_png(options: &ImageOptions, ply: usize) -> Result<Vec<u8>, String> {
    let paint = painter(options)?;
    let (board, last) = positions(options, ply)?
        .pop()
        .ok_or("That game cannot be replayed.")?;
    let frame = rgba(paint.draw(&board, last.as_deref()))?;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "The image could not be made.")?;
    Ok(out)
}

/// An animated GIF of the whole game; `delay` is the time per move in hundredths of a second.
/// `progress` hears each frame drawn; setting `cancel` stops it.
pub fn game_gif(
    options: &ImageOptions,
    delay: u32,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize),
) -> Result<Vec<u8>, String> {
    let paint = painter(options)?;
    let list = positions(options, options.moves.len())?;
    if list.len() > MAX_FRAMES {
        return Err("That game is too long for a GIF (300 moves at most).".into());
    }
    let mut out = Cursor::new(Vec::new());
    {
        let mut encoder = GifEncoder::new_with_speed(&mut out, 10);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|_| "The image could not be made.")?;
        let count = list.len();
        for (index, (board, last)) in list.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let hundredths = if index == 0 {
                delay * 2
            } else if index == count - 1 {
                300
            } else {
                delay
            };
            let frame = Frame::from_parts(
                rgba(paint.draw(board, last.as_deref()))?,
                0,
                0,
                Delay::from_numer_denom_ms(hundredths * 10, 1),
            );
            encoder
                .encode_frame(frame)
                .map_err(|_| "The image could not be made.")?;
            progress(index + 1, count);
        }
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{ImageOptions, TextLine, game_gif, position_png, rotated_text_png};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn a_turned_face_is_drawn_sideways_or_upside_down() {
        let lines = [TextLine {
            text: "5:00".into(),
            size: 120.,
            bold: true,
            opacity: 1.,
        }];
        let size = |rotation| {
            let png = rotated_text_png(&lines, "#000000", rotation, 1.).unwrap();
            let image = image::load_from_memory(&png).unwrap();
            (image.width(), image.height())
        };
        assert_eq!(size(180), (560, 120));
        assert_eq!(size(90), (120, 560));
    }

    fn game() -> ImageOptions {
        ImageOptions {
            variant: "standard".into(),
            fen: crate::game::STANDARD_FEN.into(),
            moves: ["e2e4", "e7e5", "g1f3"].map(String::from).to_vec(),
            orientation: shakmaty::Color::White,
            board_theme: "brown".into(),
            piece_set: "cburnett".into(),
            white: "You".into(),
            black: "Stockfish".into(),
            size: 360,
        }
    }

    #[test]
    fn a_png_of_the_position_shown() {
        let png = position_png(&game(), 2).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!((image.width(), image.height()), (360, 360 + 56));
        // The names' bar is the dark background.
        assert_eq!(
            image.to_rgba8().get_pixel(300, 4).0,
            [0x26, 0x24, 0x21, 255]
        );
    }

    #[test]
    fn a_gif_of_every_position() {
        let heard = std::cell::Cell::new(0);
        let gif = game_gif(&game(), 80, &AtomicBool::new(false), |done, total| {
            assert_eq!(total, 4);
            heard.set(done);
        })
        .unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!(heard.get(), 4);
        assert!(game_gif(&game(), 80, &AtomicBool::new(true), |_, _| {}).is_err());
    }
}
