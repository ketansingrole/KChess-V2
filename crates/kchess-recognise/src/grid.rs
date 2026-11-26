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

use crate::picture::{Picture, distance, median_colour};

/// The flat stretch, in pixels, each side of an edge: the pixel either side of the boundary is
/// left out, since a scaled or compressed picture blurs it.
const SPAN: usize = 3;
/// How much brightness may vary along a stretch that is still flat.
const FLAT: f32 = 18.;
/// The least step in brightness across a square's edge.
const STEP: f32 = 8.;
/// The smallest square looked for, in pixels.
pub(crate) const MIN_SQUARE: f32 = 7.;
/// How far past the picture's edge, as a part of a square, a board looked for again may reach:
/// a screenshot cut a little into it leaves its outer squares narrower than the rest.
pub(crate) const CUT: f32 = 0.2;

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
    /// How wide the lines drawn between the squares are, if any: what of each square's width
    /// is line, half at either side.
    pub gap: f32,
}

/// How strongly a square's edge runs down each boundary between columns (`0..=width`), within
/// `rows`.
pub(crate) fn edges(luma: &[f32], width: usize, rows: Range<usize>, skip: usize) -> Vec<f32> {
    let mut rise = vec![0f32; width + 1];
    let mut fall = vec![0f32; width + 1];
    if width < 2 * (SPAN + skip) {
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
        for boundary in SPAN + skip..=width - SPAN - skip {
            let (Some(left), Some(right)) = (
                flat(&row[boundary - skip - SPAN..boundary - skip]),
                flat(&row[boundary + skip..boundary + skip + SPAN]),
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
/// `keep` strongest that differ; reaching a pixel past either end of `edges`, or `cut` of a
/// square.
pub(crate) fn search(
    edges: &[f32],
    steps: (f32, f32),
    increment: f32,
    keep: usize,
    cut: f32,
) -> Vec<Lines> {
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
    while step <= steps.1 && 8. * step <= len + 2. * (step * cut).max(1.) {
        let reach = (step * cut).max(1.);
        let mut best = Lines {
            start: 0.,
            step,
            score: 0.,
        };
        // Whole pixels, as within the picture.
        let mut start = -reach.ceil();
        while start + 8. * step <= len + reach {
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
    skip: usize,
    cut: f32,
}

/// From lines found along one axis, the board: the lines across it within the columns they span,
/// then along again within those rows.
fn follow(axes: &Axes, first: Lines) -> Vec<(Lines, Lines)> {
    let mut found = Vec::new();
    let across = edges(axes.across, axes.height, span(first, axes.width), axes.skip);
    for second in search(
        &across,
        (first.step * 0.92, first.step * 1.08),
        0.25,
        2,
        axes.cut,
    ) {
        let second = refine(&across, second);
        let along = edges(axes.along, axes.width, span(second, axes.height), axes.skip);
        if let Some(&again) = search(
            &along,
            (second.step * 0.95, second.step * 1.05),
            0.25,
            1,
            axes.cut,
        )
        .first()
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
/// Boards reaching `cut` of a square past the picture's edges are looked for too.
pub(crate) fn candidates(picture: &Picture, skip: usize, cut: f32) -> Vec<Grid> {
    let (width, height) = (picture.width, picture.height);
    let luma = picture.luma();
    let turned = transpose(&luma, width, height);
    let largest = width.min(height) as f32 / (8. - 2. * cut);
    let mut grids = Vec::new();
    let columns = Axes {
        along: &luma,
        across: &turned,
        width,
        height,
        skip,
        cut,
    };
    let rows = Axes {
        along: &turned,
        across: &luma,
        width: height,
        height: width,
        skip,
        cut,
    };
    for first in search(
        &edges(&luma, width, 0..height, skip),
        (MIN_SQUARE, largest),
        0.5,
        3,
        cut,
    ) {
        for (x, y) in follow(&columns, first) {
            grids.push(Grid {
                x: x.start,
                y: y.start,
                width: x.step,
                height: y.step,
                score: x.score + y.score,
                gap: 0.,
            });
        }
    }
    for first in search(
        &edges(&turned, height, 0..width, skip),
        (MIN_SQUARE, largest),
        0.5,
        3,
        cut,
    ) {
        for (y, x) in follow(&rows, first) {
            grids.push(Grid {
                x: x.start,
                y: y.start,
                width: x.step,
                height: y.step,
                score: x.score + y.score,
                gap: 0.,
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

/// How wide the lines drawn between `grid`'s squares are, in pixels: across each square's edge,
/// the run of columns (and rows) that are one colour all down the board, where a column through
/// squares alternates between their two `colours`. A column one blend of the two all down (an
/// edge between pixels, in a scaled picture) is no line: a line is a colour of its own.
pub(crate) fn gap(picture: &Picture, grid: &Grid, colours: [[f32; 3]; 2]) -> f32 {
    let (width, height) = (picture.width, picture.height);
    // Whether the line through the board at `at` across it is one colour of its own.
    let line = |at: f32, down: bool| -> bool {
        let (start, step, len, across_len) = if down {
            (grid.y, grid.height, height, width)
        } else {
            (grid.x, grid.width, width, height)
        };
        if at < 0. || at as usize >= across_len {
            return false;
        }
        let pixels: Vec<[f32; 3]> = (0..(8. * step) as usize)
            .map(|k| start + k as f32)
            .filter(|&along| along >= 0. && (along as usize) < len)
            .map(|along| {
                let (x, y) = if down {
                    (at as usize, along as usize)
                } else {
                    (along as usize, at as usize)
                };
                picture.rgb[y * width + x]
            })
            .collect();
        let Some(colour) = median_colour(pixels.iter().copied()) else {
            return false;
        };
        let alike = pixels
            .iter()
            .filter(|&&p| distance(p, colour) <= 20.)
            .count();
        alike * 10 >= pixels.len() * 8 && off_between(colour, colours) > 24.
    };
    let mut widths = Vec::new();
    for down in [true, false] {
        let (start, step) = if down {
            (grid.x, grid.width)
        } else {
            (grid.y, grid.height)
        };
        let reach = (step * 0.1).ceil() as isize;
        for k in 1..8 {
            let edge = start + k as f32 * step;
            let lined: Vec<bool> = (-reach..=reach)
                .map(|offset| line((edge + offset as f32).floor(), down))
                .collect();
            // The run of line columns through the edge, or beside it.
            let middle = reach as usize;
            let Some(centre) = [middle, middle - 1, middle + 1]
                .into_iter()
                .find(|&at| lined.get(at) == Some(&true))
            else {
                widths.push(0.);
                continue;
            };
            let before = lined[..centre].iter().rev().take_while(|l| **l).count();
            let after = lined[centre + 1..].iter().take_while(|l| **l).count();
            widths.push((before + after + 1) as f32);
        }
    }
    widths.sort_by(f32::total_cmp);
    // Most edges show it; a piece standing across an edge's whole length cannot.
    widths[widths.len() / 2]
}

/// How far `colour` is from every blend of `colours`.
fn off_between(colour: [f32; 3], [a, b]: [[f32; 3]; 2]) -> f32 {
    let along = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = along.iter().map(|c| c * c).sum::<f32>();
    let share = if length > 0. {
        ((0..3).map(|c| (colour[c] - a[c]) * along[c]).sum::<f32>() / length).clamp(0., 1.)
    } else {
        0.
    };
    distance(colour, [0, 1, 2].map(|c| a[c] + share * along[c]))
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
        let grid = candidates(&picture, 1, 0.)[0];
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

    /// A board cut into by the picture's edges is found as one reaching past them, when looked
    /// for so.
    #[test]
    fn a_board_cut_into_is_found_past_the_edges() {
        let whole = checkerboard((480, 480), 0, 0, 60);
        let side = 466;
        let mut rgb = Vec::with_capacity(side * side);
        for y in 7..7 + side {
            rgb.extend_from_slice(&whole.rgb[y * 480 + 7..y * 480 + 7 + side]);
        }
        let cut = Picture {
            width: side,
            height: side,
            rgb,
        };
        assert!(
            candidates(&cut, 1, 0.)
                .iter()
                .all(|grid| (grid.width - 60.).abs() > 1.)
        );
        let grid = candidates(&cut, 1, CUT)[0];
        assert!((grid.x + 7.).abs() < 0.5, "{grid:?}");
        assert!((grid.width - 60.).abs() < 0.1, "{grid:?}");
    }

    /// Lines drawn between the squares are measured; a plain board's edges, and a scaled one's
    /// edges blended over a pixel, are no line.
    #[test]
    fn lines_between_the_squares_are_measured() {
        let colours = [[240., 217., 181.], [181., 136., 99.]];
        let mut picture = checkerboard((400, 400), 0, 0, 50);
        let grid = Grid {
            x: 0.,
            y: 0.,
            width: 50.,
            height: 50.,
            score: 1.,
            gap: 0.,
        };
        assert_eq!(gap(&picture, &grid, colours), 0.);
        // Each edge a pixel blended half and half, as scaling leaves it.
        let mut blended = Picture {
            rgb: picture.rgb.clone(),
            ..picture
        };
        for k in 1..8 {
            for along in 0..400 {
                for (x, y) in [(k * 50, along), (along, k * 50)] {
                    let (a, b) = (picture.rgb[y * 400 + x - 1], picture.rgb[y * 400 + x]);
                    blended.rgb[y * 400 + x] = [0, 1, 2].map(|c| (a[c] + b[c]) / 2.);
                }
            }
        }
        assert_eq!(gap(&blended, &grid, colours), 0.);
        for k in 1..8 {
            for along in 0..400 {
                for across in k * 50 - 2..k * 50 + 2 {
                    picture.rgb[along * 400 + across] = [231., 196., 106.];
                    picture.rgb[across * 400 + along] = [231., 196., 106.];
                }
            }
        }
        assert_eq!(gap(&picture, &grid, colours), 4.);
    }

    #[test]
    fn a_blank_page_has_no_board() {
        let picture = Picture {
            width: 200,
            height: 200,
            rgb: vec![[250.; 3]; 200 * 200],
        };
        assert!(candidates(&picture, 1, 0.).is_empty());
        assert_eq!(
            transpose(&[1., 2., 3., 4., 5., 6.], 3, 2),
            [1., 4., 2., 5., 3., 6.]
        );
    }
}
