//! One reading per sprite. A picture draws every piece of a kind alike: the squares that show the
//! same drawing (a sprite) are read together, as one piece, and the sprites are read together too,
//! within what a position allows (one king a side, no more than eight pawns and none on the first
//! or last rank, two of a minor piece or a rook and one queen but by promotion). A white piece and
//! a black piece of one kind share their outline, so a kind can be tried for every sprite of an
//! outline at once. A set no bundled set draws then reads by what most of its squares and the rules
//! say, not each square by its own nearest bundled drawing.
//!
//! The sides are told apart by the sprites' own tones in this picture, the lighter White's,
//! where the bundled sets do not clearly say otherwise.

use crate::squares::{Cell, Features, Silhouette};

/// Squares whose pieces are drawn this alike (`drawn_apart`) show one sprite: over what the
/// copies of one sprite differ by in a scaled or compressed picture (nine pairs in ten), under
/// what tells two pieces apart (all but one pair in a hundred).
const ALIKE: f32 = 0.1;
/// Sprites whose outlines differ this little (`outline_apart`) are one kind of piece: so near
/// that pieces of different kinds are almost never (under one pair in a thousand).
const ONE_KIND: f32 = 0.12;
/// What a piece on the side its tone does not show costs.
const OTHER_SIDE: f32 = 0.6;
/// How much nearer the bundled sets read a sprite as the other side's pieces than as its tone's
/// for them to keep it there: a set may shade a side's pieces unlike the rest of the side (the
/// letter set's black queen, drawn light on dark).
const CLEAR: f32 = 0.3;
/// What a side's count of kings other than one costs, for each king more or less.
const KINGS: f32 = 2.;
/// What each pawn of a side past eight costs.
const PAWNS: f32 = 1.;
/// What each knight, bishop or rook of a side past two costs: one only a promotion makes.
const MORE: f32 = 1.;
/// What each queen of a side past one costs.
const QUEENS: f32 = 0.5;
/// What a pawn costs on a square of a sprite with a square on the first or last rank.
const PAWN_HOME: f32 = 1.;
/// The least difference in tone (0 to 1) between the two sides' pieces for it to tell them.
const SIDES_APART: f32 = 0.12;

/// One square's piece as seen: its square, its silhouette and features, and how unlike each of
/// the 12 pieces (`piece_index`) it reads, `f32::MAX` for a piece no reference was drawn of.
pub(crate) struct Seen<'a> {
    pub cell: &'a Cell,
    pub shape: &'a Silhouette,
    pub features: &'a Features,
    pub best: [f32; 12],
    /// Whether the square is on the board's first or last rank, where no pawn stands.
    pub home: bool,
}

/// A sprite: the squares that show it, how unlike each piece it reads, and its tone.
struct Sprite {
    members: Vec<usize>,
    cost: [f32; 12],
    tone: f32,
}

