//! Finding the board: its squares' edges, as long straight steps in brightness that change
//! direction every square, since they pass light squares and dark ones in turn.
//!
//! Each boundary between two pixel columns gets a strength: down it, every row where a short flat
//! stretch of colour on its left meets a flat stretch of another on its right adds that step,
//! rising or falling; the strength is the smaller of the two sums. A square's edge rises past half
//! its length and falls past the other half, so it scores high; a piece's outline is no flat
//! stretch on its inner side, and the edge of the board against the page only rises or only
//! falls, so neither does. The board's seven inner edges are the seven evenly spaced boundaries
//! that score highest together; the rows are found the same way across the columns that spans,
//! then the columns again across those rows.

use std::ops::Range;

use crate::picture::Picture;

/// The flat stretch, in pixels, each side of an edge: the pixel either side of the boundary is
/// left out, since a scaled or compressed picture blurs it.
const SPAN: usize = 3;
/// How much brightness may vary along a stretch that is still flat.
const FLAT: f32 = 18.;
/// The least step in brightness across a square's edge.
const STEP: f32 = 8.;
/// The smallest square looked for, in pixels.
pub(crate) const MIN_SQUARE: f32 = 7.;

/// Evenly spaced lines along one axis: the board's edge at `start`, then a square's edge every
/// `step`; `score` is how strongly its seven inner edges show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lines {
    pub start: f32,
    pub step: f32,
    pub score: f32,
}

/// Where the board lies: its top left corner and its squares' width and height, in the picture's
/// pixels (pixel `x` covers `x` to `x + 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub score: f32,
}

/// How strongly a square's edge runs down each boundary between columns (`0..=width`), within
/// `rows`.
pub(crate) fn edges(luma: &[f32], width: usize, rows: Range<usize>) -> Vec<f32> {
    let mut rise = vec![0f32; width + 1];
    let mut fall = vec![0f32; width + 1];
    if width < 2 * SPAN + 2 {
        return rise;
    }
    let flat = |stretch: &[f32]| {
        let (low, high) = stretch
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), &v| {
                (low.min(v), high.max(v))
            });
        (high - low <= FLAT).then(|| stretch.iter().sum::<f32>() / stretch.len() as f32)
    };
    for y in rows {
        let Some(row) = luma.get(y * width..(y + 1) * width) else {
            break;
        };
        for boundary in SPAN + 1..=width - SPAN - 1 {
            let (Some(left), Some(right)) = (
                flat(&row[boundary - 1 - SPAN..boundary - 1]),
                flat(&row[boundary + 1..boundary + 1 + SPAN]),
            ) else {
                continue;
            };
            let step = right - left;
            if step >= STEP {
                rise[boundary] += step;
            } else if step <= -STEP {
                fall[boundary] -= step;
            }
        }
    }
    rise.iter().zip(&fall).map(|(r, f)| r.min(*f)).collect()
}

/// `luma` (`width` × `height`) turned so its rows are the picture's columns.
pub(crate) fn transpose(luma: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut turned = vec![0.; luma.len()];
    for y in 0..height {
        for x in 0..width {
            turned[x * height + y] = luma[y * width + x];
        }
    }
    turned
}

