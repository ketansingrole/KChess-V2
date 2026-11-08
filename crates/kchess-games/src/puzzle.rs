//! One puzzle on the board as every frontend plays it (`PuzzleState` in the rules,
//! `kchess-domain/src/training/puzzle.rs`): the position, the solution move to find next, and
//! whether it is solved. The app's Puzzles page and the CLI's `puzzle` both play puzzles through
//! it, so a move counts the same in each (any checkmate solves, as on Lichess).

use serde_json::{Value, json};

use crate::rules;

/// One puzzle on the board (`PuzzleState`): the position, the next solution move to find, and
/// whether it is still being played.
pub struct OnBoard {
    pub puzzle: Value,
    pub state: Value,
}

impl OnBoard {
    pub fn start(puzzle: &Value) -> OnBoard {
        OnBoard {
            puzzle: puzzle.clone(),
            state: rules("startPuzzle", std::slice::from_ref(puzzle)).unwrap_or(Value::Null),
        }
    }

    pub fn fen(&self) -> &str {
        self.state["fen"].as_str().unwrap_or_default()
    }

    /// The variant it is played in: a drill from a variant game keeps the game's.
    pub fn variant(&self) -> &str {
        self.puzzle["variant"].as_str().unwrap_or("standard")
    }

    /// `playing`, `solved` or `failed`.
    pub fn status(&self) -> &str {
        self.state["status"].as_str().unwrap_or("playing")
    }

    /// The index in the solution of the move the player must make next.
    pub fn index(&self) -> usize {
        crate::whole(&self.state["index"]) as usize
    }

    pub fn player_color(&self) -> shakmaty::Color {
        match rules("playerColor", std::slice::from_ref(&self.puzzle)) {
            Ok(color) if color == "black" => shakmaty::Color::Black,
            _ => shakmaty::Color::White,
        }
    }

    /// The last move's squares.
    pub fn last_move(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        let squares = self.state["lastMove"].as_array()?;
        Some((
            squares.first()?.as_str()?.parse().ok()?,
            squares.get(1)?.as_str()?.parse().ok()?,
        ))
    }

