//! What stands on one square: the square's own colour from a band just inside its edges, then
//! whatever differs from it, filled in where an outline encloses it, as the piece's silhouette;
//! then that silhouette and the brightness inside it, scaled to a small grid, to compare with the
//! bundled pieces drawn the same way.

use crate::grid::Grid;
use crate::picture::{Picture, distance, luma, median, median_colour};

/// The grid a silhouette is compared on, `FEATURE` × `FEATURE`.
pub(crate) const FEATURE: usize = 24;
/// The least part of a square a piece covers; less is a mark (a label, a speck), not a piece.
pub(crate) const PIECE_AREA: f32 = 0.03;
/// A mark under this part of a square is ignored; between this and `PIECE_AREA` the square is
/// doubtful.
const MARK_AREA: f32 = 0.012;
/// How far a pixel's colour must be from its square's to be part of something on it.
const DIFFERENT: f32 = 24.;

/// A square of the picture, resampled to `size` × `size` pixels.
pub(crate) struct Cell {
    pub rgb: Vec<[f32; 3]>,
    /// The square's own colour, from the band inside its edges.
    pub background: [f32; 3],
    /// How far that band's colours typically stray from it (the 60th percentile, since a piece
    /// may reach into the band): a textured board's grain.
    pub spread: f32,
    /// The width over which the picture was averaged, in the cell's pixels (`texture`).
    pub blur: f32,
    /// On a textured picture, each pixel's brightness before averaging; else empty.
    pub crisp: Vec<f32>,
    /// On a textured picture, the brightness below which a pixel of `crisp` is ink.
    pub paper: f32,
    /// On a textured picture, the least brightness its squares' texture shows once averaged,
    /// however unevenly its lines are drawn; else 0.
    pub floor: f32,
    /// Colours laid over the board, see-through, as an arrow drawn across squares is.
    pub overlays: Vec<Overlay>,
}

/// A colour laid over the board: what shows is `alpha` of `colour` over what is beneath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Overlay {
    pub alpha: f32,
    pub colour: [f32; 3],
}

/// How far in from a cell's edges its neighbours' colours reach: a pixel when the board is
/// placed a pixel off, and half the width a textured picture was averaged over.
fn inset(size: usize, blur: f32) -> usize {
    ((size as f32 * 0.03).round() as usize).max(1) + (blur / 2.) as usize
}

/// Whether a pixel of a `size`-pixel cell is in the band its colour is taken from, `inset` to
/// `inset + width` in from its edges, and how far in it is.
fn band(size: usize, index: usize, blur: f32) -> (bool, usize) {
    let (x, y) = (index % size, index / size);
    let from_edge = x.min(y).min(size - 1 - x).min(size - 1 - y);
    let inset = inset(size, blur);
    let width = ((size as f32 * 0.06).round() as usize).max(2);
    (from_edge >= inset && from_edge < inset + width, from_edge)
}

/// The 64 squares `grid` places in `picture`, from the top left, row by row.
pub(crate) fn cells(picture: &Picture, grid: &Grid, size: usize, blur: f32) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(64);
    // Several samples a pixel when the picture's squares are larger than `size`.
    let samples = ((grid.width.max(grid.height) / size as f32).ceil() as usize).clamp(1, 4);
    let pixel = |x: f32, y: f32| {
        let x = (x.floor().max(0.) as usize).min(picture.width - 1);
        let y = (y.floor().max(0.) as usize).min(picture.height - 1);
        picture.rgb[y * picture.width + x]
    };
    for row in 0..8 {
        for column in 0..8 {
            // Inside the lines drawn between the squares.
            let left = grid.x + column as f32 * grid.width + grid.gap / 2.;
            let top = grid.y + row as f32 * grid.height + grid.gap / 2.;
            let (step_x, step_y) = (
                (grid.width - grid.gap) / size as f32,
                (grid.height - grid.gap) / size as f32,
            );
            let mut rgb = Vec::with_capacity(size * size);
            for j in 0..size {
                for i in 0..size {
                    let mut sum = [0f32; 3];
                    for sy in 0..samples {
                        for sx in 0..samples {
                            let x = left + (i as f32 + (sx as f32 + 0.5) / samples as f32) * step_x;
                            let y = top + (j as f32 + (sy as f32 + 0.5) / samples as f32) * step_y;
                            let colour = pixel(x, y);
                            for c in 0..3 {
                                sum[c] += colour[c];
                            }
                        }
                    }
                    let n = (samples * samples) as f32;
                    rgb.push([sum[0] / n, sum[1] / n, sum[2] / n]);
                }
            }
            cells.push(Cell::new(size, rgb, blur));
        }
    }
    cells
}

impl Cell {
    pub fn new(size: usize, rgb: Vec<[f32; 3]>, blur: f32) -> Cell {
        let ring = || {
            rgb.iter()
                .enumerate()
                .filter(|(index, _)| band(size, *index, blur).0)
                .map(|(_, &colour)| colour)
        };
        let background = median_colour(ring()).unwrap_or([255.; 3]);
        let mut strays: Vec<f32> = ring().map(|colour| distance(colour, background)).collect();
        strays.sort_by(f32::total_cmp);
        let spread = strays
            .get(strays.len() * 6 / 10)
            .copied()
            .unwrap_or_default();
        Cell {
            rgb,
            background,
            spread,
            blur,
            crisp: Vec::new(),
            paper: 0.,
            floor: 0.,
            overlays: Vec::new(),
        }
    }
}

/// The board's light and dark colours when `cells` alternate between two as a board's squares do
/// (a few, such as a highlighted last move, may stray), else `None`.
pub(crate) fn checkered(cells: &[Cell]) -> Option<[[f32; 3]; 2]> {
    let class = |parity: usize| {
        median_colour(
            cells
                .iter()
                .enumerate()
                .filter(move |(index, _)| (index / 8 + index % 8) % 2 == parity)
                .map(|(_, cell)| cell.background),
        )
    };
    let (even, odd) = (class(0)?, class(1)?);
    let contrast = distance(even, odd);
    if contrast < 16. {
        return None;
    }
    let mut stray = Vec::new();
    let mut agree = 0;
    for (index, cell) in cells.iter().enumerate() {
        let (own, other) = if (index / 8 + index % 8) % 2 == 0 {
            (even, odd)
        } else {
            (odd, even)
        };
        let off = distance(cell.background, own);
        stray.push(off);
        if off < distance(cell.background, other) {
            agree += 1;
        }
    }
    // Most squares sit close to their colour, not merely nearer it than the other.
    let typical = median(&mut stray)?;
    (agree >= 58 && typical < 0.3 * contrast).then(|| {
        if luma(even) >= luma(odd) {
            [even, odd]
        } else {
            [odd, even]
        }
    })
}

