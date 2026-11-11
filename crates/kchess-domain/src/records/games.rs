//! How a position ended a game, who won
//! on time, PGN result tokens, whether a Lichess game is still being played, and how a game in
//! Game history ended for its player.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position};

use crate::js;
use crate::variant::Variant;

use crate::{position, replay, rules};

/// How a game ended: the winner, none for a draw, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameEnd {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::color::option_by_name"
    )]
    pub winner: Option<Color>,
    pub reason: String,
}

/// A finished game's result: a side won, or it was drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Wins(Color),
    Draw,
}

impl Outcome {
    /// The result of a game that is `over`, with `winner` (none for a draw).
    pub fn of(over: bool, winner: Option<Color>) -> Option<Outcome> {
        over.then(|| winner.map_or(Outcome::Draw, Outcome::Wins))
    }

    /// The PGN `Result` tag; `*` while the game goes on.
    pub fn tag(outcome: Option<Outcome>) -> &'static str {
        match outcome {
            None => "*",
            Some(Outcome::Wins(Color::White)) => "1-0",
            Some(Outcome::Wins(Color::Black)) => "0-1",
            Some(Outcome::Draw) => "1/2-1/2",
        }
    }

    /// A PGN `Result` tag's outcome; none for `*` or anything else.
    pub fn from_tag(tag: &str) -> Option<Outcome> {
        match tag {
            "1-0" => Some(Outcome::Wins(Color::White)),
            "0-1" => Some(Outcome::Wins(Color::Black)),
            "1/2-1/2" => Some(Outcome::Draw),
            _ => None,
        }
    }
}

/// How the position ended the game, or the draw already found by repetition or the fifty-move
/// rule (`draw`); none while the game goes on.
pub fn board_result(
    pos: &VariantPosition,
    variant: Variant,
    draw: Option<&str>,
) -> Option<GameEnd> {
    let facts = position::info(pos);
    if let Some(outcome) = &facts.outcome {
        let reason = if facts.checkmate {
            "Checkmate"
        } else if facts.stalemate {
            "Stalemate"
        } else if facts.variant_end {
            variant.win_reason().unwrap_or("Game over")
        } else {
            "Insufficient material"
        };
        return Some(GameEnd {
            winner: outcome
                .get("winner")
                .and_then(Value::as_str)
                .and_then(crate::color::from_name),
            reason: reason.to_string(),
        });
    }
    draw.map(|reason| GameEnd {
        winner: None,
        reason: reason.to_string(),
    })
}

/// Who wins when `flagged` runs out of time: the other side, unless it cannot possibly mate.
pub fn timeout_winner(pos: &VariantPosition, flagged: Color) -> Option<Color> {
    let winner = !flagged;
    (!pos.has_insufficient_material(winner)).then_some(winner)
}

fn ended(pos: &VariantPosition) -> bool {
    pos.is_variant_end() || pos.is_insufficient_material() || pos.legal_moves().is_empty()
}

/// Threefold repetition or the fifty-move rule over `moves` from `pos`, reading each move
/// trimmed when `trim`; the count stops at the first move that is not legal.
fn repetition(mut pos: VariantPosition, moves: &[&str]) -> Option<&'static str> {
    let mut seen = rules::Repetitions::starting_at(&rules::make_fen(&pos));
    for &uci in moves {
        let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
            break;
        };
        pos.play_unchecked(m);
        if seen.reached(&rules::make_fen(&pos)) {
            return Some("Threefold repetition");
        }
    }
    (!ended(&pos) && pos.halfmoves() >= 100).then_some("Fifty-move rule")
}

/// Repetition or the fifty-move rule reached by UCI `moves` from a setup, for the variants where
/// those rules apply.
pub fn setup_draw_reason(variant: Variant, fen: &str, moves: &[&str]) -> Option<&'static str> {
    let replayed = replay::replay_setup(variant, fen, moves)?;
    if !variant.counts_repetition() {
        return None;
    }
    let start = rules::setup_start(variant, fen)?;
    // Replayed moves are read as given, so a padded move ends the count there.
    let played: Vec<&str> = replayed.played.iter().map(|m| m.uci.as_str()).collect();
    repetition(start, &played)
}

/// Whether a Lichess game's `status` is live: none, or a game started (`created` is before its
/// first move).
pub fn is_game_in_progress(status: Option<&Value>) -> bool {
    js::falsy(status) || matches!(status.and_then(Value::as_str), Some("started" | "created"))
}

/// The side named “You” in a saved computer game; none for a game at the board.
pub fn archived_player_side(game: &Value) -> Option<&'static str> {
    let name = |side: &str| game.get(side).and_then(Value::as_str);
    if name("white") == Some("You") {
        Some("white")
    } else if name("black") == Some("You") {
        Some("black")
    } else {
        None
    }
}

/// A saved local game's outcome, won or lost for the side named “You”
/// (computer games) or the winning colour when both players were at the board; `unfinished`,
/// `ongoing` or `clock` when it has no result.
pub fn archived_outcome(game: &Value) -> &'static str {
    let field = |key: &str| game.get(key);
    if field("source").and_then(Value::as_str) == Some("clock") {
        return "clock";
    }
    match field("result").and_then(Value::as_str) {
        Some("*") if js::falsy(field("finished")) => "ongoing",
        Some("*") => "unfinished",
        Some("1/2-1/2") => "draw",
        result => {
            let winner = if result == Some("1-0") {
                "white"
            } else {
                "black"
            };
            match archived_player_side(game) {
                None => winner,
                Some(you) if you == winner => "win",
                Some(_) => "loss",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn outcome(game: Value) -> &'static str {
        archived_outcome(&game)
    }

    #[test]
    fn archived_games_end_for_their_player_or_by_colour() {
        let computer = |white: &str, black: &str, result: &str| json!({ "source": "computer", "white": white, "black": black, "result": result, "finished": true });
        assert_eq!(outcome(computer("You", "Stockfish", "1-0")), "win");
        assert_eq!(outcome(computer("You", "Stockfish", "0-1")), "loss");
        assert_eq!(outcome(computer("Stockfish", "You", "1-0")), "loss");
        assert_eq!(outcome(computer("Stockfish", "You", "0-1")), "win");
        assert_eq!(outcome(computer("You", "Stockfish", "1/2-1/2")), "draw");
        assert_eq!(outcome(computer("White", "Black", "0-1")), "black");
        assert_eq!(outcome(computer("You", "Stockfish", "*")), "unfinished");
        assert_eq!(
            outcome(json!({ "source": "board", "result": "*", "finished": false })),
            "ongoing"
        );
        assert_eq!(
            outcome(json!({ "source": "clock", "result": "1-0" })),
            "clock"
        );
        assert_eq!(
            archived_player_side(&computer("White", "You", "*")),
            Some("black")
        );
        assert_eq!(archived_player_side(&computer("White", "Black", "*")), None);
    }
}
