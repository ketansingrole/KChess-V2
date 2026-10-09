//! `core/src/domain/review.ts`: the review helpers that stayed in TypeScript. `replay` and
//! `analyseReview` are in `review.rs`.

use serde_json::{Value, json};

use super::{Out, arg, list, text_of};
use crate::js;

/// Lichess speeds of standard chess; variants are not reviewed.
const STANDARD_PERFS: &[&str] = &[
    "ultraBullet",
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "correspondence",
];

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "reviewKey" => {
            list(args, 1, "moves").map(|moves| json!(review_key(&text_of(args, 0), &join(moves))))
        }
        "hasScore" => Ok(json!(has_score(arg(args, 0)))),
        "endEval" => Ok(end_eval(
            arg(args, 0).and_then(Value::as_str) == Some("checkmate"),
        )),
        "isReviewablePerf" => Ok(json!(
            arg(args, 0)
                .and_then(Value::as_str)
                .is_some_and(|perf| STANDARD_PERFS.contains(&perf))
        )),
        _ => return None,
    })
}

/// The text `Array.prototype.join(' ')` makes of list items: null and undefined become ''.
pub(super) fn join(items: &[Value]) -> String {
    items
        .iter()
        .map(|item| {
            if item.is_null() {
                String::new()
            } else {
                js::to_string(Some(item))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `reviewKey`: cyrb53 of `fen|moves`, over UTF-16 code units, as 16 hex digits.
pub fn review_key(fen: &str, moves: &str) -> String {
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

/// `score.cp !== undefined || score.mate !== undefined`: a field present, even as null.
fn has_score(score: Option<&Value>) -> bool {
    score.is_some_and(|s| s.get("cp").is_some() || s.get("mate").is_some())
}

/// `endEval`: checkmate is lost for the side to move, the rest drawn.
fn end_eval(checkmate: bool) -> Value {
    if checkmate {
        json!({ "mate": 0 })
    } else {
        json!({ "cp": 0 })
    }
}