/// On a textured picture every square takes its colour from the board: the colour its
/// parity's squares typically show, with the grain they show. Averaged over the texture, a
/// large piece reaches into the band a square's own colour is taken from, and next to the
/// board's frame so does the frame; a printed diagram highlights no square. Returns `colours`
/// (the board's, light and dark, as `checkered` found them nearer the squares' edges) as the
/// squares now have them.
pub(crate) fn even_out(cells: &mut [Cell], colours: [[f32; 3]; 2]) -> [[f32; 3]; 2] {
    let mut evened = colours;
    for parity in 0..2 {
        let of_parity = |index: &usize| (index / 8 + index % 8) % 2 == parity;
        let Some(typical) = median_colour(
            (0..cells.len())
                .filter(of_parity)
                .map(|index| cells[index].background),
        ) else {
            continue;
        };
        let colour = typical;
        let nearest = usize::from(distance(typical, colours[1]) < distance(typical, colours[0]));
        evened[nearest] = typical;
        // The grain over every square's band at once: what averaging leaves of a texture
        // varies across a square slower than one band shows.
        let mut strays: Vec<f32> = (0..cells.len())
            .filter(of_parity)
            .flat_map(|index| {
                let cell = &cells[index];
                let size = cell.rgb.len().isqrt();
                cell.rgb
                    .iter()
                    .enumerate()
                    .filter(move |(at, _)| band(size, *at, cell.blur).0)
                    .map(move |(_, &c)| distance(c, colour))
            })
            .collect();
        strays.sort_by(f32::total_cmp);
        let grain = strays
            .get(strays.len() * 6 / 10)
            .copied()
            .unwrap_or_default();
        // How dark the texture gets: a little of the bands is a large piece reaching in.
        let mut shades: Vec<f32> = (0..cells.len())
            .filter(of_parity)
            .flat_map(|index| {
                let cell = &cells[index];
                let size = cell.rgb.len().isqrt();
                cell.rgb
                    .iter()
                    .enumerate()
                    .filter(move |(at, _)| band(size, *at, cell.blur).0)
                    .map(|(_, &c)| luma(c))
            })
            .collect();
        shades.sort_by(f32::total_cmp);
        let floor = shades.get(shades.len() / 20).copied().unwrap_or_default();
        for index in (0..cells.len()).filter(of_parity) {
            cells[index].background = colour;
            cells[index].spread = grain;
            cells[index].floor = floor;
        }
    }
    evened
}

/// What stands on a square: which of its pixels it covers.
pub(crate) struct Silhouette {
    pub size: usize,
    pub inside: Vec<bool>,
    /// The part of the square it covers.
    pub area: f32,
    /// Its bounds, `[left, top, right, bottom)`.
    pub bounds: [usize; 4],
}

/// A region of a cell, `[left, top, right, bottom)`.
pub(crate) type Bounds = [usize; 4];

/// A coordinate label on a square: where it is and its colour.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Label {
    pub bounds: Bounds,
    pub colour: [f32; 3],
}

/// How far `colour` is from the colours between `from` and `to`, as a label's antialiased edge
/// blends the label into its square.
fn off_the_way(colour: [f32; 3], from: [f32; 3], to: [f32; 3]) -> f32 {
    let way = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = way.iter().map(|v| v * v).sum::<f32>();
    let along = if length > 0. {
        ((0..3).map(|c| (colour[c] - from[c]) * way[c]).sum::<f32>() / length).clamp(0., 1.)
    } else {
        0.
    };
    distance(
        colour,
        [
            from[0] + along * way[0],
            from[1] + along * way[1],
            from[2] + along * way[2],
        ],
    )
}