/// The pieces of one picture read together: each square's `best` becomes its sprite's, the
/// piece the sprites and the rules choose for it first.
pub(crate) fn together(seen: &mut [Seen]) {
    let n = seen.len();
    if n == 0 {
        return;
    }
    let outline: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| outline_apart(seen[i].shape, seen[j].shape))
                .collect()
        })
        .collect();
    // One sprite is drawn alike and outlined as one kind: a square's own colour that differs
    // across it (a check's glow, a highlight's gradient) can pass for the share of the square a
    // piece lets through, never for its outline.
    let drawn: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let drawn =
                        drawn_apart((seen[i].cell, seen[i].shape), (seen[j].cell, seen[j].shape));
                    drawn.max(outline[i][j] * ALIKE / ONE_KIND)
                })
                .collect()
        })
        .collect();
    let mut sprites: Vec<Sprite> = cluster(&drawn, ALIKE)
        .into_iter()
        .map(|members| {
            let mut cost = [0f32; 12];
            for (slot, value) in cost.iter_mut().enumerate() {
                *value = if members.iter().all(|&i| seen[i].best[slot] < f32::MAX) {
                    members.iter().map(|&i| seen[i].best[slot]).sum::<f32>() / members.len() as f32
                } else {
                    f32::MAX
                };
            }
            let tone =
                members.iter().map(|&i| tone(seen[i].features)).sum::<f32>() / members.len() as f32;
            if members.iter().any(|&i| seen[i].home) {
                for slot in [0, 6] {
                    if cost[slot] < f32::MAX {
                        cost[slot] += PAWN_HOME;
                    }
                }
            }
            Sprite {
                members,
                cost,
                tone,
            }
        })
        .collect();
    if let Some(white) = sides(&sprites) {
        for (sprite, white) in sprites.iter_mut().zip(white) {
            let nearest = |side: std::ops::Range<usize>| {
                sprite.cost[side].iter().copied().fold(f32::MAX, f32::min)
            };
            let leaning = nearest(6..12) - nearest(0..6);
            if (if white { -leaning } else { leaning }) > CLEAR {
                continue;
            }
            let other = if white { 6..12 } else { 0..6 };
            for slot in other {
                if sprite.cost[slot] < f32::MAX {
                    sprite.cost[slot] += OTHER_SIDE;
                }
            }
        }
    }
    // How unlike each two sprites' outlines are, on average over their squares.
    let outlines: Vec<Vec<f32>> = sprites
        .iter()
        .map(|a| {
            sprites
                .iter()
                .map(|b| {
                    let mut sum = 0.;
                    for &i in &a.members {
                        for &j in &b.members {
                            sum += outline[i][j];
                        }
                    }
                    sum / (a.members.len() * b.members.len()) as f32
                })
                .collect()
        })
        .collect();
    let chosen = assign(&sprites, &outlines);
    for (sprite, piece) in sprites.iter().zip(chosen) {
        let mut cost = sprite.cost;
        // The chosen piece first: the nearest, or just nearer than the nearest where the other
        // sprites or the rules chose it, so the square is doubtful.
        let nearest = cost.iter().copied().fold(f32::MAX, f32::min);
        if cost[piece] > nearest {
            cost[piece] = nearest - 1e-4;
        }
        for &i in &sprite.members {
            seen[i].best = cost;
        }
    }
}

/// How light a piece is inside, 0 to 1.
fn tone(features: &Features) -> f32 {
    let covered: f32 = features.shape.iter().sum();
    let light: f32 = features
        .shape
        .iter()
        .zip(&features.brightness)
        .map(|(s, b)| s * b)
        .sum();
    light / covered.max(1e-6)
}

/// The squares grouped so that every two of a group are nearer than `within` (`apart`), the
/// nearest joined first.
fn cluster(apart: &[Vec<f32>], within: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = (0..apart.len()).map(|i| vec![i]).collect();
    loop {
        // The two groups whose furthest members are the nearest.
        let mut join: Option<(usize, usize, f32)> = None;
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                let furthest = groups[a]
                    .iter()
                    .flat_map(|&i| groups[b].iter().map(move |&j| apart[i][j]))
                    .fold(0f32, f32::max);
                if furthest < within && join.is_none_or(|(_, _, best)| furthest < best) {
                    join = Some((a, b, furthest));
                }
            }
        }
        let Some((a, b, _)) = join else {
            return groups;
        };
        let moved = groups.remove(b);
        groups[a].extend(moved);
    }
}

/// Which sprites are White's, by their tones: the split of the sprites into a lighter and a
/// darker side that sets them furthest apart, counting each square. `None` when the two sides'
/// tones are too close to tell.
fn sides(sprites: &[Sprite]) -> Option<Vec<bool>> {
    if sprites.len() < 2 {
        return None;
    }
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by(|&a, &b| sprites[a].tone.total_cmp(&sprites[b].tone));
    let weight = |k: usize| sprites[k].members.len() as f32;
    let mut best: Option<(usize, f32, f32)> = None;
    for split in 1..order.len() {
        let (dark, light) = order.split_at(split);
        let total = |side: &[usize]| side.iter().map(|&k| weight(k)).sum::<f32>();
        let mean = |side: &[usize]| {
            side.iter()
                .map(|&k| weight(k) * sprites[k].tone)
                .sum::<f32>()
                / total(side)
        };
        let gap = mean(light) - mean(dark);
        let between = total(dark) * total(light) * gap * gap;
        if best.is_none_or(|(_, score, _)| between > score) {
            best = Some((split, between, gap));
        }
    }
    let (split, _, gap) = best?;
    // The gap between the two sides' nearest sprites, not only their means.
    let nearest = sprites[order[split]].tone - sprites[order[split - 1]].tone;
    if gap < SIDES_APART * 2. || nearest < SIDES_APART {
        return None;
    }
    let mut white = vec![false; sprites.len()];
    for &k in &order[split..] {
        white[k] = true;
    }
    Some(white)
}

