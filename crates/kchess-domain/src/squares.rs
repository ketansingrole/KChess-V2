//! Square names as players and stored documents write them (`e4`), read with JavaScript's
//! semantics where the rules corpus pins them.

use crate::js;

pub const FILES: [char; 8] = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];

/// Every square, file by file: a1, a2, … h8.
pub fn all_squares() -> Vec<String> {
    FILES
        .iter()
        .flat_map(|file| (1..=8).map(move |rank| format!("{file}{rank}")))
        .collect()
}

/// `Number(unit)` for one UTF-16 code unit of a square name (NaN when absent).
pub fn unit_number(unit: Option<&u16>) -> f64 {
    match unit {
        None => f64::NAN,
        Some(unit) => js::string_to_number(&String::from_utf16_lossy(&[*unit])),
    }
}

/// A square name's first UTF-16 unit minus `a`'s (so `a` is 0); None when empty.
pub fn file_offset(square: &str) -> Option<i64> {
    square
        .encode_utf16()
        .next()
        .map(|unit| i64::from(unit) - 97)
}
