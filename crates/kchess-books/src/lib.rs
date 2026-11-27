//! A chess book's pages, on this computer alone: a PDF read with `hayro` and each page drawn as a
//! picture, which the Book page shows and finds the diagrams in (`kchess_recognise::find_boards`).
//! Scanned books are pictures of pages, compressed as JBIG2 or CCITT; `hayro` decodes both.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// The widest a page is drawn, in pixels: four times a printed page's width in points, enough
/// for a diagram's squares to be read.
pub const MAX_WIDTH: u32 = 2_400;
/// The tallest, so a strip of a page is not drawn a mile long.
const MAX_HEIGHT: u32 = 6_000;

/// Why a book could not be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a PDF, or one too damaged to read.
    Unreadable,
    /// A PDF locked with a password.
    Locked,
    /// A PDF with no pages.
    Empty,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Unreadable => "That file is not a PDF KChess can read",
            Refusal::Locked => "That PDF is locked with a password",
            Refusal::Empty => "That PDF has no pages",
        })
    }
}

impl std::error::Error for Refusal {}

/// An opened book: its bytes and the size of each page, in points. Cheap to clone and to send
/// to another thread; each page is read from the bytes again when it is drawn.
#[derive(Clone)]
pub struct Book {
    bytes: Arc<Vec<u8>>,
    pages: Arc<Vec<(f32, f32)>>,
}

impl std::fmt::Debug for Book {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Book {{ {} pages }}", self.pages.len())
    }
}

impl Book {
    /// The book in `bytes`, a PDF.
    pub fn open(bytes: Vec<u8>) -> Result<Book, Refusal> {
        let bytes = Arc::new(bytes);
        let pages = guarded(|| {
            let pdf = Pdf::new(bytes.clone()).map_err(|error| match error {
                LoadPdfError::Decryption(_) => Refusal::Locked,
                LoadPdfError::Invalid => Refusal::Unreadable,
            })?;
            Ok(pdf
                .pages()
                .iter()
                .map(|page| page.render_dimensions())
                .collect::<Vec<_>>())
        })
        .unwrap_or(Err(Refusal::Unreadable))?;
        if pages.is_empty() {
            return Err(Refusal::Empty);
        }
        Ok(Book {
            bytes,
            pages: Arc::new(pages),
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page `index`'s width and height in points, from 0.
    pub fn page_size(&self, index: usize) -> Option<(f32, f32)> {
        self.pages.get(index).copied()
    }

    /// Page `index` drawn `width` pixels wide (at most `MAX_WIDTH`) on white, or `None` when
    /// there is no such page or it cannot be drawn.
    pub fn render(&self, index: usize, width: u32) -> Option<image::RgbaImage> {
        let (page_width, page_height) = self.page_size(index)?;
        if page_width <= 0. || page_height <= 0. {
            return None;
        }
        let scale =
            (width.clamp(1, MAX_WIDTH) as f32 / page_width).min(MAX_HEIGHT as f32 / page_height);
        let bytes = self.bytes.clone();
        guarded(move || {
            let pdf = Pdf::new(bytes).ok()?;
            let pages = pdf.pages();
            let page = pages.get(index)?;
            let pixmap = hayro::render(
                page,
                &RenderCache::new(),
                &InterpreterSettings::default(),
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                },
            );
            // Drawn over opaque white, so its premultiplied colours are its colours.
            image::RgbaImage::from_raw(
                u32::from(pixmap.width()),
                u32::from(pixmap.height()),
                pixmap.data_as_u8_slice().to_vec(),
            )
        })
        .flatten()
    }
}

/// `read`'s answer, or `None` when it panicked: a damaged file must not take the app down.
fn guarded<T>(read: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).ok()
}

#[cfg(test)]
mod tests;