/// How far `colour` is from `background` under `overlay`, at any share of it from none (an
/// arrow's edges blend into the square) to the overlay laid twice (where two arrows of its
/// colour cross).
fn beneath(colour: [f32; 3], background: [f32; 3], overlay: &Overlay) -> f32 {
    let to = [0, 1, 2].map(|c| overlay.alpha * (overlay.colour[c] - background[c]));
    let along = [0, 1, 2].map(|c| colour[c] - background[c]);
    let length = to.iter().map(|v| v * v).sum::<f32>();
    if length < 1. {
        return f32::MAX;
    }
    let share = (along.iter().zip(&to).map(|(a, t)| a * t).sum::<f32>() / length)
        .clamp(0., 2. - overlay.alpha);
    along
        .iter()
        .zip(&to)
        .map(|(a, t)| (a - share * t).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// How near a pixel must be to a square's colour under an overlay to be the square.
const OVERLAID: f32 = 15.;

/// The see-through colours laid over the board (an arrow drawn across it, a marked square's
/// tint), up to `MOST_OVERLAYS`. A piece stays inside its square; what crosses the line between
/// a light square and a dark one, showing each through it in the same share, is laid over them.
/// Over a pixel pair either side of the line, `pixel = square + alpha * (colour - square)`, so
/// their difference is the squares' own times `1 - alpha`. The pairs that bands across the lines
/// agree on, `AGREEING` of a square's side in all, are an overlay.
pub(crate) fn overlays(cells: &[Cell], size: usize) -> Vec<Overlay> {
    if cells.len() != 64 {
        return Vec::new();
    }
    let inset = inset(size, 0.);
    let mut found: Vec<Overlay> = Vec::new();
    for index in 0..64 {
        let (row, column) = (index / 8, index % 8);
        let right = (column < 7).then(|| (index + 1, true));
        let below = (row < 7).then(|| (index + 8, false));
        for (next, across) in [right, below].into_iter().flatten() {
            let (a, b) = (&cells[index], &cells[next]);
            let between = [0, 1, 2].map(|c| a.background[c] - b.background[c]);
            let apart = between.iter().map(|v| v * v).sum::<f32>();
            if apart < 400. {
                continue;
            }
            // Each pixel pair along the line, in order, and what overlay it shows if any.
            let mut along = Vec::new();
            for t in inset..size - inset {
                let (ia, ib) = if across {
                    (t * size + size - 1 - inset, t * size + inset)
                } else {
                    ((size - 1 - inset) * size + t, inset * size + t)
                };
                let (pa, pb) = (a.rgb[ia], b.rgb[ib]);
                if distance(pa, a.background) <= DIFFERENT
                    || distance(pb, b.background) <= DIFFERENT
                {
                    along.push(None);
                    continue;
                }
                let differ = [0, 1, 2].map(|c| pa[c] - pb[c]);
                let through = differ.iter().zip(&between).map(|(d, e)| d * e).sum::<f32>() / apart;
                let alpha = 1. - through;
                // What is left of their difference once the squares' share is taken out.
                let left = differ
                    .iter()
                    .zip(&between)
                    .map(|(d, e)| (d - through * e).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if !(0.15..0.95).contains(&alpha) || left > OVERLAID {
                    along.push(None);
                    continue;
                }
                let colour = [0, 1, 2].map(|c| {
                    ((pa[c] - through * a.background[c]) + (pb[c] - through * b.background[c]))
                        / 2.
                        / alpha
                });
                along.push(Some(Overlay { alpha, colour }));
            }
            // An overlay crosses the line as a band, the same all the way across it; a piece's
            // edge reaching over the line crosses it at a pixel or two, blended differently at
            // each.
            let even = |a: &Option<Overlay>, b: &Option<Overlay>| {
                a.zip(*b).is_some_and(|(a, b)| {
                    (a.alpha - b.alpha).abs() < 0.05 && distance(a.colour, b.colour) < 20.
                })
            };
            for run in along.chunk_by(even) {
                if run.len() >= BAND {
                    found.extend(run.iter().flatten());
                }
            }
        }
    }
    let near = |a: &Overlay, b: &Overlay| {
        (a.alpha - b.alpha).abs() < 0.1 && distance(a.colour, b.colour) < 40.
    };
    let mut kept = Vec::new();
    while kept.len() < MOST_OVERLAYS {
        let Some(best) = found
            .iter()
            .max_by_key(|a| found.iter().filter(|b| near(a, b)).count())
            .copied()
        else {
            break;
        };
        let agree: Vec<Overlay> = found.iter().filter(|b| near(&best, b)).copied().collect();
        if (agree.len() as f32) < AGREEING * size as f32 {
            break;
        }
        let mut alphas: Vec<f32> = agree.iter().map(|overlay| overlay.alpha).collect();
        alphas.sort_by(f32::total_cmp);
        let colour =
            median_colour(agree.iter().map(|overlay| overlay.colour)).unwrap_or(best.colour);
        kept.push(Overlay {
            alpha: alphas[alphas.len() / 2],
            colour,
        });
        found.retain(|b| !near(&best, b));
    }
    kept
}

/// The fewest pixels across that an overlay crossing a line between squares shows.
const BAND: usize = 3;

/// How many pixel pairs, as a share of a square's side, must agree on an overlay: an arrow
/// two squares long and a sixth of a square wide crosses lines for about a third of one.
const AGREEING: f32 = 0.15;

/// The most overlays a board is read with: a website's arrows come in a few colours.
const MOST_OVERLAYS: usize = 4;

/// Which of a cell's pixels differ from its colour, leaving out the outermost (a neighbour's
/// colour bleeds into them when the board is placed a pixel off), a coordinate label's in
/// `labels`, the square's colour under an overlay, and lone specks (a compressed picture's
/// noise).
fn differing(size: usize, cell: &Cell, labels: &[Label]) -> Vec<bool> {
    let Cell {
        background, blur, ..
    } = *cell;
    let threshold = DIFFERENT.max(cell.spread * 3. + 10.);
    let inset = inset(size, blur);
    let enclosed = enclosed(size, cell, inset);
    let differs: Vec<bool> = cell
        .rgb
        .iter()
        .enumerate()
        .map(|(index, &colour)| {
            let (x, y) = (index % size, index / size);
            let label = labels.iter().any(|label| {
                let [left, top, right, bottom] = label.bounds;
                (left..right).contains(&x)
                    && (top..bottom).contains(&y)
                    && off_the_way(colour, background, label.colour) < 28.
            });
            // On a textured picture, paper and ink: what is drawn on a square is ink, and
            // lighter than the square only where an outline encloses it.
            let apart = if blur > 0. {
                luma(background) - luma(colour)
            } else {
                distance(colour, background)
            };
            // Darker than the texture ever is, however much it strays.
            let darker = luma(colour) < cell.floor - DIFFERENT;
            // Under one overlay, or under one laid over another where two arrows cross.
            let overlaid = cell.overlays.iter().any(|overlay| {
                beneath(colour, background, overlay) < OVERLAID
                    || cell.overlays.iter().any(|under| {
                        let laid = [0, 1, 2].map(|c| {
                            background[c] + under.alpha * (under.colour[c] - background[c])
                        });
                        under != overlay && beneath(colour, laid, overlay) < OVERLAID
                    })
            });
            !label
                && !overlaid
                && band(size, index, blur).1 >= inset
                && (apart > threshold || darker || enclosed[index])
        })
        .collect();
    (0..size * size)
        .map(|index| {
            let (x, y) = (index % size, index / size);
            differs[index]
                && [
                    (x > 0).then(|| index - 1),
                    (x + 1 < size).then(|| index + 1),
                    (y > 0).then(|| index - size),
                    (y + 1 < size).then(|| index + size),
                ]
                .into_iter()
                .flatten()
                .any(|neighbour| differs[neighbour])
        })
        .collect()
}

/// On a textured square, the paper an outline encloses and that outline: a white piece's
/// inside and the ink just around it. The texture's lines may touch the outline, but the paper
/// between them reaches the square's edges, and an outline is darker than the texture around
/// it, where the paper of a gap in the texture is enclosed by the texture's own lines. Nothing
/// on a square of a picture that is not textured.
fn enclosed(size: usize, cell: &Cell, inset: usize) -> Vec<bool> {
    let mut found = vec![false; size * size];
    if cell.crisp.len() != size * size {
        return found;
    }
    let paper: Vec<bool> = cell.crisp.iter().map(|&v| v >= cell.paper).collect();
    // The paper reached from the edges, where a neighbouring square or the board's frame may
    // reach, past the corners of the texture's diagonal lines.
    let mut open: Vec<bool> = (0..size * size)
        .map(|index| from_edge(size, index) < inset)
        .collect();
    let mut stack: Vec<usize> = (0..size * size).filter(|&index| open[index]).collect();
    while let Some(index) = stack.pop() {
        for next in around(size, index, 1) {
            if paper[next] && !open[next] {
                open[next] = true;
                stack.push(next);
            }
        }
    }
    let shut: Vec<bool> = (0..size * size).map(|i| paper[i] && !open[i]).collect();
    let least = (size * size).div_ceil(200);
    // An outline's width.
    let reach = ((size as f32 * 0.04).round() as usize).max(1);
    let mut seen = vec![usize::MAX; size * size];
    for (part, (pixels, _)) in parts(size, &shut).into_iter().enumerate() {
        if pixels.len() < least {
            continue;
        }
        for &index in &pixels {
            seen[index] = part;
        }
        // The pixels within an outline's width of it.
        let mut ring = Vec::new();
        for &index in &pixels {
            for near in around(size, index, reach) {
                if seen[near] != part {
                    seen[near] = part;
                    ring.push(near);
                }
            }
        }
        let shade = ring.iter().map(|&i| cell.crisp[i]).sum::<f32>() / ring.len().max(1) as f32;
        if shade > luma(cell.background) - DIFFERENT {
            continue;
        }
        for index in pixels.into_iter().chain(ring) {
            if !paper[index] || shut[index] {
                found[index] = true;
            }
        }
    }
    found
}

/// How far a pixel of a `size`-pixel cell is from its nearest edge.
fn from_edge(size: usize, index: usize) -> usize {
    let (x, y) = (index % size, index / size);
    x.min(y).min(size - 1 - x).min(size - 1 - y)
}

/// The pixels of a `size`-pixel cell within `reach` of `index` across and down, itself
/// included.
fn around(size: usize, index: usize, reach: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (index % size, index / size);
    (y.saturating_sub(reach)..(y + reach + 1).min(size)).flat_map(move |ny| {
        (x.saturating_sub(reach)..(x + reach + 1).min(size)).map(move |nx| ny * size + nx)
    })
}

/// The connected parts of `on` (four neighbours), each its pixels and bounds.
fn parts(size: usize, on: &[bool]) -> Vec<(Vec<usize>, Bounds)> {
    let mut seen = vec![false; size * size];
    let mut parts = Vec::new();
    for start in 0..size * size {
        if !on[start] || seen[start] {
            continue;
        }
        let mut pixels = vec![start];
        seen[start] = true;
        let mut next = 0;
        let mut bounds = [size, size, 0, 0];
        while let Some(&index) = pixels.get(next) {
            next += 1;
            let (x, y) = (index % size, index / size);
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x + 1),
                bounds[3].max(y + 1),
            ];
            let neighbours = [
                (x > 0).then(|| index - 1),
                (x + 1 < size).then(|| index + 1),
                (y > 0).then(|| index - size),
                (y + 1 < size).then(|| index + size),
            ];
            for neighbour in neighbours.into_iter().flatten() {
                if on[neighbour] && !seen[neighbour] {
                    seen[neighbour] = true;
                    pixels.push(neighbour);
                }
            }
        }
        parts.push((pixels, bounds));
    }
    parts
}

/// The corner of a cell `bounds` lies in (0 top left, 1 top right, 2 bottom left, 3 bottom
/// right), if it lies within one.
fn corner_of(size: usize, [left, top, right, bottom]: Bounds) -> Option<usize> {
    let corner = size as f32 * 0.36;
    let side = |low: usize, high: usize| {
        if high as f32 <= corner {
            Some(0)
        } else if low as f32 >= size as f32 - corner {
            Some(1)
        } else {
            None
        }
    };
    Some(side(top, bottom)? * 2 + side(left, right)?)
}

/// Coordinate labels: marks in the same corner of at least three squares along one edge of the
/// board. For each cell, the labels to leave out, so a piece standing against one is read
/// without it.
pub(crate) fn labels(cells: &[Cell], size: usize) -> Vec<Vec<Label>> {
    // Each cell's marks standing alone in a corner: the corner, the mark's bounds and its colour.
    let marks: Vec<Vec<(usize, Bounds, [f32; 3])>> = cells
        .iter()
        .map(|cell| {
            let on = differing(size, cell, &[]);
            parts(size, &on)
                .into_iter()
                .filter(|(pixels, _)| pixels.len() * 300 >= size * size)
                .filter_map(|(pixels, bounds)| {
                    let corner = corner_of(size, bounds)?;
                    let colour = median_colour(pixels.iter().map(|&index| cell.rgb[index]))?;
                    Some((corner, bounds, colour))
                })
                .collect()
        })
        .collect();
    let edges: [Vec<usize>; 4] = [
        (0..8).collect(),
        (56..64).collect(),
        (0..8).map(|row| row * 8).collect(),
        (0..8).map(|row| row * 8 + 7).collect(),
    ];
    let groups = || {
        edges
            .iter()
            .flat_map(|edge| (0..4).map(move |corner| (edge, corner)))
    };
    let alone = |edge: &Vec<usize>, corner: usize| -> Vec<(Bounds, [f32; 3])> {
        edge.iter()
            .flat_map(|&cell| marks.get(cell).into_iter().flatten())
            .filter(|(at, ..)| *at == corner)
            .map(|(_, bounds, colour)| (*bounds, *colour))
            .collect()
    };
    // The labels' colours, from the edges where at least three stand alone.
    let known: Vec<[f32; 3]> = groups()
        .map(|(edge, corner)| alone(edge, corner))
        .filter(|found| found.len() >= 3)
        .flat_map(|found| found.into_iter().map(|(_, colour)| colour))
        .collect();
    let mut erase = vec![Vec::new(); cells.len()];
    if known.is_empty() {
        return erase;
    }
    for (edge, corner) in groups() {
        let mut found = alone(edge, corner);
        if found.len() < 3 {
            // Along an edge full of pieces a label touches the piece beside it: there, the
            // pixels of a label's colour in that corner.
            found = edge
                .iter()
                .filter_map(|&cell| labelled_pixels(cells.get(cell)?, size, corner, &known))
                .collect();
        }
        if found.len() < 3 {
            continue;
        }
        let [left, top, right, bottom] = found.iter().fold([size, size, 0, 0], |all, (b, _)| {
            [
                all[0].min(b[0]),
                all[1].min(b[1]),
                all[2].max(b[2]),
                all[3].max(b[3]),
            ]
        });
        let bounds = [
            left.saturating_sub(1),
            top.saturating_sub(1),
            (right + 1).min(size),
            (bottom + 1).min(size),
        ];
        // A label's colour differs on light and dark squares: each square takes the colour
        // furthest from its own, the one drawn to show on it.
        for &cell in edge {
            let Some(own) = cells.get(cell) else {
                continue;
            };
            let apart = |colour: &[f32; 3]| distance(*colour, own.background);
            let colour = known
                .iter()
                .copied()
                .max_by(|a, b| apart(a).total_cmp(&apart(b)));
            if let (Some(colour), Some(cell)) = (colour, erase.get_mut(cell)) {
                cell.push(Label { bounds, colour });
            }
        }
    }
    erase
}

/// The pixels in `corner` of `cell` of one of the `known` label colours, as one mark: its bounds
/// and colour, when there are enough of them.
fn labelled_pixels(
    cell: &Cell,
    size: usize,
    corner: usize,
    known: &[[f32; 3]],
) -> Option<(Bounds, [f32; 3])> {
    let zone = (size as f32 * 0.36) as usize;
    let (xs, ys) = match corner {
        0 => (0..zone, 0..zone),
        1 => (size - zone..size, 0..zone),
        2 => (0..zone, size - zone..size),
        _ => (size - zone..size, size - zone..size),
    };
    let on = differing(size, cell, &[]);
    let mut bounds = [size, size, 0, 0];
    let mut colours = Vec::new();
    for y in ys {
        for x in xs.clone() {
            let index = y * size + x;
            let colour = cell.rgb[index];
            let label = known.iter().any(|&label| distance(colour, label) < 20.);
            if on[index] && label {
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x + 1),
                    bounds[3].max(y + 1),
                ];
                colours.push(colour);
            }
        }
    }
    (colours.len() * 300 >= size * size)
        .then(|| median_colour(colours.into_iter()).map(|colour| (bounds, colour)))
        .flatten()
}

