//! What a review is stored under, and which games get one.

use serde::{Deserialize, Serialize};

use crate::variant::Variant;

/// Lichess speeds of standard chess; variants are not reviewed.
const STANDARD_PERFS: &[&str] = &crate::perf::SPEED_KEYS;

/// The key a review of `moves` (UCI) from `fen` is stored under. A game of a variant not played
/// by standard rules has a key that names it: the same moves are another game in Atomic.
pub fn review_key<S: AsRef<str>>(fen: &str, moves: &[S], variant: Option<Variant>) -> String {
    let moves: Vec<&str> = moves.iter().map(AsRef::as_ref).collect();
    match variant {
        Some(variant) if !variant.standard_rules() => {
            cyrb53(&format!("{}|{fen}", variant.key()), &moves.join(" "))
        }
        _ => cyrb53(fen, &moves.join(" ")),
    }
}

/// Whether a review is made for games of this Lichess perf: the standard speeds only.
pub fn is_reviewable_perf(perf: &str) -> bool {
    STANDARD_PERFS.contains(&perf)
}

/// cyrb53 of `fen|moves`, over UTF-16 code units, as 16 hex digits.
fn cyrb53(fen: &str, moves: &str) -> String {
    let text = format!("{fen}|{moves}");
    let mut h1 = 0xdeadbeefu32 as i32;
    let mut h2 = 0x41c6ce57u32 as i32;
    for unit in text.encode_utf16() {
        let code = i32::from(unit);
        h1 = (h1 ^ code).wrapping_mul(2_654_435_761u32 as i32);
        h2 = (h2 ^ code).wrapping_mul(1_597_334_677u32 as i32);
    }
    h1 = (h1 ^ shr(h1, 16)).wrapping_mul(2_246_822_507u32 as i32)
        ^ (h2 ^ shr(h2, 13)).wrapping_mul(3_266_489_909u32 as i32);
    h2 = (h2 ^ shr(h2, 16)).wrapping_mul(2_246_822_507u32 as i32)
        ^ (h1 ^ shr(h1, 13)).wrapping_mul(3_266_489_909u32 as i32);
    format!("{:08x}{:08x}", h2 as u32, h1 as u32)
}

/// JavaScript's `x >>> n` read back as the signed 32-bit value `^` works on.
fn shr(x: i32, n: u32) -> i32 {
    ((x as u32) >> n) as i32
}

/// A score: centipawns or mate in moves, from White's side.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Score {
    Cp(f64),
    Mate(f64),
}

/// The score of a finished game's last position: checkmate is lost for the side to move, a
/// variant won by the side to move (`won`, Antichess) is mate for it, the rest drawn.
pub fn end_eval(end: &str, turn: &str) -> Score {
    match end {
        "won" => Score::Mate(if turn == "white" { 1.0 } else { -1.0 }),
        "checkmate" => Score::Mate(0.0),
        _ => Score::Cp(0.0),
    }
}
