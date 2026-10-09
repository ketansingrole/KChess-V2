//! `core/src/domain/gameResult.ts` and `gameStatus.ts`: how a position ended a game, who won
//! on time, PGN result tokens, and whether a Lichess game is still being played.

use serde_json::{Map, Value, json};

use super::{Out, arg, setup_position};
use crate::js;
use crate::position;

/// Why a variant game ends on a win, by variant (`VARIANT_WIN`).
const VARIANT_WIN: &[(&str, &str)] = &[
    ("kingOfTheHill", "King reached the centre"),
    ("threeCheck", "Third check"),
    ("antichess", "Lost every piece"),
    ("atomic", "King exploded"),
    ("horde", "Horde captured"),
    ("racingKings", "King reached the eighth rank"),
];

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "opponent" => Ok(json!(opponent(arg(args, 0).and_then(Value::as_str)))),
        "boardResult" => board_result(args),
        "timeoutWinner" => timeout_winner(args),
        "pgnResult" => Ok(json!(pgn_result(
            arg(args, 0),
            arg(args, 1).and_then(Value::as_str)
        ))),
        "isGameInProgress" => Ok(json!(is_game_in_progress(arg(args, 0)))),
        "gameResult" => Ok(json!(game_result(arg(args, 0)))),
        _ => return None,
    })
}

/// `opponent(color)`: anything but white is black's opponent, as the TypeScript reads it.
fn opponent(color: Option<&str>) -> &'static str {
    if color == Some("white") {
        "black"
    } else {
        "white"
    }
}

/// `boardResult(pos, variant, draw?)`: how the position ended the game, or the draw already
/// found by repetition or the fifty-move rule; null while the game goes on.
fn board_result(args: &[Value]) -> Out {
    let pos = setup_position(arg(args, 0))?;
    let variant = arg(args, 1).and_then(Value::as_str).unwrap_or("");
    let facts = position::info(&pos);
    if let Some(outcome) = &facts.outcome {
        let reason = if facts.checkmate {
            "Checkmate"
        } else if facts.stalemate {
            "Stalemate"
        } else if facts.variant_end {
            VARIANT_WIN
                .iter()
                .find(|(key, _)| *key == variant)
                .map_or("Game over", |(_, reason)| reason)
        } else {
            "Insufficient material"
        };
        let mut result = Map::new();
        if let Some(winner) = outcome.get("winner") {
            result.insert("winner".into(), winner.clone());
        }
        result.insert("reason".into(), json!(reason));
        return Ok(Value::Object(result));
    }
    Ok(match arg(args, 2) {
        Some(draw) if !js::falsy(Some(draw)) => json!({ "reason": js::to_string(Some(draw)) }),
        _ => Value::Null,
    })
}

/// `timeoutWinner(pos, flagged)`: the other side, unless it cannot possibly mate.
fn timeout_winner(args: &[Value]) -> Out {
    let pos = setup_position(arg(args, 0))?;
    let winner = opponent(arg(args, 1).and_then(Value::as_str));
    let facts = position::info(&pos);
    let cannot_win = if winner == "white" {
        facts.cannot_win.white
    } else {
        facts.cannot_win.black
    };
    Ok(if cannot_win {
        Value::Null
    } else {
        json!(winner)
    })
}

/// `pgnResult(over, winner?)`: the PGN result token.
fn pgn_result(over: Option<&Value>, winner: Option<&str>) -> &'static str {
    if js::falsy(over) {
        return "*";
    }
    match winner {
        Some("white") => "1-0",
        Some("black") => "0-1",
        _ => "1/2-1/2",
    }
}

/// `isGameInProgress(status?)`: no status, or a live game (`created` is before its first move).
fn is_game_in_progress(status: Option<&Value>) -> bool {
    js::falsy(status) || matches!(status.and_then(Value::as_str), Some("started" | "created"))
}

/// `gameResult(game)`: the result from the account's side.
fn game_result(game: Option<&Value>) -> &'static str {
    let winner = game.and_then(|g| g.get("winner"));
    if js::falsy(winner) {
        return "draw";
    }
    if winner == game.and_then(|g| g.get("color")) {
        "win"
    } else {
        "loss"
    }
}
