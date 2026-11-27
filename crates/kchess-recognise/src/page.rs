//! Every diagram on a page of a book: the ink that joins each board's squares, frame and pieces
//! into one roughly square patch, then the board found in it as in a single picture (`locate`),
//! which gives its squares' edges. Text and coordinates are many small patches; a photograph or
//! a figure is not checkered.

use image::DynamicImage;
use image::imageops::FilterType;

use crate::grid::MIN_SQUARE;
use crate::picture::Picture;

/// Where a board lies on a page, in the page picture's pixels: its squares, without the frame,
/// coordinates or caption around them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    /// This area grown by `part` of its size on every side, kept within a `width` by `height`
    /// picture: the board with its coordinates, which say which way up it is.
    pub fn grown(&self, part: f32, width: u32, height: u32) -> Area {
        let dx = (self.width as f32 * part) as u32;
        let dy = (self.height as f32 * part) as u32;
        let x = self.x.saturating_sub(dx);
        let y = self.y.saturating_sub(dy);
        Area {
            x,
            y,
            width: (self.x + self.width + dx).min(width).saturating_sub(x),
            height: (self.y + self.height + dy).min(height).saturating_sub(y),
        }
    }

    fn contains(&self, other: &Area) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

/// Pages larger than this on their longer side are searched scaled down: a board on it is still
/// a hundred pixels or more.
const MAX_SIDE: u32 = 1600;
/// The least a board's side is of the page's shorter side.
const LEAST_PART: f32 = 0.06;
/// How much longer one side of a board's patch may be than the other.
const SQUARENESS: f32 = 1.25;
/// How much darker than the paper ink is.
const INK: u8 = 35;
/// What of its size the picture checked for a board takes around a patch.
const MARGIN: f32 = 0.08;

/// Every board on `page`, in reading order: row by row from the top, each row from the left.
pub fn find_boards(page: &DynamicImage) -> Vec<Area> {
    let (width, height) = (page.width(), page.height());
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let scale = (MAX_SIDE as f32 / width.max(height) as f32).min(1.);
    let scaled;
    let searched = if scale < 1. {
        scaled = page.resize(
            (width as f32 * scale) as u32,
            (height as f32 * scale) as u32,
            FilterType::Triangle,
        );
        &scaled
    } else {
        page
    };
    let grey = searched.to_luma8();
    let (w, h) = (grey.width() as usize, grey.height() as usize);
    let paper = paper(grey.as_raw());
    let ink: Vec<bool> = grey
        .as_raw()
        .iter()
        .map(|&v| v < paper.saturating_sub(INK))
        .collect();
    let least = (LEAST_PART * w.min(h) as f32).max(8. * MIN_SQUARE) as u32;
    let mut patches: Vec<Area> = patches(&ink, w, h)
        .into_iter()
        .filter(|patch| {
            let (long, short) = (
                patch.width.max(patch.height) as f32,
                patch.width.min(patch.height) as f32,
            );
            short >= least as f32 && long <= SQUARENESS * short
        })
        .collect();
    // A board drawn inside another's patch (a picture within a picture) is the outer one's.
    let all = patches.clone();
    patches.retain(|patch| {
        !all.iter()
            .any(|other| other != patch && other.contains(patch))
    });
    let back = 1. / scale;
    let mut boards: Vec<Area> = patches
        .into_iter()
        .map(|patch| Area {
            x: (patch.x as f32 * back) as u32,
            y: (patch.y as f32 * back) as u32,
            width: ((patch.width as f32 * back) as u32).min(width),
            height: ((patch.height as f32 * back) as u32).min(height),
        })
        .filter_map(|patch| {
            // The board's own edges, found as in a picture of it alone: a patch may take in a
            // coordinate or a number printed against the frame.
            let checked = patch.grown(MARGIN, width, height);
            let picture =
                Picture::of(&page.crop_imm(checked.x, checked.y, checked.width, checked.height));
            let grid = crate::locate(&picture)?.grid;
            let scale = checked.width as f32 / picture.width as f32;
            let at = |start: f32, origin: u32| origin + (start.max(0.) * scale).round() as u32;
            Some(Area {
                x: at(grid.x, checked.x),
                y: at(grid.y, checked.y),
                width: (8. * grid.width * scale).round() as u32,
                height: (8. * grid.height * scale).round() as u32,
            })
        })
        .collect();
    reading_order(&mut boards);
    boards
}

/// The paper's brightness: what nine pixels in ten are no brighter than.
fn paper(grey: &[u8]) -> u8 {
    let mut counts = [0usize; 256];
    for &v in grey {
        counts[usize::from(v)] += 1;
    }
    let wanted = grey.len() * 9 / 10;
    let mut seen = 0;
    for (value, count) in counts.iter().enumerate() {
        seen += count;
        if seen >= wanted {
            return value as u8;
        }
    }
    255
}

/// The bounds of each patch of ink, its pixels touching at sides or corners.
fn patches(ink: &[bool], width: usize, height: usize) -> Vec<Area> {
    let mut seen = vec![false; ink.len()];
    let mut stack = Vec::new();
    let mut found = Vec::new();
    for start in 0..ink.len() {
        if !ink[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        while let Some(at) = stack.pop() {
            let (x, y) = (at % width, at / width);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if ink[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        found.push(Area {
            x: left as u32,
            y: top as u32,
            width: (right - left + 1) as u32,
            height: (bottom - top + 1) as u32,
        });
    }
    found
}

/// Sorts `boards` row by row from the top, each row from the left: a board whose middle is above
/// the bottom of a row's first board is in that row.
fn reading_order(boards: &mut Vec<Area>) {
    boards.sort_by_key(|board| board.y);
    let mut rows: Vec<Vec<Area>> = Vec::new();
    for board in boards.drain(..) {
        let middle = board.y + board.height / 2;
        match rows.last_mut() {
            Some(row)
                if row
                    .first()
                    .is_some_and(|first| middle < first.y + first.height) =>
            {
                row.push(board);
            }
            _ => rows.push(vec![board]),
        }
    }
    for mut row in rows {
        row.sort_by_key(|board| board.x);
        boards.extend(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_touching_at_corners_are_one() {
        // Two squares meeting at a corner, and one apart.
        let (w, h) = (10, 6);
        let mut ink = vec![false; w * h];
        for (x, y) in [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
            (8, 5),
        ] {
            ink[y * w + x] = true;
        }
        let found = patches(&ink, w, h);
        assert_eq!(
            found,
            vec![
                Area {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 4
                },
                Area {
                    x: 8,
                    y: 5,
                    width: 1,
                    height: 1
                },
            ]
        );
    }

    #[test]
    fn boards_are_ordered_as_read() {
        let at = |x, y| Area {
            x,
            y,
            width: 100,
            height: 100,
        };
        // The second row's boards stand a little higher or lower than each other, as a scan's do.
        let mut boards = vec![
            at(300, 210),
            at(0, 5),
            at(150, 200),
            at(300, 0),
            at(0, 195),
            at(150, 0),
        ];
        reading_order(&mut boards);
        assert_eq!(
            boards,
            vec![
                at(0, 5),
                at(150, 0),
                at(300, 0),
                at(0, 195),
                at(150, 200),
                at(300, 210)
            ]
        );
    }

    #[test]
    fn grown_stays_on_the_page() {
        let area = Area {
            x: 5,
            y: 10,
            width: 100,
            height: 100,
        };
        assert_eq!(
            area.grown(0.1, 112, 400),
            Area {
                x: 0,
                y: 0,
                width: 112,
                height: 120
            }
        );
    }
}