/// Each sprite's piece (`piece_index`): the choice that costs least, counting each square's
/// distance and what each side has more or fewer of than a position allows.
fn assign(sprites: &[Sprite], outlines: &[Vec<f32>]) -> Vec<usize> {
    let n = sprites.len();
    let total = |chosen: &[usize]| -> f32 {
        let mut sum = 0.;
        let mut kings = [0usize; 2];
        let mut counts = [[0usize; 6]; 2];
        for (k, sprite) in sprites.iter().enumerate() {
            let piece = chosen[k];
            let members = sprite.members.len();
            sum += sprite.cost[piece].min(1e6) * members as f32;
            if piece % 6 == 5 {
                kings[piece / 6] += members;
            }
            counts[piece / 6][piece % 6] += members;
        }
        for side in 0..2 {
            sum += KINGS * kings[side].abs_diff(1) as f32;
            let [pawns, knights, bishops, rooks, queens, _] = counts[side];
            sum += PAWNS * pawns.saturating_sub(8) as f32;
            sum += MORE
                * (knights.saturating_sub(2) + bishops.saturating_sub(2) + rooks.saturating_sub(2))
                    as f32;
            sum += QUEENS * queens.saturating_sub(1) as f32;
        }
        sum
    };
    let mut chosen: Vec<usize> = sprites
        .iter()
        .map(|sprite| {
            (0..12)
                .min_by(|&a, &b| sprite.cost[a].total_cmp(&sprite.cost[b]))
                .unwrap_or(0)
        })
        .collect();
    let mut cost = total(&chosen);
    let allowed = |k: usize, piece: usize| sprites[k].cost[piece] < f32::MAX;
    // Better choices one sprite at a time; then a kind for a sprite and every sprite of its
    // outline at once, each keeping its side; then two sprites at once (a king's sprite and the
    // one that should be), until none is better.
    loop {
        let mut improved = false;
        for k in 0..n {
            for piece in 0..12 {
                if !allowed(k, piece) || piece == chosen[k] {
                    continue;
                }
                let was = chosen[k];
                chosen[k] = piece;
                let now = total(&chosen);
                if now < cost - 1e-4 {
                    cost = now;
                    improved = true;
                } else {
                    chosen[k] = was;
                }
            }
        }
        if !improved {
            for near in outlines {
                let kind: Vec<usize> = (0..n).filter(|&l| near[l] < ONE_KIND).collect();
                for role in 0..6 {
                    let was = chosen.clone();
                    let mut fits = true;
                    for &l in &kind {
                        let piece = chosen[l] / 6 * 6 + role;
                        fits &= allowed(l, piece);
                        chosen[l] = piece;
                    }
                    let now = total(&chosen);
                    if fits && now < cost - 1e-4 {
                        cost = now;
                        improved = true;
                    } else {
                        chosen = was;
                    }
                }
            }
        }
        if !improved {
            for k in 0..n {
                for l in k + 1..n {
                    for pk in 0..12 {
                        for pl in 0..12 {
                            if !allowed(k, pk) || !allowed(l, pl) {
                                continue;
                            }
                            let (wk, wl) = (chosen[k], chosen[l]);
                            chosen[k] = pk;
                            chosen[l] = pl;
                            let now = total(&chosen);
                            if now < cost - 1e-4 {
                                cost = now;
                                improved = true;
                            } else {
                                chosen[k] = wk;
                                chosen[l] = wl;
                            }
                        }
                    }
                }
            }
        }
        if !improved {
            return chosen;
        }
    }
}

/// How unlike two squares' pieces are as drawn, 0 alike: one piece drawn over two square colours
/// differs between them only where it lets the square show through, and there by a share of the
/// difference between the two squares' colours. What is left of each pixel's difference past
/// that, over what either piece covers, as a part of how far the pieces stand out from their
/// squares there; at the nearest of a pixel's shifts either way, as a board's squares need not
/// fall on whole pixels.
fn drawn_apart(a: (&Cell, &Silhouette), b: (&Cell, &Silhouette)) -> f32 {
    let ((a, sa), (b, sb)) = (a, b);
    let size = sa.size;
    let between = [
        a.background[0] - b.background[0],
        a.background[1] - b.background[1],
        a.background[2] - b.background[2],
    ];
    let across = between.iter().map(|c| c * c).sum::<f32>();
    let mut nearest = f32::MAX;
    for (dx, dy) in SHIFTS {
        let (mut left, mut shown) = (0f32, 0f32);
        for y in 0..size {
            for x in 0..size {
                let Some(j) = shifted(size, x, y, dx, dy) else {
                    continue;
                };
                let i = y * size + x;
                if !sa.inside[i] && !sb.inside[j] {
                    continue;
                }
                let (pa, pb) = (a.rgb[i], b.rgb[j]);
                shown += crate::picture::distance(pa, a.background)
                    .max(crate::picture::distance(pb, b.background));
                let differ = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
                let through = if across > 1. {
                    (differ.iter().zip(&between).map(|(d, e)| d * e).sum::<f32>() / across)
                        .clamp(0., 1.)
                } else {
                    0.
                };
                left += differ
                    .iter()
                    .zip(&between)
                    .map(|(d, e)| (d - through * e).powi(2))
                    .sum::<f32>()
                    .sqrt();
            }
        }
        if shown > 0. {
            nearest = nearest.min(left / shown);
        }
    }
    nearest.min(1.)
}

