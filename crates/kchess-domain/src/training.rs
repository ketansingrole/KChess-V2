//! Training: puzzle runs (Storm, Streak, Rush), puzzle solving, endgame drills, openings, the
//! client clock, engine levels, UCI `info` lines and Guess the move's scoring. Their constants
//! are data (`crates/kchess-domain/data/`).

pub mod clock;
pub mod data;
pub mod endgames;
pub mod engine_levels;
pub mod guess;
pub mod lessons;
pub mod openings;
pub mod puzzle;
pub mod repetition;
pub mod rush;
pub mod uci_info;

pub use clock::low_time_threshold;

use crate::js;

/// `String(n).padStart(2, '0')`
pub(crate) fn pad2(n: f64) -> String {
    let text = js::number_to_string(n);
    if text.len() < 2 {
        format!("0{text}")
    } else {
        text
    }
}

/// The end index of `array.slice(0, end)` over `len` items.
pub(crate) fn js_slice_end(len: usize, end: f64) -> usize {
    let end = if end.is_nan() { 0.0 } else { end.trunc() };
    let len = len as f64;
    if end < 0.0 {
        (len + end).max(0.0) as usize
    } else {
        end.min(len) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_rounding_and_slicing() {
        assert_eq!(js::round(2.5), 3.0);
        assert_eq!(js::round(-2.5), -2.0);
        assert_eq!(js::round(0.49999999999999994), 0.0);
        assert_eq!(js_slice_end(4, 2.7), 2);
        assert_eq!(js_slice_end(4, -1.0), 3);
        assert_eq!(js_slice_end(4, f64::NAN), 0);
        assert_eq!(js_slice_end(4, f64::INFINITY), 4);
        assert_eq!(js::number_to_string(f64::NAN), "NaN");
        assert_eq!(pad2(9.0), "09");
    }
}
