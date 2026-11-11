//! Puzzles: turning Lichess puzzles from the API or the database
//! into one shape, and the move rules of solving them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position as _, Role};

use crate::variant::Variant;
use crate::{js, position, rules};

/// How a solve stands: still being played, solved, failed, or given up with its solution shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PuzzleStatus {
    #[default]
    Playing,
    Solved,
    Failed,
    Revealed,
}

impl PuzzleStatus {
    /// Solved, failed or revealed: what is left is the next puzzle.
    pub fn is_over(self) -> bool {
        self != PuzzleStatus::Playing
    }
}

/// Where a solve stands. `index` is the solution move the player must make next.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleState {
    pub fen: String,
    pub index: f64,
    pub status: PuzzleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<Vec<String>>,
    pub sans: Vec<String>,
    /// A drill's variant (a mistake from a Crazyhouse game); every Lichess puzzle is standard
    /// chess.
    #[serde(default, skip_serializing_if = "Variant::is_standard")]
    pub variant: Variant,
}

/// The state's position, by its variant's rules.
fn position_of(state: &PuzzleState) -> Option<VariantPosition> {
    rules::setup_start(state.variant, &state.fen)
}

/// The state after the player's move, whether it was right, and the opponent's reply.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMove {
    pub state: PuzzleState,
    pub correct: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
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
fn expected_at<S: AsRef<str>>(solution: &[S], index: f64) -> Option<&str> {
    if index < 0.0 || index.fract() != 0.0 {
        return None;
    }
    solution
        .get(index as usize)
        .map(|uci| uci.as_ref())
        .filter(|uci| !uci.is_empty())
}