/// The bounds of `inside`, less the outermost rows and columns that hold under 1.5% of it each
/// side: a few stray pixels (a compressed picture's ringing) move a piece's bounds, and so how
/// its shape is scaled, more than anything else.
fn trimmed_bounds(size: usize, inside: &[bool]) -> Bounds {
    let mut rows = vec![0usize; size];
    let mut columns = vec![0usize; size];
    for (index, _) in inside.iter().enumerate().filter(|(_, on)| **on) {
        rows[index / size] += 1;
        columns[index % size] += 1;
    }
    let allowed = inside.iter().filter(|on| **on).count() * 15 / 1000;
    let span = |counts: &[usize]| {
        let (mut low, mut high) = (0, counts.len());
        while low < high && counts[low] == 0 {
            low += 1;
        }
        while high > low && counts[high - 1] == 0 {
            high -= 1;
        }
        let mut removed = 0;
        while low + 1 < high && removed + counts[low] <= allowed {
            removed += counts[low];
            low += 1;
        }
        let mut removed = 0;
        while high > low + 1 && removed + counts[high - 1] <= allowed {
            removed += counts[high - 1];
            high -= 1;
        }
        (low, high)
    };
    let ((left, right), (top, bottom)) = (span(&columns), span(&rows));
    [left, top, right, bottom]
}