/// Evenly spaced lines along `edges` with a step in `steps` (tried every `increment` pixels), the
/// `keep` strongest that differ.
pub(crate) fn search(edges: &[f32], steps: (f32, f32), increment: f32, keep: usize) -> Vec<Lines> {
    let len = edges.len().saturating_sub(1) as f32;
    // Each boundary's strength, as the strongest within a pixel of it (an edge blurred over two
    // pixels shows on either), square-rooted so seven fair edges beat three strong ones.
    let near: Vec<f32> = (0..edges.len())
        .map(|at| {
            edges[at.saturating_sub(1)..(at + 2).min(edges.len())]
                .iter()
                .fold(0f32, |a, &b| a.max(b))
                .sqrt()
        })
        .collect();
    let strength = |at: f32| near.get(at.round() as usize).copied().unwrap_or(0.);
    let mut found = Vec::new();
    let mut step = steps.0.max(MIN_SQUARE);
    while step <= steps.1 && 8. * step <= len + 2. {
        let mut best = Lines {
            start: 0.,
            step,
            score: 0.,
        };
        let mut start = -1f32;
        while start + 8. * step <= len + 1. {
            let score: f32 = (1..8).map(|k| strength(start + k as f32 * step)).sum();
            if score > best.score {
                best = Lines { start, step, score };
            }
            start += 1.;
        }
        if best.score > 0. {
            found.push(best);
        }
        step += increment;
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Lines> = Vec::new();
    for lines in found {
        let differs = |other: &Lines| {
            (lines.step - other.step).abs() > 0.06 * other.step
                || (lines.start - other.start).abs() > 0.3 * other.step
        };
        if kept.iter().all(differs) {
            kept.push(lines);
            if kept.len() == keep {
                break;
            }
        }
    }
    kept
}

/// `lines` placed to a fraction of a pixel: each inner edge at the middle of its strongest
/// response near where `lines` put it (the nearer the better: a piece's flat base beside a
/// square's edge may step as strongly), and the line through them, each weighed by its strength;
/// an edge far off that line, a piece's after all, is left out and the line drawn again.
pub(crate) fn refine(edges: &[f32], lines: Lines) -> Lines {
    let mut points = Vec::new();
    let radius = (lines.step * 0.15).max(2.);
    for k in 1..8 {
        let guess = lines.start + k as f32 * lines.step;
        let low = (guess - radius).floor().max(0.) as usize;
        let high = ((guess + radius).ceil() as usize).min(edges.len().saturating_sub(1));
        let near = |at: usize| {
            let off = (at as f32 - guess) / (radius + 1.);
            edges[at] * (1. - off * off)
        };
        let Some(peak_at) = (low..=high).max_by(|&a, &b| near(a).total_cmp(&near(b))) else {
            continue;
        };
        let peak = edges[peak_at];
        if peak <= 0. {
            continue;
        }
        let (mut weight, mut sum) = (0., 0.);
        let from = peak_at.saturating_sub(2);
        let around = edges
            .get(from..=(peak_at + 2).min(edges.len() - 1))
            .unwrap_or_default();
        for (at, &edge) in (from..).zip(around) {
            if edge >= 0.5 * peak {
                weight += edge;
                sum += edge * at as f32;
            }
        }
        points.push((k as f32, sum / weight, peak.sqrt()));
    }
    let Some((start, step)) = fit(&points) else {
        return lines;
    };
    let tolerance = (lines.step * 0.03).max(1.5);
    let close: Vec<_> = points
        .iter()
        .copied()
        .filter(|&(k, x, _)| (x - start - k * step).abs() <= tolerance)
        .collect();
    let (start, step) = if close.len() < points.len() {
        match fit(&close) {
            Some(fitted) => fitted,
            None => return lines,
        }
    } else {
        (start, step)
    };
    if (step - lines.step).abs() > 0.1 * lines.step {
        return lines;
    }
    Lines {
        step,
        start,
        ..lines
    }
}

/// The weighted least-squares line `x = start + k × step` through `points` (`k`, `x`, weight),
/// from four points or more.
fn fit(points: &[(f32, f32, f32)]) -> Option<(f32, f32)> {
    if points.len() < 4 {
        return None;
    }
    let (mut w, mut sk, mut sx, mut skk, mut skx) = (0., 0., 0., 0., 0.);
    for &(k, x, weight) in points {
        w += weight;
        sk += weight * k;
        sx += weight * x;
        skk += weight * k * k;
        skx += weight * k * x;
    }
    let denominator = w * skk - sk * sk;
    if denominator.abs() < f32::EPSILON {
        return None;
    }
    let step = (w * skx - sk * sx) / denominator;
    Some(((sx - step * sk) / w, step))
}

/// The pixels `lines`' board spans, within `len`.
fn span(lines: Lines, len: usize) -> Range<usize> {
    let start = lines.start.max(0.) as usize;
    let end = ((lines.start + 8. * lines.step).ceil().max(0.) as usize).min(len);
    start.min(end)..end
}

/// One way of reading the picture: `along` its rows (`width` wide), `across` it turned.
struct Axes<'a> {
    along: &'a [f32],
    across: &'a [f32],
    width: usize,
    height: usize,
}

/// From lines found along one axis, the board: the lines across it within the columns they span,
/// then along again within those rows.
fn follow(axes: &Axes, first: Lines) -> Vec<(Lines, Lines)> {
    let mut found = Vec::new();
    let across = edges(axes.across, axes.height, span(first, axes.width));
    for second in search(&across, (first.step * 0.92, first.step * 1.08), 0.25, 2) {
        let second = refine(&across, second);
        let along = edges(axes.along, axes.width, span(second, axes.height));
        if let Some(&again) =
            search(&along, (second.step * 0.95, second.step * 1.05), 0.25, 1).first()
        {
            let again = refine(&along, again);
            found.push((
                again,
                Lines {
                    score: second.score,
                    ..second
                },
            ));
        }
    }
    found
}

