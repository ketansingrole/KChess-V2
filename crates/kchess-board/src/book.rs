//! A chess book as the Books page reads it: a PDF opened from a file (`kchess_books`), each page
//! drawn sharp enough to read its diagrams, the diagrams on it found (`picture::find_boards`)
//! and one read into its position (`picture::read_board_image`). Everything here is slow and
//! runs off the main thread.

use std::path::Path;
use std::sync::Arc;

use image::{DynamicImage, Frame, RgbaImage};

pub use kchess_books::{Book, Refusal};

use crate::picture::{Area, Recognised, find_boards, read_board_image};

/// The largest book file opened.
const MAX_BOOK_FILE: u64 = 512 * 1024 * 1024;
/// How many pixels a page is drawn for each of its points: a printed diagram's squares come out
/// some fifty pixels wide, which the reader reads well, where at half that it misreads pieces.
const PIXELS_PER_POINT: f32 = 4.;
/// A diagram is read with this much of its size around it: its coordinates, which say which way
/// up it is.
const AROUND_DIAGRAM: f32 = 0.12;

/// The book in the file at `path`, or why there is none.
pub fn open(path: &Path) -> Result<Book, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("That file could not be read: {error}"))?
        .len();
    if size > MAX_BOOK_FILE {
        return Err("That file is too large to open".into());
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("That file could not be read: {error}"))?;
    Book::open(bytes).map_err(|refusal| refusal.to_string())
}

/// A page drawn: the picture shown, its pixels for reading diagrams and where each diagram is.
#[derive(Clone)]
pub struct DrawnPage {
    pub index: usize,
    /// As GPUI draws it. Whoever replaces it releases its texture (`App::drop_image`).
    pub image: Arc<gpui_kit::RenderImage>,
    pub pixels: Arc<RgbaImage>,
    /// The diagrams, in reading order, in `pixels`' pixels.
    pub diagrams: Vec<Area>,
}

impl DrawnPage {
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    pub fn height(&self) -> u32 {
        self.pixels.height()
    }
}

/// Page `index` of `book` drawn, with its diagrams found; `None` when it cannot be drawn.
pub fn draw_page(book: &Book, index: usize) -> Option<DrawnPage> {
    let (width, _) = book.page_size(index)?;
    let page = book.render(index, (width * PIXELS_PER_POINT).round() as u32)?;
    let page = DynamicImage::ImageRgba8(page);
    let diagrams = find_boards(&page);
    let pixels = page.into_rgba8();
    let mut shown = pixels.as_raw().clone();
    // GPUI draws straight-alpha BGRA; the page is opaque RGBA.
    for pixel in shown.as_chunks_mut::<4>().0 {
        gpui_kit::swap_rgba_pa_to_bgra(pixel);
    }
    let shown = RgbaImage::from_raw(pixels.width(), pixels.height(), shown)?;
    Some(DrawnPage {
        index,
        image: Arc::new(gpui_kit::RenderImage::new(vec![Frame::new(shown)])),
        pixels: Arc::new(pixels),
        diagrams,
    })
}

/// The position of the diagram at `area` on a drawn page.
pub fn read_diagram(pixels: &RgbaImage, area: Area) -> Result<Recognised, crate::picture::Refusal> {
    let around = area.grown(AROUND_DIAGRAM, pixels.width(), pixels.height());
    let cut = image::imageops::crop_imm(pixels, around.x, around.y, around.width, around.height)
        .to_image();
    read_board_image(&DynamicImage::ImageRgba8(cut))
}
