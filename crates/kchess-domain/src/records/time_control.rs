//! `crates/kchess-wasm/js/timeControl.ts`: Lichess Board API time-control rules.

use serde_json::{Value, json};

use super::{Out, arg};
use crate::js;

/// Lichess perf formula: estimated game length = initial time + 40 × increment.
fn total_seconds(minutes: Option<&Value>, increment: Option<&Value>) -> f64 {
    js::to_number(minutes) * 60.0 + 40.0 * js::to_number(increment)
}

/// `perfFor`: the Lichess perf of a time control.
fn perf_for(total: f64) -> &'static str {
    if total < 30.0 {
        "UltraBullet"
    } else if total < 180.0 {
        "Bullet"
    } else if total < 480.0 {
        "Blitz"
    } else if total < 1500.0 {
        "Rapid"
    } else {
        "Classical"
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let minutes = arg(args, 0);
    let increment = arg(args, 1);
    let total = || total_seconds(minutes, increment);
    Some(Ok(match method {
        "totalSeconds" => json!(total()),
        "perfFor" => json!(perf_for(total())),
        // Public matchmaking (`POST /api/board/seek`): Rapid and slower only.
        "canBoardSeek" => json!(total() >= 480.0),
        // Direct challenge (`POST /api/challenge/{username}`): Blitz and slower.
        "canDirectChallenge" => json!(total() >= 180.0),
        "canPlayOnline" => {
            let targeted = !js::falsy(arg(args, 2));
            json!(if targeted {
                total() >= 180.0
            } else {
                total() >= 480.0
            })
        }
        _ => return None,
    }))
}