    /// The SAN of the last move played.
    pub fn last_san(&self) -> String {
        self.state["sans"]
            .as_array()
            .and_then(|sans| sans.last())
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The player's move: whether it was the solution's, and the opponent's reply to play next.
    pub fn player_move(&mut self, uci: &str) -> (bool, Option<String>) {
        let Ok(result) = rules(
            "playerMove",
            &[self.puzzle.clone(), self.state.clone(), json!(uci)],
        ) else {
            return (false, None);
        };
        let correct = result["correct"].as_bool() == Some(true);
        if correct {
            self.state = result["state"].clone();
        }
        (correct, result["reply"].as_str().map(str::to_string))
    }

    pub fn opponent_reply(&mut self, uci: &str) {
        if let Ok(state) = rules("opponentReply", &[self.state.clone(), json!(uci)]) {
            self.state = state;
        }
    }

    /// The next solution move's squares (the hint, and the solution's arrow).
    pub fn next_squares(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        let squares = rules(
            "nextSolutionSquares",
            &[self.puzzle.clone(), self.state.clone()],
        )
        .ok()?;
        let squares = squares.as_array()?;
        Some((
            squares.first()?.as_str()?.parse().ok()?,
            squares.get(1)?.as_str()?.parse().ok()?,
        ))
    }

    /// The solution's move at `index`.
    pub fn solution(&self, index: usize) -> Option<String> {
        self.puzzle["solution"][index].as_str().map(str::to_string)
    }

    /// Player moves in the solution.
    pub fn player_moves(&self) -> usize {
        self.puzzle["solution"]
            .as_array()
            .map_or(0, Vec::len)
            .div_ceil(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn puzzle() -> Value {
        // White mates with Qxf7# after Black's Nf6? (the opponent's move sets the puzzle).
        json!({
            "id": "test",
            "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            "solution": ["h5f7"],
            "rating": 600,
            "themes": ["mateIn1"],
            "plays": 1
        })
    }

    #[test]
    fn the_solution_is_checked_by_the_rules() {
        let mut board = OnBoard::start(&puzzle());
        assert_eq!(board.status(), "playing");
        assert_eq!(board.player_color(), shakmaty::Color::White);
        assert_eq!(
            board.next_squares().map(|(from, _)| from.to_string()),
            Some("h5".into())
        );
        assert!(!board.player_move("c4f7").0);
        assert_eq!(board.status(), "playing");
        assert!(board.player_move("h5f7").0);
        assert_eq!(board.status(), "solved");
        assert_eq!(board.last_san(), "Qxf7#");
    }

    /// The rules write the solution index as a JavaScript number (`2.0`): read as an integer it
    /// was 0, and Solution stopped after the opponent's first reply.
    #[test]
    fn the_solution_index_moves_on_after_a_reply() {
        let mut board = OnBoard::start(&json!({
            "id": "line",
            "fen": STANDARD_FEN,
            "solution": ["e2e4", "e7e5", "g1f3"],
            "rating": 1000,
            "themes": [],
        }));
        let (correct, reply) = board.player_move("e2e4");
        assert!(correct);
        assert_eq!(reply.as_deref(), Some("e7e5"));
        board.opponent_reply("e7e5");
        assert_eq!(board.index(), 2);
        assert_eq!(board.solution(board.index()).as_deref(), Some("g1f3"));
    }

    #[test]
    fn a_puzzle_shows_its_board_and_its_solution() {
        let mut board = OnBoard::start(&puzzle());
        assert_eq!(board.fen(), puzzle()["fen"].as_str().unwrap());
        assert_eq!(board.variant(), "standard");
        assert_eq!(board.index(), 0);
        assert_eq!(board.solution(0).as_deref(), Some("h5f7"));
        assert_eq!(board.solution(1), None);
        assert_eq!(board.player_moves(), 1);
        // A move that is no move at all leaves the puzzle as it was.
        assert_eq!(board.player_move("h5h9"), (false, None));
        assert_eq!(board.status(), "playing");
        assert!(board.player_move("h5f7").0);
        assert_eq!(
            board.last_move(),
            Some((shakmaty::Square::H5, shakmaty::Square::F7))
        );
        assert_ne!(board.fen(), puzzle()["fen"].as_str().unwrap());
    }

    #[test]
    fn black_plays_a_longer_line_in_the_puzzle_s_variant() {
        let line = json!({
            "id": "black",
            "fen": "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            "solution": ["e7e5", "g1f3", "b8c6"],
            "rating": 900,
            "themes": [],
        });
        let mut board = OnBoard::start(&line);
        assert_eq!(board.player_color(), shakmaty::Color::Black);
        assert_eq!(board.player_moves(), 2);
        assert_eq!(
            board.next_squares(),
            Some((shakmaty::Square::E7, shakmaty::Square::E5))
        );
        let (correct, reply) = board.player_move("e7e5");
        assert!(correct);
        assert_eq!(board.last_san(), "e5");
        board.opponent_reply(&reply.expect("White answers"));
        assert_eq!(board.last_san(), "Nf3");
        assert_eq!(board.index(), 2);
        assert_eq!(
            board.next_squares(),
            Some((shakmaty::Square::B8, shakmaty::Square::C6))
        );
        assert_eq!(board.status(), "playing");
        assert!(board.player_move("b8c6").0);
        assert_eq!(board.status(), "solved");

        let mut drill = line.clone();
        drill["variant"] = json!("chess960");
        assert_eq!(OnBoard::start(&drill).variant(), "chess960");
    }

    #[test]
    fn a_puzzle_the_rules_cannot_set_up_shows_nothing() {
        let board = OnBoard::start(&json!({ "id": "broken" }));
        assert_eq!(board.fen(), "");
        assert_eq!(board.status(), "playing");
        assert_eq!(board.index(), 0);
        assert_eq!(board.last_move(), None);
        assert_eq!(board.last_san(), "");
        assert_eq!(board.next_squares(), None);
        assert_eq!(board.player_moves(), 0);
    }
}
