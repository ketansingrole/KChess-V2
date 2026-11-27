//! A chess book as the Books page reads it: a PDF opened from a file (`kchess_books`), each page
//! drawn to be shown with the diagrams on it found (`picture::find_boards`), and one read into
//! its position from the page drawn again sharper (`picture::read_board_image`). Everything here
//! is slow and runs off the main thread.

use std::path::Path;
use std::sync::Arc;

use image::{DynamicImage, Frame};

pub use kchess_books::{Book, Refusal};

use crate::picture::{Area, Recognised, find_boards, read_board_image};

/// The largest book file opened.
const MAX_BOOK_FILE: u64 = 512 * 1024 * 1024;
/// How many pixels a page is drawn for each of its points to be shown: sharp on a Retina screen
/// with the book's side half the window, and a quarter of the memory of a page drawn to be read.
const SHOWN_PER_POINT: f32 = 2.5;
/// How many a page is drawn to read a diagram on it: a printed diagram's squares come out some
/// fifty pixels wide, which the reader reads well, where at half that it misreads pieces.
const READ_PER_POINT: f32 = 4.;
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

/// A page drawn to be shown, with where each diagram is on it.
#[derive(Clone)]
pub struct DrawnPage {
    pub index: usize,
    /// As GPUI draws it. Whoever drops it releases its texture (`App::drop_image`).
    pub image: Arc<gpui_kit::RenderImage>,
    pub width: u32,
    pub height: u32,
    /// The diagrams, in reading order, in `image`'s pixels.
    pub diagrams: Vec<Area>,
}

/// Page `index` of `book` drawn to be shown, with its diagrams found; `None` when it cannot be
/// drawn.
pub fn draw_page(book: &Book, index: usize) -> Option<DrawnPage> {
    let (width, _) = book.page_size(index)?;
    let page = book.render(index, (width * SHOWN_PER_POINT).round() as u32)?;
    let page = DynamicImage::ImageRgba8(page);
    let diagrams = find_boards(&page);
    let (width, height) = (page.width(), page.height());
    Some(DrawnPage {
        index,
        image: shown(page.into_rgba8()),
        width,
        height,
        diagrams,
    })
}

/// A page drawn, as GPUI draws it: straight-alpha BGRA, where the page is opaque RGBA.
fn shown(mut page: image::RgbaImage) -> Arc<gpui_kit::RenderImage> {
    for pixel in page.as_chunks_mut::<4>().0 {
        gpui_kit::swap_rgba_pa_to_bgra(pixel);
    }
    Arc::new(gpui_kit::RenderImage::new(vec![Frame::new(page)]))
}

/// What the file of a book opened before holds now, for the list of books.
pub enum Cover {
    /// Its first page, `width` pixels wide. Whoever drops it releases its texture.
    Drawn(Arc<gpui_kit::RenderImage>),
    /// There is no file there any more.
    Missing,
    /// The file is there but is no book KChess can read.
    Unreadable,
}

/// The cover of the book in the file at `path`, drawn `width` pixels wide.
pub fn cover(path: &Path, width: u32) -> Cover {
    if std::fs::metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
        return Cover::Missing;
    }
    open(path)
        .ok()
        .and_then(|book| book.render(0, width))
        .map_or(Cover::Unreadable, |page| Cover::Drawn(shown(page)))
}

/// The position of the diagram at `area` of the drawn `page` (in its pixels), read from the page
/// drawn again sharp enough to read it.
pub fn read_diagram(
    book: &Book,
    page: &DrawnPage,
    area: Area,
) -> Result<Recognised, crate::picture::Refusal> {
    let unread = crate::picture::Refusal::NoBoard;
    let (width, _) = book.page_size(page.index).ok_or(unread)?;
    let pixels = book
        .render(page.index, (width * READ_PER_POINT).round() as u32)
        .ok_or(unread)?;
    let scale = pixels.width() as f32 / page.width.max(1) as f32;
    let at = |value: u32| (value as f32 * scale).round() as u32;
    let area = Area {
        x: at(area.x),
        y: at(area.y),
        width: at(area.width),
        height: at(area.height),
    };
    let around = area.grown(AROUND_DIAGRAM, pixels.width(), pixels.height());
    let cut = image::imageops::crop_imm(&pixels, around.x, around.y, around.width, around.height)
        .to_image();
    read_board_image(&DynamicImage::ImageRgba8(cut))
}