/// What stands on a cell against `background`, leaving out `labels`: `Ok` with its silhouette,
/// or `Err` with the part of the square the largest mark on it covers (none, a label, a speck).
pub(crate) fn silhouette(size: usize, cell: &Cell, labels: &[Label]) -> Result<Silhouette, f32> {
    let blur = cell.blur;
    let different = differing(size, cell, labels);
    // What the square's colour reaches from its edges is outside; the rest is something on it,
    // its outline's inside filled in.
    let mut outside = vec![false; size * size];
    let mut stack: Vec<usize> = (0..size * size)
        .filter(|&index| band(size, index, blur).1 == 0 && !different[index])
        .collect();
    for &index in &stack {
        outside[index] = true;
    }
    while let Some(index) = stack.pop() {
        let (x, y) = (index % size, index / size);
        let mut visit = |next: usize| {
            if !outside[next] && !different[next] {
                outside[next] = true;
                stack.push(next);
            }
        };
        if x > 0 {
            visit(index - 1);
        }
        if x + 1 < size {
            visit(index + 1);
        }
        if y > 0 {
            visit(index - size);
        }
        if y + 1 < size {
            visit(index + size);
        }
    }
    // Its parts: a piece, perhaps in pieces, and marks such as a coordinate in a corner.
    let inner: Vec<bool> = outside.iter().map(|out| !out).collect();
    let mut parts = parts(size, &inner);
    // On a textured picture, what averaging leaves of the texture where its lines are unevenly
    // spaced (a one-bit picture's lines a pixel thicker here and there) is long and thinner
    // than the width it was averaged over; anything drawn on a square is wider somewhere.
    if blur > 0. {
        let reach = (blur / 2.).ceil() as usize;
        let mut owner = vec![usize::MAX; size * size];
        for (part, (pixels, _)) in parts.iter().enumerate() {
            for &index in pixels {
                owner[index] = part;
            }
        }
        let mut part = 0;
        parts.retain(|(pixels, _)| {
            let wide = pixels
                .iter()
                .any(|&index| around(size, index, reach).all(|near| owner[near] == part));
            part += 1;
            wide
        });
    }
    let whole = (size * size) as f32;
    // A mark: small, in a corner, or a thin strip along an edge (the next square's colour, where
    // the board was placed a little off).
    let strip = |[left, top, right, bottom]: Bounds| {
        let (thin, near) = (size as f32 * 0.1, size as f32 * 0.15);
        let flat = ((bottom - top) as f32) < thin
            && ((top as f32) < near || (bottom as f32) > size as f32 - near);
        let tall = ((right - left) as f32) < thin
            && ((left as f32) < near || (right as f32) > size as f32 - near);
        flat || tall
    };
    let mark = |pixels: &Vec<usize>, bounds: Bounds| {
        (pixels.len() as f32) < MARK_AREA * whole
            || corner_of(size, bounds).is_some()
            || strip(bounds)
    };
    let largest = parts
        .iter()
        .map(|(pixels, _)| pixels.len())
        .max()
        .unwrap_or(0);
    let kept: Vec<&(Vec<usize>, Bounds)> = parts
        .iter()
        .filter(|(pixels, bounds)| !mark(pixels, *bounds) && pixels.len() * 5 >= largest)
        .collect();
    let area = kept.iter().map(|(pixels, _)| pixels.len()).sum::<usize>() as f32 / whole;
    if area < PIECE_AREA {
        return Err(largest as f32 / whole);
    }
    let mut inside = vec![false; size * size];
    for (pixels, _) in kept {
        for &index in pixels {
            inside[index] = true;
        }
    }
    let bounds = trimmed_bounds(size, &inside);
    Ok(Silhouette {
        size,
        inside,
        area,
        bounds,
    })
}

