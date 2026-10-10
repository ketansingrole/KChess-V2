//! Endgame drills (`crates/kchess-wasm/js/endgames.ts`): the drill positions, and where a drill stands
//! after a line of moves.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::data::data;
use super::{Out, to_value, value_arg};
use crate::{position, rules};

/// One drill (`ENDGAME_DRILLS`); its data is shared with TypeScript.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndgameDrill {
    pub id: String,
    pub title: String,
    pub group: String,
    pub fen: String,
    pub player: String,
    pub goal: String,
    pub level: String,
    pub idea: String,
}

/// `EndgameStatus` with the turn and FEN the TypeScript result carries alongside it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub over: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<&'static str>,
    pub turn: &'static str,
    pub fen: String,
}

/// `repetitionKey`: the first four FEN fields.
fn repetition_key(fen: &str) -> String {
    fen.split(' ').take(4).collect::<Vec<_>>().join(" ")
}

/// `evaluateEndgame`: where the drill stands after `moves` (UCI, from `fen`), judged for
/// `player` and the drill's `goal`.
pub fn evaluate(fen: &str, moves: &[String], player: &str, goal: &str) -> Result<Status, String> {
    const INVALID: &str = "Invalid drill position.";
    let start = rules::standard_from_fen(fen).ok_or(INVALID)?;
    let mut seen: HashMap<String, u32> =
        HashMap::from([(repetition_key(&rules::make_fen(&start)), 1)]);
    let (after, line) = position::line(start, moves, false, false);
    let mut repeated = false;
    for step in &line {
        let key = repetition_key(&step.fen);
        let count = seen.get(&key).copied().unwrap_or(0) + 1;
        seen.insert(key, count);
        if count >= 3 {
            repeated = true;
        }
    }
    let pos = match line.last() {
        Some(last) => rules::standard_from_fen(&last.fen).ok_or(INVALID)?,
        None => after,
    };
    let info = position::info(&pos);
    let turn = info.turn;
    if info.checkmate {
        let won = turn != player;
        return Ok(Status {
            over: true,
            success: Some(won),
            title: if won {
                "Checkmate — well done!".into()
            } else {
                "You were checkmated".into()
            },
            detail: (!won).then_some("Try the position again."),
            turn,
            fen: info.fen,
        });
    }
    let drawn = if info.stalemate {
        Some("Stalemate")
    } else if info.insufficient {
        Some("Insufficient material")
    } else if repeated {
        Some("Threefold repetition")
    } else if info.halfmoves >= 100 {
        Some("Fifty-move rule")
    } else {
        None
    };
    if let Some(drawn) = drawn {
        let success = goal == "draw";
        let lower = drawn.to_lowercase();
        return Ok(Status {
            over: true,
            success: Some(success),
            title: if success {
                format!("Draw held — {lower}")
            } else {
                format!("Draw — {lower}")
            },
            detail: Some(if success {
                "That is the result to aim for here."
            } else {
                "This position is winning; try again."
            }),
            turn,
            fen: info.fen,
        });
    }
    Ok(Status {
        over: false,
        success: None,
        title: if info.check {
            "Check!".into()
        } else {
            format!(
                "{} to play",
                if turn == "white" { "White" } else { "Black" }
            )
        },
        detail: None,
        turn,
        fen: info.fen,
    })
}

/// `sanFrom`: SAN of a UCI move list played from `fen`; empty when the FEN is not a position.
pub fn san_from(fen: &str, moves: &[String]) -> Vec<String> {
    rules::standard_from_fen(fen)
        .map(|start| {
            position::line(start, moves, false, false)
                .1
                .into_iter()
                .map(|step| step.san)
                .collect()
        })
        .unwrap_or_default()
}

fn evaluate_call(args: &[Value]) -> Out {
    let fen: String = value_arg(args, 0, "fen")?;
    let moves: Vec<String> = value_arg(args, 1, "moves")?;
    let player: String = value_arg(args, 2, "player")?;
    let goal: String = value_arg(args, 3, "goal")?;
    to_value(evaluate(&fen, &moves, &player, &goal)?)
}

fn san_from_call(args: &[Value]) -> Out {
    let fen: String = value_arg(args, 0, "fen")?;
    let moves: Vec<String> = value_arg(args, 1, "moves")?;
    to_value(san_from(&fen, &moves))
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "endgameDrills" => data().and_then(|d| to_value(&d.endgame_drills)),
        "evaluateEndgame" => evaluate_call(args),
        "sanFrom" => san_from_call(args),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves(list: &[&str]) -> Vec<String> {
        list.iter().map(|m| (*m).to_string()).collect()
    }

    #[test]
    fn mate_stalemate_and_repetition() {
        let mated = evaluate(
            "7k/5Q2/6K1/8/8/8/8/8 w - - 0 1",
            &moves(&["f7g7"]),
            "white",
            "win",
        )
        .unwrap();
        assert_eq!((mated.over, mated.success), (true, Some(true)));
        let stale = evaluate(
            "7k/8/6K1/8/8/8/8/5Q2 w - - 0 1",
            &moves(&["f1f7"]),
            "white",
            "win",
        )
        .unwrap();
        assert_eq!((stale.over, stale.success), (true, Some(false)));
        let held = evaluate(
            "7k/8/6K1/8/8/8/8/5Q2 w - - 0 1",
            &moves(&["f1f7"]),
            "black",
            "draw",
        )
        .unwrap();
        assert_eq!((held.over, held.success), (true, Some(true)));
        let repeated = evaluate(
            "4k3/8/8/8/8/8/8/3QK3 w - - 0 1",
            &moves(&[
                "d1d2", "e8f8", "d2d1", "f8e8", "d1d2", "e8f8", "d2d1", "f8e8",
            ]),
            "white",
            "win",
        )
        .unwrap();
        assert!(repeated.title.contains("repetition"));
    }

    #[test]
    fn invalid_fen_is_an_error() {
        assert_eq!(
            evaluate("nonsense", &[], "white", "win").err().as_deref(),
            Some("Invalid drill position.")
        );
        assert!(san_from("nonsense", &[]).is_empty());
    }
}