/// How unlike two silhouettes' outlines are, whatever their colours: the part of what either
/// covers that the other does not, at the nearest of a pixel's shifts either way.
fn outline_apart(a: &Silhouette, b: &Silhouette) -> f32 {
    let size = a.size;
    let mut nearest = 1f32;
    for (dx, dy) in SHIFTS {
        let (mut both, mut either) = (0usize, 0usize);
        for y in 0..size {
            for x in 0..size {
                let i = y * size + x;
                let (ina, inb) = (
                    a.inside[i],
                    shifted(size, x, y, dx, dy).is_some_and(|j| b.inside[j]),
                );
                both += usize::from(ina && inb);
                either += usize::from(ina || inb);
            }
        }
        if either > 0 {
            nearest = nearest.min(1. - both as f32 / either as f32);
        }
    }
    nearest
}

/// No shift, and a pixel either way.
const SHIFTS: [(isize, isize); 5] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];

/// The index of pixel `(x, y)` shifted by `(dx, dy)`, if still on the square.
fn shifted(size: usize, x: usize, y: usize, dx: isize, dy: isize) -> Option<usize> {
    let (x, y) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
    (x < size && y < size).then_some(y * size + x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squares::{features, silhouette};

    const SIZE: usize = 48;
    const LIGHT: [f32; 3] = [240., 217., 181.];
    const DARK: [f32; 3] = [181., 136., 99.];

    /// A piece of `kind` (0 a pawn's round head on a base, 1 a tower, 2 a tall pointed one)
    /// filled with `fill` and outlined black, on a square of `square`.
    fn piece(kind: usize, fill: [f32; 3], square: [f32; 3]) -> Cell {
        let inside = |x: f32, y: f32| -> bool {
            match kind {
                0 => {
                    (x - 24.).hypot(y - 20.) < 8.
                        || ((14. ..34.).contains(&x) && (28. ..40.).contains(&y))
                }
                1 => (12. ..36.).contains(&x) && (8. ..40.).contains(&y),
                _ => (y - 6.) > 2.2 * (x - 24.).abs() && y < 42.,
            }
        };
        let rgb = (0..SIZE * SIZE)
            .map(|index| {
                let (x, y) = ((index % SIZE) as f32 + 0.5, (index / SIZE) as f32 + 0.5);
                let edge = [(-2., 0.), (2., 0.), (0., -2.), (0., 2.)]
                    .iter()
                    .any(|(dx, dy)| !inside(x + dx, y + dy));
                match (inside(x, y), edge) {
                    (true, true) => [20.; 3],
                    (true, false) => fill,
                    _ => square,
                }
            })
            .collect();
        Cell::new(SIZE, rgb, 0.)
    }

    fn shape(cell: &Cell) -> Silhouette {
        silhouette(SIZE, cell, &[]).unwrap()
    }

    /// A piece drawn on a light square and on a dark one is one sprite; the same piece in the
    /// other side's colour is another sprite of the same outline; another piece is unlike both.
    #[test]
    fn one_drawing_is_one_sprite_on_either_square() {
        let white = [250.; 3];
        let black = [40.; 3];
        let on_light = piece(0, white, LIGHT);
        let on_dark = piece(0, white, DARK);
        let black_pawn = piece(0, black, DARK);
        let tower = piece(1, white, DARK);
        let (a, b, c, d) = (
            shape(&on_light),
            shape(&on_dark),
            shape(&black_pawn),
            shape(&tower),
        );
        assert!(drawn_apart((&on_light, &a), (&on_dark, &b)) < ALIKE / 4.);
        assert!(outline_apart(&a, &b) < ONE_KIND / 4.);
        assert!(drawn_apart((&on_dark, &b), (&black_pawn, &c)) > 2. * ALIKE);
        assert!(outline_apart(&b, &c) < ONE_KIND / 4.);
        assert!(drawn_apart((&on_dark, &b), (&tower, &d)) > 2. * ALIKE);
        assert!(outline_apart(&b, &d) > 2. * ONE_KIND);
    }

    /// The pieces of one sprite read as most of them read; seven pieces of one sprite read
    /// nearer bishops than pawns are pawns, as a side has at most two bishops; a side has
    /// one king.
    #[test]
    fn a_sprite_reads_as_one_within_the_rules() {
        let white = [250.; 3];
        let pawns: Vec<Cell> = (0..7)
            .map(|k| piece(0, white, if k % 2 == 0 { LIGHT } else { DARK }))
            .collect();
        let towers: Vec<Cell> = (0..3)
            .map(|k| piece(1, white, if k % 2 == 0 { LIGHT } else { DARK }))
            .collect();
        let king = piece(2, white, LIGHT);
        let cells: Vec<&Cell> = pawns.iter().chain(&towers).chain([&king]).collect();
        let shapes: Vec<Silhouette> = cells.iter().map(|cell| shape(cell)).collect();
        let found: Vec<Features> = cells
            .iter()
            .zip(&shapes)
            .map(|(cell, shape)| features(shape, &cell.rgb))
            .collect();
        // How unlike each piece a square reads alone: `near` for `piece`, more for `second`.
        let reads = |piece: usize, near: f32, second: usize| {
            let mut best = [near + 0.5; 12];
            best[second] = near + 0.05;
            best[piece] = near;
            best
        };
        let mut readings = vec![reads(2, 0.3, 0); 7];
        readings.extend([reads(3, 0.2, 4), reads(4, 0.2, 3), reads(3, 0.2, 4)]);
        // Read alone, the king is a queen by a little.
        readings.push(reads(4, 0.3, 5));
        let mut seen: Vec<Seen> = (0..cells.len())
            .map(|k| Seen {
                cell: cells[k],
                shape: &shapes[k],
                features: &found[k],
                best: readings[k],
                home: false,
            })
            .collect();
        together(&mut seen);
        let chosen: Vec<usize> = seen
            .iter()
            .map(|seen| {
                (0..12)
                    .min_by(|&a, &b| seen.best[a].total_cmp(&seen.best[b]))
                    .unwrap()
            })
            .collect();
        assert_eq!(chosen, [0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 5]);
    }

    /// A sprite with a square on the first or last rank is no pawn, however near a pawn it
    /// reads.
    #[test]
    fn no_pawn_stands_on_the_first_or_last_rank() {
        let white = [250.; 3];
        let cells: Vec<Cell> = (0..3)
            .map(|k| piece(0, white, if k % 2 == 0 { LIGHT } else { DARK }))
            .collect();
        let shapes: Vec<Silhouette> = cells.iter().map(shape).collect();
        let found: Vec<Features> = cells
            .iter()
            .zip(&shapes)
            .map(|(cell, shape)| features(shape, &cell.rgb))
            .collect();
        let mut best = [0.8; 12];
        best[0] = 0.3;
        best[2] = 0.35;
        let read = |home: [bool; 3]| {
            let mut seen: Vec<Seen> = (0..3)
                .map(|k| Seen {
                    cell: &cells[k],
                    shape: &shapes[k],
                    features: &found[k],
                    best,
                    home: home[k],
                })
                .collect();
            together(&mut seen);
            seen.iter()
                .map(|seen| {
                    (0..12)
                        .min_by(|&a, &b| seen.best[a].total_cmp(&seen.best[b]))
                        .unwrap()
                })
                .collect::<Vec<usize>>()
        };
        assert_eq!(read([false; 3]), [0, 0, 0]);
        assert_eq!(read([false, true, false]), [2, 2, 2]);
    }

    /// The sides by tone: the lighter sprites White's, the darker Black's; sprites of one
    /// tone tell nothing.
    #[test]
    fn the_lighter_side_is_whites() {
        let sprites: Vec<Sprite> = [0.9, 0.85, 0.8, 0.2, 0.15]
            .into_iter()
            .enumerate()
            .map(|(k, tone)| Sprite {
                members: vec![k],
                cost: [0.; 12],
                tone,
            })
            .collect();
        assert_eq!(sides(&sprites), Some(vec![true, true, true, false, false]));
        assert_eq!(sides(&sprites[..3]), None);
    }
}