/// How much of what stands on a square is ink rather than paper, from 0 to 1: of the pixels deep
/// inside either (an outline's width from the other and from the square), the part that is
/// ink. A black piece is mostly solid dark, however its details are drawn; a white piece is an
/// outline around a light inside, however thick the outline. Ink is what is darker than halfway
/// from the piece's darkest to the square's own colour; on a textured picture it is read before
/// averaging, where the texture's lines are too thin to be deep in anything.
pub(crate) fn inkiness(silhouette: &Silhouette, cell: &Cell) -> f32 {
    let size = silhouette.size;
    let value = |index: usize| {
        if cell.crisp.is_empty() {
            luma(cell.rgb[index])
        } else {
            cell.crisp[index]
        }
    };
    let mut inside: Vec<f32> = (0..size * size)
        .filter(|&index| silhouette.inside[index])
        .map(value)
        .collect();
    inside.sort_by(f32::total_cmp);
    let Some(&darkest) = inside.get(inside.len() / 20) else {
        return 0.5;
    };
    let square = luma(cell.background);
    let threshold = (darkest + square.max(darkest + 2. * DIFFERENT)) / 2.;
    // 0 outside, 1 ink, 2 light.
    let kind: Vec<u8> = (0..size * size)
        .map(|index| match silhouette.inside[index] {
            false => 0,
            true if value(index) < threshold => 1,
            true => 2,
        })
        .collect();
    let reach = ((size as f32 * 0.04).round() as usize).max(1);
    let (mut ink, mut light) = (0usize, 0usize);
    for index in 0..size * size {
        let own = kind[index];
        if own != 0 && around(size, index, reach).all(|near| kind[near] == own) {
            if own == 1 {
                ink += 1;
            } else {
                light += 1;
            }
        }
    }
    if ink + light == 0 {
        return 0.5;
    }
    ink as f32 / (ink + light) as f32
}

/// A silhouette as compared: its shape scaled to fit `FEATURE` × `FEATURE`, centred, the
/// brightness inside it on the same grid, and its height and width as parts of the square.
pub(crate) struct Features {
    pub shape: Vec<f32>,
    pub brightness: Vec<f32>,
    pub height: f32,
    pub width: f32,
}

/// `silhouette`'s features, its brightness from `rgb`.
pub(crate) fn features(silhouette: &Silhouette, rgb: &[[f32; 3]]) -> Features {
    const SAMPLES: usize = 3;
    let size = silhouette.size;
    let [left, top, right, bottom] = silhouette.bounds;
    let (wide, high) = ((right - left) as f32, (bottom - top) as f32);
    let scale = FEATURE as f32 / wide.max(high);
    let (offset_x, offset_y) = (
        (FEATURE as f32 - wide * scale) / 2.,
        (FEATURE as f32 - high * scale) / 2.,
    );
    let mut shape = vec![0.; FEATURE * FEATURE];
    let mut brightness = vec![0.; FEATURE * FEATURE];
    for gy in 0..FEATURE {
        for gx in 0..FEATURE {
            let (mut covered, mut light) = (0usize, 0f32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let fx = gx as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let fy = gy as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    let x = left as f32 + (fx - offset_x) / scale;
                    let y = top as f32 + (fy - offset_y) / scale;
                    if x < left as f32 || y < top as f32 || x >= right as f32 || y >= bottom as f32
                    {
                        continue;
                    }
                    let index = (y as usize).min(size - 1) * size + (x as usize).min(size - 1);
                    if silhouette.inside[index] {
                        covered += 1;
                        light += luma(rgb[index]);
                    }
                }
            }
            let at = gy * FEATURE + gx;
            shape[at] = covered as f32 / (SAMPLES * SAMPLES) as f32;
            if covered > 0 {
                brightness[at] = light / covered as f32 / 255.;
            }
        }
    }
    Features {
        shape,
        brightness,
        height: high / size as f32,
        width: wide / size as f32,
    }
}

/// How unlike two silhouettes are: the part of either's shape the other lacks, the difference in
/// brightness where both cover, and the difference in their size on the square.
pub(crate) fn unlike(a: &Features, b: &Features) -> f32 {
    let (mut apart, mut either, mut both, mut shade) = (0., 0., 0., 0.);
    for at in 0..FEATURE * FEATURE {
        let (sa, sb) = (a.shape[at], b.shape[at]);
        apart += (sa - sb).abs();
        either += sa.max(sb);
        let common = sa.min(sb);
        both += common;
        shade += common * (a.brightness[at] - b.brightness[at]).abs();
    }
    let shape = if either > 0. { apart / either } else { 1. };
    let brightness = if both > 0. { shade / both } else { 1. };
    shape + 1.5 * brightness + 1.5 * ((a.height - b.height).abs() + (a.width - b.width).abs())
}

/// Which way up the board is by its coordinates, where the squares carry them: `Some(true)`
/// when White is at the bottom, `None` when none tell. Along a side edge, the rank numbers'
/// "8" is the one with two holes, and the "1" the narrowest by far; along the top or bottom
/// edge, the file letters' "a" has a hole and the "h" none.
pub(crate) fn ones_at_bottom(cells: &[Cell], size: usize) -> Option<bool> {
    // Each cell's marks alone in a corner: the corner, the width and the holes.
    let marks: Vec<Vec<(usize, usize, usize)>> = cells
        .iter()
        .map(|cell| {
            let on = differing(size, cell, &[]);
            parts(size, &on)
                .into_iter()
                .filter(|(pixels, _)| pixels.len() * 300 >= size * size)
                .filter_map(|(pixels, bounds)| {
                    let corner = corner_of(size, bounds)?;
                    Some((corner, bounds[2] - bounds[0], holes(size, &pixels, bounds)))
                })
                .collect()
        })
        .collect();
    let mark = |cell: usize, corner: usize| {
        marks
            .get(cell)
            .and_then(|marks| marks.iter().find(|(at, ..)| *at == corner))
            .map(|&(_, wide, holes)| (wide, holes))
    };
    let mut votes = 0i32;
    for corner in 0..4 {
        for column in [0, 7] {
            let ranks: Vec<Option<(usize, usize)>> =
                (0..8).map(|row| mark(row * 8 + column, corner)).collect();
            votes += ranks_vote(&ranks);
        }
        for row in [0, 7] {
            let files: Vec<Option<(usize, usize)>> = (0..8)
                .map(|column| mark(row * 8 + column, corner))
                .collect();
            votes += files_vote(&files);
        }
    }
    (votes != 0).then_some(votes > 0)
}