/// A Lichess API puzzle in KChess's shape. Current responses carry the starting `fen` and
/// `lastMove`; older ones only the game's moves, of which the first `initialPly + 1` lead to it.
pub fn from_api(raw: &Value) -> Option<Value> {
    let puzzle = raw.get("puzzle")?;
    let game = raw.get("game");
    let mut fen = js::string(puzzle.get("fen")).map(str::to_string);
    let mut last_move = js::string(puzzle.get("lastMove")).map(str::to_string);
    if fen.as_deref().is_none_or(str::is_empty) || last_move.as_deref().is_none_or(str::is_empty) {
        let start = rules::standard_from_fen(rules::INITIAL_FEN)?;
        let pgn = game.and_then(|g| js::string(g.get("pgn"))).unwrap_or("");
        let sans: Vec<&str> = pgn
            .split(js::is_space)
            .filter(|token| !token.is_empty() && !is_move_number(token))
            .collect();
        // An absent `initialPly` reads as NaN, which slices nothing.
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

/// The squares of a UCI move as UCI notation gives them.
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

/// `'h'` or `'a'` when a king's move from `from` to `to` castles, else None.
fn castling_side(pos: &VariantPosition, from: &str, to: &str) -> Option<char> {
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

/// The squares to highlight for a UCI move. Castling shows the king's two-square
/// step, not "king takes rook".
fn squares_for(pos: Option<&VariantPosition>, uci: &str) -> Option<Vec<String>> {
    let (from, to) = uci_squares(uci)?;
    match pos.and_then(|pos| castling_side(pos, from, to)) {
        None => Some(vec![from.to_string(), to.to_string()]),
        Some(side) => {
            let file = if side == 'h' { 'g' } else { 'c' };
            Some(vec![from.to_string(), format!("{file}{}", &from[1..2])])
        }
    }
}

/// A drill's move squares: a Crazyhouse drop (`N@f3`) marks the square it lands on.
fn drill_squares(
    state: &PuzzleState,
    pos: Option<&VariantPosition>,
    uci: &str,
) -> Option<Vec<String>> {
    match uci.split_once('@') {
        Some((_, to)) if !state.variant.is_standard() && rules::parse_square(to).is_some() => {
            Some(vec![to.to_string(), to.to_string()])
        }
        _ => squares_for(pos, uci),
    }
}

/// The squares to highlight for a UCI move from `fen`. An unreadable FEN still highlights them:
/// only castling detection needs the position.
pub fn move_squares(fen: &str, uci: &str) -> Option<Vec<String>> {
    let pos = rules::standard_from_fen(fen);
    squares_for(pos.as_ref(), uci)
}

/// A puzzle's state before the first move: its position, the squares of the opponent's move
/// that led there (UCI), and the variant it is played in.
pub fn start_puzzle(fen: &str, last_move: Option<&str>, variant: Variant) -> PuzzleState {
    PuzzleState {
        fen: fen.to_string(),
        index: 0.0,
        status: PuzzleStatus::Playing,
        last_move: last_move
            .filter(|uci| !uci.is_empty())
            .and_then(|uci| move_squares(fen, uci)),
        sans: Vec::new(),
        variant,
    }
}

/// `uci` played from the state's position, or None when it is illegal.
fn apply_move(state: &PuzzleState, uci: &str) -> Option<PuzzleState> {
    let pos = position_of(state)?;
    apply_move_at(&pos, state, uci)
}

/// The same, reusing the position the caller already parsed.
fn apply_move_at(pos: &VariantPosition, state: &PuzzleState, uci: &str) -> Option<PuzzleState> {
    let played = position::play_uci(pos, uci)?;
    let mut sans = state.sans.clone();
    sans.push(played.san);
    Some(PuzzleState {
        fen: played.position.fen,
        last_move: drill_squares(state, Some(pos), uci),
        sans,
        index: state.index,
        status: state.status,
        variant: state.variant,
    })
}

/// The player moves. The solution's move is right, and so is any other move that checkmates.
/// The puzzle is solved once the last solution move, or a mate, is played.
pub fn player_move(solution: &[String], state: PuzzleState, uci: &str) -> PlayerMove {
    let wrong = |state| PlayerMove {
        state,
        correct: false,
        reply: None,
    };
    let Some(expected) = expected_at(solution, state.index) else {
        return wrong(state);
    };
    // The player moves first, so theirs are the solution's even moves: at an odd one the
    // opponent's scripted reply is due (or could not be played), and typing it is no answer.
    if state.status != PuzzleStatus::Playing || state.index % 2.0 != 0.0 {
        return wrong(state);
    }
    let Some(pos) = position_of(&state) else {
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
    let Some(next) = apply_move_at(&pos, &state, uci) else {
        return wrong(state);
    };
    let index = state.index + 1.0;
    if mates || index >= solution.len() as f64 {
        return PlayerMove {
            state: PuzzleState {
                index,
                status: PuzzleStatus::Solved,
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

/// The opponent's scripted reply. The state is unchanged when the move is illegal.
pub fn opponent_reply(state: PuzzleState, uci: &str) -> PuzzleState {
    match apply_move(&state, uci) {
        Some(next) => PuzzleState {
            index: state.index + 1.0,
            ..next
        },
        None => state,
    }
}

/// The side the player solves for: the side to move in the puzzle's FEN.
pub fn player_color(fen: &str) -> Color {
    if fen.split(' ').nth(1) == Some("b") {
        Color::Black
    } else {
        Color::White
    }
}

/// The squares of the next solution move (the hint, and the solution's arrow).
pub fn next_solution_squares(solution: &[String], state: &PuzzleState) -> Option<Vec<String>> {
    expected_at(solution, state.index)
        .and_then(|uci| drill_squares(state, position_of(state).as_ref(), uci))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted reply that could not be played left the opponent to move, and the player
    /// typing the opponent's move was counted right and moved the puzzle on.
    #[test]
    fn the_opponents_move_is_no_answer() {
        let solution: Vec<String> = ["e2e4", "e7e5", "g1f3"].map(String::from).to_vec();
        let start = start_puzzle(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            None,
            Variant::Standard,
        );
        let first = player_move(&solution, start, "e2e4");
        assert!(first.correct);
        assert_eq!(first.reply.as_deref(), Some("e7e5"));
        let typed = player_move(&solution, first.state.clone(), "e7e5");
        assert!(!typed.correct);
        assert_eq!(typed.state, first.state);
    }

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
    fn a_puzzle_without_its_last_move_starts_from_its_games_moves() {
        let raw = json!({
            "puzzle": {
                "id": "p1",
                "fen": "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
                "initialPly": 1,
                "solution": ["g1f3"],
            },
            "game": { "pgn": "1. e4 e5 2. Nf3" },
        });
        let puzzle = from_api(&raw).unwrap();
        assert_eq!(puzzle["lastMove"], "e7e5");
        assert_eq!(
            puzzle["fen"],
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2"
        );
    }

    #[test]
    fn a_variant_drill_is_solved_by_its_rules() {
        // Crazyhouse, a pawn in each pocket: the drop is the answer, which standard rules refuse.
        let state = start_puzzle(
            "rnb1kbnr/ppp1pppp/8/q7/8/2N5/PPPP1PPP/R1BQKBNR[Pp] w KQkq - 2 4",
            None,
            Variant::Crazyhouse,
        );
        let squares = next_solution_squares(&["P@d4".to_string()], &state);
        assert_eq!(squares, Some(vec!["d4".to_string(), "d4".to_string()]));
        let played = player_move(&["P@d4".to_string()], state.clone(), "P@d4");
        assert!(played.correct);
        assert_eq!(played.state.status, PuzzleStatus::Solved);
        assert_eq!(played.state.sans, ["P@d4"]);
        assert_eq!(played.state.last_move, Some(vec!["d4".into(), "d4".into()]));
        let standard = PuzzleState {
            variant: Variant::Standard,
            ..state
        };
        assert!(!player_move(&["P@d4".to_string()], standard.clone(), "P@d4").correct);
        // A standard puzzle's state has no variant field, as before.
        assert!(json!(standard).get("variant").is_none());
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
