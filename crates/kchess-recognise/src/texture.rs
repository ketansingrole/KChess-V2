//! Textured squares: a printed diagram's dark squares are hatching, thin parallel lines on
//! white, not a colour. Seen from further away they are a tone, so a picture whose squares are
//! textured is read through that tone: the picture averaged over the texture's period.

use crate::picture::Picture;

/// The least depth, in brightness, of a texture's line against what lies either side of it.
const DEPTH: f32 = 32.;
/// The longest period looked for, in pixels.
const LONGEST: usize = 32;

/// The period, in pixels, of a texture covering much of `luma` (`width` × `height`): lines
/// whose spacing along the rows or the columns repeats. `None` when nothing repeats so.
pub(crate) fn period(luma: &[f32], width: usize, height: usize) -> Option<f32> {
    let mut spacings = [0usize; LONGEST + 1];
    let mut count = |line: &mut dyn Iterator<Item = f32>| {
        let values: Vec<f32> = line.collect();
        let mut last: Option<usize> = None;
        for at in dips(&values) {
            if let Some(last) = last
                && at - last <= LONGEST
            {
                spacings[at - last] += 1;
            }
            last = Some(at);
        }
    };
    for y in 0..height {
        count(&mut luma[y * width..(y + 1) * width].iter().copied());
    }
    for x in 0..width {
        count(&mut (0..height).map(|y| luma[y * width + x]));
    }
    // The commonest spacing, with its neighbours (a line drawn between pixels lands on either).
    let (best, covered) = (2..LONGEST)
        .map(|at| {
            let near = spacings[at - 1] + spacings[at] + spacings[at + 1];
            (at, near)
        })
        .max_by_key(|&(_, near)| near)?;
    let near = spacings[best - 1..=best + 1].iter().sum::<usize>() as f32;
    let mean = (best - 1..=best + 1)
        .map(|at| at as f32 * spacings[at] as f32)
        .sum::<f32>()
        / near.max(1.);
    // What the repeating lines cover, along the rows and the columns, as a part of the picture.
    let share = covered as f32 * mean / (2 * width * height) as f32;
    (share >= 0.15).then_some(mean)
}

/// Where `values` dip: a darker point at least `DEPTH` below the brightest each side of it,
/// up to where the brightness falls again.
fn dips(values: &[f32]) -> Vec<usize> {
    let mut found = Vec::new();
    let n = values.len();
    let mut at = 1;
    while at + 1 < n {
        let v = values[at];
        // The bottom of a dip: lower than the pixel before, no higher than the one after; a
        // flat bottom counts once, at its start.
        if !(v < values[at - 1] && v <= values[at + 1]) {
            at += 1;
            continue;
        }
        let mut left = at;
        while left > 0 && values[left - 1] >= values[left] {
            left -= 1;
        }
        let mut right = at;
        while right + 1 < n && values[right + 1] >= values[right] {
            right += 1;
        }
        if values[left].min(values[right]) - v >= DEPTH {
            found.push(at);
        }
        at += 1;
    }
    found
}

/// `picture` averaged over a `window`-pixel square around each pixel.
pub(crate) fn smooth(picture: &Picture, window: usize) -> Picture {
    let (width, height) = (picture.width, picture.height);
    let half = window / 2;
    let mut rows = vec![[0f32; 3]; width * height];
    for y in 0..height {
        let row = &picture.rgb[y * width..(y + 1) * width];
        box_line(row, &mut rows[y * width..(y + 1) * width], half, window);
    }
    let mut out = vec![[0f32; 3]; width * height];
    let mut column = vec![[0f32; 3]; height];
    let mut smoothed = vec![[0f32; 3]; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = rows[y * width + x];
        }
        box_line(&column, &mut smoothed, half, window);
        for y in 0..height {
            out[y * width + x] = smoothed[y];
        }
    }
    Picture {
        width,
        height,
        rgb: out,
    }
}

/// `line` averaged over `window` values, `half` before each and the rest after; the ends
/// average what lies within the line.
fn box_line(line: &[[f32; 3]], out: &mut [[f32; 3]], half: usize, window: usize) {
    let n = line.len();
    let mut prefix = vec![[0f32; 3]; n + 1];
    for (at, colour) in line.iter().enumerate() {
        for c in 0..3 {
            prefix[at + 1][c] = prefix[at][c] + colour[c];
        }
    }
    for (at, slot) in out.iter_mut().enumerate() {
        let low = at.saturating_sub(half);
        let high = (at + window - half).min(n);
        let count = (high - low) as f32;
        for c in 0..3 {
            slot[c] = (prefix[high][c] - prefix[low][c]) / count;
        }
    }
}