/// What a side's rank numbers, each a mark's width and holes from the top row down, say of which
/// way up the board is: 1 when White is at the bottom, -1 when Black is, 0 when they do not tell.
/// The "8" is the one with two holes, at the top or the bottom alone; the "1" is the narrowest,
/// by two pixels and a quarter.
fn ranks_vote(ranks: &[Option<(usize, usize)>]) -> i32 {
    if ranks.iter().flatten().count() < 4 {
        return 0;
    }
    let mut votes = 0;
    let eight = |row: usize| ranks[row].is_some_and(|(_, holes)| holes == 2);
    if eight(0) && !eight(7) {
        votes += 1;
    } else if eight(7) && !eight(0) {
        votes -= 1;
    }
    let mut widths: Vec<(usize, usize)> = ranks
        .iter()
        .enumerate()
        .filter_map(|(row, mark)| mark.map(|(wide, _)| (wide, row)))
        .collect();
    widths.sort_unstable();
    if let [(narrowest, row), (next, _), ..] = widths[..]
        && narrowest + 2 <= next
        && narrowest as f32 <= 0.75 * next as f32
    {
        votes += match row {
            7 => 1,
            0 => -1,
            _ => 0,
        };
    }
    votes
}

/// What a side's file letters, from the left column, say of which way up the board is, as
/// `ranks_vote`: the "a" has a hole and the "h" none.
fn files_vote(files: &[Option<(usize, usize)>]) -> i32 {
    if files.iter().flatten().count() < 4 {
        return 0;
    }
    let holes = |column: usize| files[column].map(|(_, holes)| holes);
    match (holes(0), holes(7)) {
        (Some(1), Some(0)) => 1,
        (Some(0), Some(1)) => -1,
        _ => 0,
    }
}

/// Which way up the board is by coordinates printed around it, beside each row and below or
/// above each column, as `ones_at_bottom`: `Some(true)` when White is at the bottom.
pub(crate) fn margin_ones_at_bottom(picture: &Picture, grid: &Grid) -> Option<bool> {
    let reach = grid.width.min(grid.height) * 0.6;
    let (right, bottom) = (grid.x + 8. * grid.width, grid.y + 8. * grid.height);
    let row = |row: usize| {
        (
            grid.y + row as f32 * grid.height,
            grid.y + (row + 1) as f32 * grid.height,
        )
    };
    let column = |column: usize| {
        (
            grid.x + column as f32 * grid.width,
            grid.x + (column + 1) as f32 * grid.width,
        )
    };
    let mut votes = 0;
    for (from, to) in [(grid.x - reach, grid.x), (right, right + reach)] {
        let ranks: Vec<Option<(usize, usize)>> = (0..8)
            .map(|at| {
                let (top, bottom) = row(at);
                glyph(picture, [from, top, to, bottom])
            })
            .collect();
        votes += ranks_vote(&ranks);
    }
    for (from, to) in [(grid.y - reach, grid.y), (bottom, bottom + reach)] {
        let files: Vec<Option<(usize, usize)>> = (0..8)
            .map(|at| {
                let (left, right) = column(at);
                glyph(picture, [left, from, right, to])
            })
            .collect();
        votes += files_vote(&files);
    }
    (votes != 0).then_some(votes > 0)
}

/// The largest mark standing alone in a region of `picture` (`[left, top, right, bottom]`, cut
/// to the picture): its width and how many holes it has. The marks touching the region's edges
/// (a frame's line, the board, the page beyond) are left out, and specks.
fn glyph(picture: &Picture, [left, top, right, bottom]: [f32; 4]) -> Option<(usize, usize)> {
    let (left, top) = (left.max(0.).ceil() as usize, top.max(0.).ceil() as usize);
    let right = right.min(picture.width as f32).floor() as usize;
    let bottom = bottom.min(picture.height as f32).floor() as usize;
    if right < left + 3 || bottom < top + 5 {
        return None;
    }
    let (width, height) = (right - left, bottom - top);
    let at = |x: usize, y: usize| picture.rgb[(top + y) * picture.width + left + x];
    let background =
        median_colour((0..width * height).map(|index| at(index % width, index / width)))?;
    // The region's ink in a square as wide as its longer side, as `parts` reads.
    let side = width.max(height);
    let ink: Vec<bool> = (0..side * side)
        .map(|index| {
            let (x, y) = (index % side, index / side);
            x < width && y < height && distance(at(x, y), background) > GLYPH_INK
        })
        .collect();
    let (pixels, bounds) = parts(side, &ink)
        .into_iter()
        .filter(|(pixels, [left, top, right, bottom])| {
            pixels.len() >= 4
                && *left > 0
                && *top > 0
                && *right < width
                && *bottom < height
                && (bottom - top) as f32 >= 0.15 * height as f32
        })
        .max_by_key(|(pixels, _)| pixels.len())?;
    Some((bounds[2] - bounds[0], holes(side, &pixels, bounds)))
}

/// How far a coordinate's pixels stand from the margin's colour.
const GLYPH_INK: f32 = 60.;

