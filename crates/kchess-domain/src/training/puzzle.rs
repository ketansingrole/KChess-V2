//! Puzzles (`crates/kchess-wasm/js/puzzle.ts`): turning Lichess puzzles from the API or the database
//! into one shape, and the move rules of solving them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use shakmaty::{Position as _, Role};

use super::{Out, text_arg, to_value, value_arg};
use crate::{js, position, rules};

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// `PuzzleState`: where a solve stands. `index` is the solution move the player must make next.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleState {
    pub fen: String,
    pub index: f64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<Vec<String>>,
    pub sans: Vec<String>,
}

/// `PlayerMove`: the state after the player's move, whether it was right, and the reply.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerMove {
    state: PuzzleState,
    correct: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<String>,
}

fn string_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn strings_of(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `/^\d+\.+$/`: a move number such as `12.` or `3...`.
fn is_move_number(token: &str) -> bool {
    let digits = token.trim_start_matches(|c: char| c.is_ascii_digit());
    digits.len() < token.len() && !digits.is_empty() && digits.bytes().all(|b| b == b'.')
}

/// The solution move at `index` when it is a non-empty one.
fn expected_at(solution: &[String], index: f64) -> Option<&str> {
    if index < 0.0 || index.fract() != 0.0 {
        return None;
    }
    solution
        .get(index as usize)
        .map(String::as_str)
        .filter(|uci| !uci.is_empty())
}

/// `puzzleFromApi`: a Lichess API puzzle. Current responses carry the starting `fen` and
/// `lastMove`; older ones only the game's moves, of which the first `initialPly + 1` lead to it.
fn from_api(raw: &Value) -> Option<Value> {
    let puzzle = raw.get("puzzle")?;
    let game = raw.get("game");
    let mut fen = string_of(puzzle.get("fen")).map(str::to_string);
    let mut last_move = string_of(puzzle.get("lastMove")).map(str::to_string);
    if fen.as_deref().is_none_or(str::is_empty) || last_move.as_deref().is_none_or(str::is_empty) {
        let start = rules::standard_from_fen(INITIAL_FEN)?;
        let pgn = game.and_then(|g| string_of(g.get("pgn"))).unwrap_or("");
        let sans: Vec<&str> = pgn
            .split(js::is_space)
            .filter(|token| !token.is_empty() && !is_move_number(token))
            .collect();
        // `initialPly` is read as JavaScript's `Number(…)`: an absent one is NaN, which slices nothing.
        let initial_ply = js::to_number(puzzle.get("initialPly"));
        let end = super::js_slice_end(sans.len(), initial_ply + 1.0);
        let wanted = &sans[..end];
        let (_, line) = position::line(start.clone(), wanted, true, false);
        if line.len() < wanted.len() {
            return None;
        }
        last_move = line.last().map(|step| step.uci.clone()).or(last_move);
        fen = Some(
            line.last()
                .map_or_else(|| rules::make_fen(&start), |step| step.fen.clone()),
        );
    }
    let solution = strings_of(puzzle.get("solution"));
    if solution.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert(
        "id".into(),
        puzzle.get("id").cloned().unwrap_or(Value::Null),
    );
    out.insert("fen".into(), json!(fen));
    if let Some(last) = last_move {
        out.insert("lastMove".into(), json!(last));
    }
    out.insert("solution".into(), json!(solution));
    out.insert(
        "rating".into(),
        puzzle.get("rating").cloned().unwrap_or(Value::Null),
    );
    out.insert("themes".into(), json!(strings_of(puzzle.get("themes"))));
    if let Some(plays) = puzzle.get("plays").filter(|v| !v.is_null()) {
        out.insert("plays".into(), plays.clone());
    }
    if let Some(game_id) = game.and_then(|g| g.get("id")).filter(|v| !v.is_null()) {
        out.insert("gameId".into(), game_id.clone());
    }
    Some(Value::Object(out))
}

/// `puzzleFromDb`: a row of Lichess's puzzle database, whose first move is the opponent's.
fn from_db(row: &Value) -> Option<Value> {
    let moves = string_of(row.get("moves"))?;
    let tokens: Vec<&str> = moves.split(' ').filter(|token| !token.is_empty()).collect();
    let (first, solution) = tokens.split_first()?;
    let pos = rules::standard_from_fen(string_of(row.get("fen"))?)?;
    let opening = position::play_uci(&pos, first)?;
    if solution.is_empty() {
        return None;
    }
    let themes: Vec<&str> = string_of(row.get("themes"))?
        .split(' ')
        .filter(|theme| !theme.is_empty())
        .collect();
    let mut out = Map::new();
    out.insert("id".into(), row.get("id").cloned().unwrap_or(Value::Null));
    out.insert("fen".into(), json!(opening.position.fen));
    out.insert("lastMove".into(), json!(first));
    out.insert("solution".into(), json!(solution));
    out.insert(
        "rating".into(),
        row.get("rating").cloned().unwrap_or(Value::Null),
    );
    out.insert("themes".into(), json!(themes));
    if let Some(plays) = row.get("plays").filter(|v| !v.is_null()) {
        out.insert("plays".into(), plays.clone());
    }
    Some(Value::Object(out))
}

/// The squares of a UCI move as chessops `parseUci` reads it (`uciSquares`).
fn uci_squares(uci: &str) -> Option<(&str, &str)> {
    let b = uci.as_bytes();
    let square_ok =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    if !(b.len() == 4 || b.len() == 5) || !square_ok(b[0], b[1]) || !square_ok(b[2], b[3]) {
        return None;
    }
    if b.len() == 5 && !b"pnbrqkPNBRQK".contains(&b[4]) {
        return None;
    }
    Some((&uci[0..2], &uci[2..4]))
}

/// `(file, rank)` of a square, both counted from zero.
fn coords(square: &str) -> (i32, i32) {
    let mut bytes = square.bytes();
    let file = bytes.next().unwrap_or(b'a');
    let rank = bytes.next().unwrap_or(b'1');
    (
        i32::from(file) - i32::from(b'a'),
        i32::from(rank) - i32::from(b'1'),
    )
}

/// `castlingSide`: `'h'` or `'a'` when a king's move from `from` to `to` castles, else None.
fn castling_side(fen: &str, from: &str, to: &str) -> Option<char> {
    let pos = rules::standard_from_fen(fen)?;
    let board = pos.board();
    let piece = |square: &str| rules::parse_square(square).and_then(|sq| board.piece_at(sq));
    let (to_file, to_rank) = coords(to);
    let (from_file, from_rank) = coords(from);
    let delta = (to_file + 8 * to_rank) - (from_file + 8 * from_rank);
    if delta.abs() != 2 && piece(to).map(|p| p.color) != Some(pos.turn()) {
        return None;
    }
    if piece(from).map(|p| p.role) != Some(Role::King) {
        return None;
    }
    Some(if delta > 0 { 'h' } else { 'a' })
}

/// `moveSquares`: the squares to highlight for a UCI move. Castling shows the king's two-square
/// step, not "king takes rook".
fn move_squares(fen: &str, uci: &str) -> Option<Vec<String>> {
    let (from, to) = uci_squares(uci)?;
    match castling_side(fen, from, to) {
        None => Some(vec![from.to_string(), to.to_string()]),
        Some(side) => {
            let file = if side == 'h' { 'g' } else { 'c' };
            Some(vec![from.to_string(), format!("{file}{}", &from[1..2])])
        }
    }
}

/// `startPuzzle`
fn start_puzzle(puzzle: &Value) -> PuzzleState {
    let fen = string_of(puzzle.get("fen")).unwrap_or("").to_string();
    let last_move = string_of(puzzle.get("lastMove"))
        .filter(|uci| !uci.is_empty())
        .and_then(|uci| move_squares(&fen, uci));
    PuzzleState {
        fen,
        index: 0.0,
        status: "playing".into(),
        last_move,
        sans: Vec::new(),
    }
}

/// `applyMove`: `uci` played from the state's position, or None when it is illegal.
fn apply_move(state: &PuzzleState, uci: &str) -> Option<PuzzleState> {
    let pos = rules::standard_from_fen(&state.fen)?;
    let played = position::play_uci(&pos, uci)?;
    let mut sans = state.sans.clone();
    sans.push(played.san);
    Some(PuzzleState {
        fen: played.position.fen,
        last_move: move_squares(&state.fen, uci),
        sans,
        ..state.clone()
    })
}

/// `playerMove`: the player moves. The solution's move is right, and so is any other move that
/// checkmates. The puzzle is solved once the last solution move, or a mate, is played.
fn player_move(solution: &[String], state: PuzzleState, uci: &str) -> PlayerMove {
    let wrong = |state| PlayerMove {
        state,
        correct: false,
        reply: None,
    };
    let Some(expected) = expected_at(solution, state.index) else {
        return wrong(state);
    };
    if state.status != "playing" {
        return wrong(state);
    }
    let Some(pos) = rules::standard_from_fen(&state.fen) else {
        return wrong(state);
    };
    let Some(played) = position::play_uci(&pos, uci) else {
        return wrong(state);
    };
    let mates = played.position.checkmate;
    let same_move = position::play_uci(&pos, expected).is_some_and(|other| other.uci == played.uci);
    if !same_move && !mates {
        return wrong(state);
    }
    let Some(next) = apply_move(&state, uci) else {
        return wrong(state);
    };
    let index = state.index + 1.0;
    if mates || index >= solution.len() as f64 {
        return PlayerMove {
            state: PuzzleState {
                index,
                status: "solved".into(),
                ..next
            },
            correct: true,
            reply: None,
        };
    }
    PlayerMove {
        reply: solution.get(index as usize).cloned(),
        state: PuzzleState { index, ..next },
        correct: true,
    }
}

/// `opponentReply`: the scripted reply. The state is unchanged when the move is illegal.
fn opponent_reply(state: PuzzleState, uci: &str) -> PuzzleState {
    match apply_move(&state, uci) {
        Some(next) => PuzzleState {
            index: state.index + 1.0,
            ..next
        },
        None => state,
    }
}

/// `playerColor`
fn player_color(puzzle: &Value) -> &'static str {
    if string_of(puzzle.get("fen")).and_then(|fen| fen.split(' ').nth(1)) == Some("b") {
        "black"
    } else {
        "white"
    }
}

