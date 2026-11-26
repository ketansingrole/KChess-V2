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
    pub blur: usize,
    /// On a textured picture, each pixel's brightness before averaging; else empty.
    pub crisp: Vec<f32>,
    /// On a textured picture, the brightness below which a pixel of `crisp` is ink.
    pub paper: f32,
}

/// How far in from a cell's edges its neighbours' colours reach: a pixel when the board is
/// placed a pixel off, and half the width a textured picture was averaged over.
fn inset(size: usize, blur: usize) -> usize {
    ((size as f32 * 0.03).round() as usize).max(1) + blur / 2
}

/// Whether a pixel of a `size`-pixel cell is in the band its colour is taken from, `inset` to
/// `inset + width` in from its edges, and how far in it is.
fn band(size: usize, index: usize, blur: usize) -> (bool, usize) {
    let (x, y) = (index % size, index / size);
    let from_edge = x.min(y).min(size - 1 - x).min(size - 1 - y);
    let inset = inset(size, blur);
    let width = ((size as f32 * 0.06).round() as usize).max(2);
    (from_edge >= inset && from_edge < inset + width, from_edge)
}

/// The 64 squares `grid` places in `picture`, from the top left, row by row.
pub(crate) fn cells(picture: &Picture, grid: &Grid, size: usize, blur: usize) -> Vec<Cell> {
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
            let left = grid.x + column as f32 * grid.width;
            let top = grid.y + row as f32 * grid.height;
            let (step_x, step_y) = (grid.width / size as f32, grid.height / size as f32);
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
    pub fn new(size: usize, rgb: Vec<[f32; 3]>, blur: usize) -> Cell {
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

/// On a textured picture every square takes its colour from the board: the one of `colours`
/// its parity's squares show, with the grain they typically show. Averaged over the texture,
/// a large piece reaches into the band a square's own colour is taken from, and next to the
/// board's frame so does the frame; a printed diagram highlights no square.
pub(crate) fn even_out(cells: &mut [Cell], colours: [[f32; 3]; 2]) {
    for parity in 0..2 {
        let of_parity = |index: &usize| (index / 8 + index % 8) % 2 == parity;
        let Some(typical) = median_colour(
            (0..cells.len())
                .filter(of_parity)
                .map(|index| cells[index].background),
        ) else {
            continue;
        };
        let colour = if distance(typical, colours[0]) <= distance(typical, colours[1]) {
            colours[0]
        } else {
            colours[1]
        };
        let mut spreads: Vec<f32> = (0..cells.len())
            .filter(of_parity)
            .map(|index| cells[index].spread)
            .collect();
        let grain = median(&mut spreads).unwrap_or_default();
        for index in (0..cells.len()).filter(of_parity) {
            cells[index].background = colour;
            cells[index].spread = grain;
        }
    }
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

/// Which of a cell's pixels differ from its colour, leaving out the outermost (a neighbour's
/// colour bleeds into them when the board is placed a pixel off), a coordinate label's in
/// `labels`, and lone specks (a compressed picture's noise).
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
            let apart = if blur > 0 {
                luma(background) - luma(colour)
            } else {
                distance(colour, background)
            };
            !label && band(size, index, blur).1 >= inset && (apart > threshold || enclosed[index])
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
    let parts = parts(size, &inner);
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
    // Or along two edges at once, the board's frame in a corner square.
    let along_edges = |pixels: &Vec<usize>| {
        let thin = size as f32 * 0.1;
        pixels.iter().all(|&index| {
            let (x, y) = (index % size, index / size);
            (x.min(y).min(size - 1 - x).min(size - 1 - y) as f32) < thin
        })
    };
    let mark = |pixels: &Vec<usize>, bounds: Bounds| {
        (pixels.len() as f32) < MARK_AREA * whole
            || corner_of(size, bounds).is_some()
            || strip(bounds)
            || along_edges(pixels)
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
        let cell = Cell::new(size, rgb.clone(), 0);
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
        let found = silhouette(size, &Cell::new(size, marked, 0), &[])
            .err()
            .unwrap();
        assert!(found > 0. && found < PIECE_AREA);
    }
}