/// Where the board may be, most likely first: from the columns' edges first, and from the rows'.
pub(crate) fn candidates(picture: &Picture) -> Vec<Grid> {
    let (width, height) = (picture.width, picture.height);
    let luma = picture.luma();
    let turned = transpose(&luma, width, height);
    let largest = width.min(height) as f32 / 8.;
    let mut grids = Vec::new();
    let columns = Axes {
        along: &luma,
        across: &turned,
        width,
        height,
    };
    let rows = Axes {
        along: &turned,
        across: &luma,
        width: height,
        height: width,
    };
    for first in search(
        &edges(&luma, width, 0..height),
        (MIN_SQUARE, largest),
        0.5,
        3,
    ) {
        for (x, y) in follow(&columns, first) {
            grids.push(Grid {
                x: x.start,
                y: y.start,
                width: x.step,
                height: y.step,
                score: x.score + y.score,
            });
        }
    }
    for first in search(
        &edges(&turned, height, 0..width),
        (MIN_SQUARE, largest),
        0.5,
        3,
    ) {
        for (y, x) in follow(&rows, first) {
            grids.push(Grid {
                x: x.start,
                y: y.start,
                width: x.step,
                height: y.step,
                score: x.score + y.score,
            });
        }
    }
    grids.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut distinct: Vec<Grid> = Vec::new();
    for grid in grids {
        let same = |other: &Grid| {
            (grid.x - other.x).abs() < 0.25 * other.width
                && (grid.y - other.y).abs() < 0.25 * other.height
                && (grid.width - other.width).abs() < 0.05 * other.width
        };
        // A board is square, give or take a picture stretched a little.
        let square = (grid.width / grid.height - 1.).abs() < 0.08;
        if square && !distinct.iter().any(same) {
            distinct.push(grid);
        }
    }
    distinct
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain checkerboard with `square`-pixel squares at `(left, top)` on a white page.
    fn checkerboard(page: (usize, usize), left: usize, top: usize, square: usize) -> Picture {
        let (width, height) = page;
        let mut rgb = vec![[255.; 3]; width * height];
        for y in top..top + 8 * square {
            for x in left..left + 8 * square {
                let dark = ((x - left) / square + (y - top) / square) % 2 == 1;
                rgb[y * width + x] = if dark {
                    [181., 136., 99.]
                } else {
                    [240., 217., 181.]
                };
            }
        }
        Picture { width, height, rgb }
    }

    #[test]
    fn a_board_on_a_page_is_found_to_the_pixel() {
        let picture = checkerboard((500, 420), 37, 21, 45);
        let grid = candidates(&picture)[0];
        assert!((grid.x - 37.).abs() < 0.5, "{grid:?}");
        assert!((grid.y - 21.).abs() < 0.5, "{grid:?}");
        assert!((grid.width - 45.).abs() < 0.1, "{grid:?}");
        assert!((grid.height - 45.).abs() < 0.1, "{grid:?}");
    }

    /// A piece's flat base a little off a square's edge, stepping more strongly than the edge
    /// beside it, does not pull the board's lines off the squares (pixel-art pieces at 640 pixels
    /// put the bottom rank 8 pixels off).
    #[test]
    fn a_stronger_step_beside_an_edge_does_not_move_the_lines() {
        let mut edges = vec![0.; 700];
        for k in 1..8 {
            edges[27 + 80 * k..30 + 80 * k].fill(17_000.);
        }
        edges[572..574].fill(18_000.);
        let lines = Lines {
            start: 30.,
            step: 79.39,
            score: 1.,
        };
        let refined = refine(&edges, lines);
        assert!((refined.start - 28.).abs() < 0.1, "{refined:?}");
        assert!((refined.step - 80.).abs() < 0.01, "{refined:?}");
    }

    #[test]
    fn a_blank_page_has_no_board() {
        let picture = Picture {
            width: 200,
            height: 200,
            rgb: vec![[250.; 3]; 200 * 200],
        };
        assert!(candidates(&picture).is_empty());
        assert_eq!(
            transpose(&[1., 2., 3., 4., 5., 6.], 3, 2),
            [1., 4., 2., 5., 3., 6.]
        );
    }
}