/// How many holes a mark has: parts of its bounds it does not cover and encloses.
fn holes(size: usize, pixels: &[usize], [left, top, right, bottom]: Bounds) -> usize {
    // The bounds with a pixel's border all round, in a square, what the mark covers set.
    let side = (right - left).max(bottom - top) + 2;
    let mut open = vec![true; side * side];
    for &index in pixels {
        let (x, y) = (index % size - left + 1, index / size - top + 1);
        open[y * side + x] = false;
    }
    // Every open part but the one around the mark.
    parts(side, &open)
        .iter()
        .filter(|(part, _)| !part.contains(&0))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: usize, colour: [f32; 3]) -> Vec<[f32; 3]> {
        vec![colour; size * size]
    }

    /// A black ring on a light square is a piece whose inside is filled; a dot in a corner is a
    /// label, not a piece.
    #[test]
    fn an_outline_is_filled_and_a_corner_mark_is_not_a_piece() {
        let size = 40;
        let light = [240., 217., 181.];
        let mut rgb = square(size, light);
        for y in 10..30 {
            for x in 12..28 {
                if y == 10 || y == 29 || x == 12 || x == 27 {
                    rgb[y * size + x] = [0.; 3];
                }
            }
        }
        let cell = Cell::new(size, rgb.clone(), 0.);
        assert_eq!(cell.background, light);
        let shape = silhouette(size, &cell, &[]).unwrap();
        assert_eq!(shape.bounds, [12, 10, 28, 30]);
        assert!((shape.area - 320. / 1600.).abs() < 1e-6);
        let features = features(&shape, &rgb);
        assert!(unlike(&features, &features) < 1e-6);

        let mut marked = square(size, light);
        for y in 33..38 {
            for x in 34..38 {
                marked[y * size + x] = [0.; 3];
            }
        }
        let found = silhouette(size, &Cell::new(size, marked, 0.), &[])
            .err()
            .unwrap();
        assert!(found > 0. && found < PIECE_AREA);
    }

    /// A `size`-pixel dark square of a printed diagram, read as a textured square is: rising
    /// lines `spacing` apart and `thickness` thick in black and white, with what `drawn` puts
    /// over them (ink 0, paper 255, `None` the hatching), averaged over the period.
    fn printed(
        size: usize,
        spacing: usize,
        thickness: usize,
        drawn: impl Fn(usize, usize) -> Option<f32>,
    ) -> Cell {
        let crisp: Vec<f32> = (0..size * size)
            .map(|index| {
                let (x, y) = (index % size, index / size);
                drawn(x, y).unwrap_or(if (x + y) % spacing < thickness {
                    0.
                } else {
                    255.
                })
            })
            .collect();
        let picture = Picture {
            width: size,
            height: size,
            rgb: crisp.iter().map(|&v| [v; 3]).collect(),
        };
        let blur = spacing as f32;
        let mut cell = Cell::new(size, crate::texture::smooth(&picture, blur).rgb, blur);
        let tone = 255. * (1. - thickness as f32 / spacing as f32);
        cell.background = [tone; 3];
        cell.paper = (255. + tone) / 2.;
        cell.floor = (0..size * size)
            .filter(|&index| band(size, index, blur).0)
            .map(|index| luma(cell.rgb[index]))
            .fold(f32::MAX, f32::min);
        cell.crisp = crisp;
        cell
    }

    /// A white figurine on a hatched square, an outline around paper, is a piece mostly paper
    /// inside; a black one, solid ink, is a piece of ink.
    #[test]
    fn figurines_on_hatching_are_pieces_white_or_black() {
        let size = 48;
        let inside = |x: usize, y: usize| (14..34).contains(&x) && (10..38).contains(&y);
        let edge =
            |x: usize, y: usize| inside(x, y) && !((16..32).contains(&x) && (12..36).contains(&y));
        let white = printed(size, 6, 3, |x, y| {
            inside(x, y).then(|| if edge(x, y) { 0. } else { 255. })
        });
        let shape = silhouette(size, &white, &[]).unwrap();
        let [left, top, right, bottom] = shape.bounds;
        assert!(
            left.abs_diff(14) <= 4 && right.abs_diff(34) <= 4,
            "{:?}",
            shape.bounds
        );
        assert!(
            top.abs_diff(10) <= 4 && bottom.abs_diff(38) <= 4,
            "{:?}",
            shape.bounds
        );
        assert!(inkiness(&shape, &white) < 0.3);

        let black = printed(size, 6, 3, |x, y| inside(x, y).then_some(0.));
        let shape = silhouette(size, &black, &[]).unwrap();
        assert!(shape.area > 0.2, "{}", shape.area);
        assert!(inkiness(&shape, &black) > 0.9);
    }

    /// Hatching alone is no piece: evenly drawn, with one line a pixel thicker than the rest
    /// (as a one-bit picture draws lines a fraction of a pixel apart), or with a stretch of a
    /// line missing (a worn print); nor is the board's frame reaching into a corner square.
    #[test]
    fn hatching_and_the_frame_are_no_piece() {
        let size = 48;
        let empty = printed(size, 6, 3, |_, _| None);
        assert!(silhouette(size, &empty, &[]).is_err());
        let thicker = printed(size, 6, 3, |x, y| ((x + y) % 48 == 3).then_some(0.));
        assert!(silhouette(size, &thicker, &[]).is_err());
        let worn = printed(size, 6, 3, |x, y| {
            ((x + y) % 6 < 3 && (x + y) / 6 == 8 && (18..30).contains(&x)).then_some(255.)
        });
        assert!(silhouette(size, &worn, &[]).is_err());
        let framed = printed(size, 6, 3, |x, y| (x < 4 || y < 4).then_some(0.));
        assert!(silhouette(size, &framed, &[]).is_err());
    }

    /// On a textured board every square takes the colour and grain its parity's squares
    /// typically show, a square a large piece reaches into the band of included.
    #[test]
    fn a_textured_boards_squares_take_their_paritys_colour() {
        let size = 30;
        let (light, dark) = ([250.; 3], [120.; 3]);
        let mut cells: Vec<Cell> = (0..64)
            .map(|index| {
                let colour = if (index / 8 + index % 8) % 2 == 0 {
                    light
                } else {
                    dark
                };
                Cell::new(size, square(size, colour), 4.)
            })
            .collect();
        cells[9] = Cell::new(size, square(size, [60.; 3]), 4.);
        let evened = even_out(&mut cells, [[240.; 3], [126.; 3]]);
        assert_eq!(evened, [light, dark]);
        for (index, cell) in cells.iter().enumerate() {
            let colour = if (index / 8 + index % 8) % 2 == 0 {
                light
            } else {
                dark
            };
            assert_eq!(cell.background, colour);
            assert_eq!(cell.spread, 0.);
        }
        assert!((cells[1].floor - 120.).abs() < 0.01);
    }
}