fn player_move_call(args: &[Value]) -> Out {
    let puzzle: Value = value_arg(args, 0, "puzzle")?;
    let state: PuzzleState = value_arg(args, 1, "state")?;
    let uci = text_arg(args, 2, "uci")?;
    to_value(player_move(&strings_of(puzzle.get("solution")), state, uci))
}

fn next_solution_squares_call(args: &[Value]) -> Out {
    let puzzle: Value = value_arg(args, 0, "puzzle")?;
    let state: PuzzleState = value_arg(args, 1, "state")?;
    let solution = strings_of(puzzle.get("solution"));
    let squares = expected_at(&solution, state.index).and_then(|uci| move_squares(&state.fen, uci));
    to_value(squares)
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result =
        match method {
            "puzzleFromApi" => {
                value_arg::<Value>(args, 0, "raw").map(|raw| from_api(&raw).unwrap_or(Value::Null))
            }
            "puzzleFromDb" => {
                value_arg::<Value>(args, 0, "row").map(|row| from_db(&row).unwrap_or(Value::Null))
            }
            "moveSquares" => value_arg::<String>(args, 0, "fen").and_then(|fen| {
                let uci = text_arg(args, 1, "uci")?;
                to_value(move_squares(&fen, uci))
            }),
            "startPuzzle" => value_arg::<Value>(args, 0, "puzzle")
                .and_then(|puzzle| to_value(start_puzzle(&puzzle))),
            "playerMove" => player_move_call(args),
            "opponentReply" => value_arg::<PuzzleState>(args, 0, "state").and_then(|state| {
                let uci = text_arg(args, 1, "uci")?;
                to_value(opponent_reply(state, uci))
            }),
            "nextSolutionSquares" => next_solution_squares_call(args),
            "playerColor" => value_arg::<Value>(args, 0, "puzzle")
                .and_then(|puzzle| to_value(player_color(&puzzle))),
            _ => return None,
        };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_numbers_and_squares() {
        assert!(is_move_number("12."));
        assert!(is_move_number("3..."));
        assert!(!is_move_number("e4"));
        assert!(!is_move_number("."));
        assert_eq!(uci_squares("e1g1"), Some(("e1", "g1")));
        assert_eq!(uci_squares("e7e8q"), Some(("e7", "e8")));
        assert_eq!(uci_squares("e7e8k"), Some(("e7", "e8")));
        assert_eq!(uci_squares("e7e8x"), None);
    }

    #[test]
    fn castling_highlights_the_king_step() {
        let fen = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert_eq!(
            move_squares(fen, "e1h1"),
            Some(vec!["e1".to_string(), "g1".to_string()])
        );
        assert_eq!(
            move_squares(fen, "e1g1"),
            Some(vec!["e1".to_string(), "g1".to_string()])
        );
    }
}
