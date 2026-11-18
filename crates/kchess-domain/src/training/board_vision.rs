//! The board-vision drills: find a square by its name, tell its colour, and walk a knight from
//! one square to another.

use serde::Serialize;
use std::collections::HashSet;

use crate::js;
use crate::squares::{FILES, all_squares, file_offset, unit_number};

/// The index into the 64 squares of a draw in [0, 1), as `Math.floor(random() * 64)`.
fn square_index(draw: f64) -> Option<usize> {
    let index = (draw * 64.0).floor();
    (0.0..64.0).contains(&index).then_some(index as usize)
}

/// The first draw whose square is not `previous`; none when the draws run out. With `allowed`, draws pick among those squares only (in the given order); a
/// single allowed square is returned even when it is `previous`, since there is no other.
pub fn random_square(
    previous: Option<&str>,
    draws: &[f64],
    allowed: Option<&[String]>,
) -> Option<String> {
    let squares = match allowed {
        Some(allowed) if !allowed.is_empty() => allowed.to_vec(),
        _ => all_squares(),
    };
    if squares.len() == 1 {
        return squares.into_iter().next();
    }
    for draw in draws {
        let index = (draw * squares.len() as f64).floor();
        if !(0.0..squares.len() as f64).contains(&index) {
            continue;
        }
        let square = &squares[index as usize];
        if Some(square.as_str()) != previous {
            return Some(square.clone());
        }
    }
    None
}

/// Whether a square is light or dark.
pub fn square_color(square: &str) -> &'static str {
    let (file, rank) = knight_index(square);
    if (file + rank) % 2.0 == 0.0 {
        "dark"
    } else {
        "light"
    }
}

/* ── Knight paths ── */

const JUMPS: [(f64, f64); 8] = [
    (1.0, 2.0),
    (2.0, 1.0),
    (2.0, -1.0),
    (1.0, -2.0),
    (-1.0, -2.0),
    (-2.0, -1.0),
    (-2.0, 1.0),
    (-1.0, 2.0),
];

/// The file and rank offsets of a square (NaN for anything not a square).
fn knight_index(square: &str) -> (f64, f64) {
    let units: Vec<u16> = square.encode_utf16().collect();
    let file = file_offset(square).map_or(f64::NAN, |f| f as f64);
    (file, unit_number(units.get(1)) - 1.0)
}

/// The squares a knight on `square` reaches in one jump.
pub fn knight_moves(square: &str) -> Vec<String> {
    let (file, rank) = knight_index(square);
    JUMPS
        .iter()
        .filter_map(|(df, dr)| {
            let f = file + df;
            let r = rank + dr;
            ((0.0..8.0).contains(&f) && (0.0..8.0).contains(&r))
                .then(|| format!("{}{}", FILES[f as usize], r as usize + 1))
        })
        .collect()
}

/// Fewest jumps from `from` to `to`; None when no path exists.
pub fn knight_distance(from: &str, to: &str) -> Option<u32> {
    if from == to {
        return Some(0);
    }
    let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
    let mut frontier = vec![from.to_string()];
    let mut steps = 1;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for square in &frontier {
            for target in knight_moves(square) {
                if target == to {
                    return Some(steps);
                }
                if seen.insert(target.clone()) {
                    next.push(target);
                }
            }
        }
        frontier = next;
        steps += 1;
    }
    None
}

/// A knight's journey: from one square to another, `best` jumps apart (none when unreachable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KnightChallenge {
    pub from: String,
    pub to: String,
    pub best: Option<u32>,
}

/// The first pair of draws whose squares are `min`–`max` jumps apart; none when the draws run
/// out.
pub fn knight_challenge(min: f64, max: f64, draws: &[f64]) -> Option<KnightChallenge> {
    let squares = all_squares();
    for pair in draws.as_chunks::<2>().0 {
        let (Some(a), Some(b)) = (square_index(pair[0]), square_index(pair[1])) else {
            continue;
        };
        let (from, to) = (&squares[a], &squares[b]);
        let best = knight_distance(from, to);
        let best_value = best.map_or(f64::INFINITY, f64::from);
        if best_value >= min && best_value <= max {
            return Some(KnightChallenge {
                from: from.clone(),
                to: to.clone(),
                best,
            });
        }
    }
    None
}

/// A number's text; zero and NaN print as nothing.
fn js_or_empty(n: f64) -> String {
    if n == 0.0 || n.is_nan() {
        String::new()
    } else {
        js::number_to_string(n)
    }
}

/// A board with one white knight, as a FEN (the board only draws it).
pub fn knight_fen(square: &str) -> String {
    let (file, rank) = knight_index(square);
    let rows: Vec<String> = (0..8)
        .map(|r| {
            let row = 7 - r;
            if f64::from(row) == rank {
                format!("{}N{}", js_or_empty(file), js_or_empty(7.0 - file))
            } else {
                "8".into()
            }
        })
        .collect();
    format!("{} w - - 0 1", rows.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_square_falls_back_to_the_board_when_none_are_allowed() {
        assert_eq!(random_square(None, &[0.5], Some(&[])), Some("e1".into()));
        let only = ["c3".to_string(), "d4".to_string()];
        assert_eq!(random_square(None, &[0.9], Some(&only)), Some("d4".into()));
    }

    #[test]
    fn squares_alternate_dark_and_light() {
        assert_eq!(square_color("a1"), "dark");
        assert_eq!(square_color("b1"), "light");
        assert_eq!(square_color("a2"), "light");
        assert_eq!(square_color("h8"), "dark");
    }
}
