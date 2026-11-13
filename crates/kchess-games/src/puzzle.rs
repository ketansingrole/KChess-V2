//! One puzzle on the board as every frontend plays it (`kchess_domain::training::puzzle`): the
//! position, the solution move to find next, and
//! whether it is solved. The app's Puzzles page and the CLI's `puzzle` both play puzzles through
//! it, so a move counts the same in each (any checkmate solves, as on Lichess).

use kchess_contracts::types::Puzzle;
use kchess_domain::training::puzzle::{self, PuzzleState, PuzzleStatus};

/// One puzzle on the board: the position, the next solution move to find, and whether it is
/// still being played.
pub struct OnBoard {
    pub puzzle: Puzzle,
    pub state: PuzzleState,
}

impl OnBoard {
    pub fn start(puzzle: Puzzle) -> OnBoard {
        let state = puzzle::start_puzzle(&puzzle.fen, puzzle.last_move.as_deref(), puzzle.variant);
        OnBoard { puzzle, state }
    }

    pub fn fen(&self) -> &str {
        &self.state.fen
    }

    /// The variant it is played in: a drill from a variant game keeps the game's.
    pub fn variant(&self) -> kchess_domain::variant::Variant {
        self.state.variant
    }

    pub fn status(&self) -> PuzzleStatus {
        self.state.status
    }

    /// Whether the solution is still being looked for.
    pub fn playing(&self) -> bool {
        self.status() == PuzzleStatus::Playing
    }

    pub fn solved(&self) -> bool {
        self.status() == PuzzleStatus::Solved
    }

    /// The index in the solution of the move the player must make next.
    pub fn index(&self) -> usize {
        self.state.index.max(0.) as usize
    }

    pub fn player_color(&self) -> shakmaty::Color {
        puzzle::player_color(&self.puzzle.fen)
    }

    /// The last move's squares.
    pub fn last_move(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        squares(self.state.last_move.as_deref()?)
    }

    /// The SAN of the last move played.
    pub fn last_san(&self) -> String {
        self.state.sans.last().cloned().unwrap_or_default()
    }

    /// The player's move: whether it was the solution's, and the opponent's reply to play next.
    pub fn player_move(&mut self, uci: &str) -> (bool, Option<String>) {
        let played = puzzle::player_move(&self.puzzle.solution, self.state.clone(), uci);
        if played.correct {
            self.state = played.state;
        }
        (played.correct, played.reply)
    }

    pub fn opponent_reply(&mut self, uci: &str) {
        self.state = puzzle::opponent_reply(self.state.clone(), uci);
    }

    /// The next solution move's squares (the hint, and the solution's arrow).
    pub fn next_squares(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        squares(&puzzle::next_solution_squares(
            &self.puzzle.solution,
            &self.state,
        )?)
    }

    /// The solution's move at `index`.
    pub fn solution(&self, index: usize) -> Option<String> {
        self.puzzle.solution.get(index).cloned()
    }

    /// Play the solution's next move for the player, as Solution shows it: the opponent's reply
    /// to play next, if any; none when nothing was played (the puzzle is over, or its solution
    /// has no further move the rules accept).
    pub fn reveal_next(&mut self) -> Option<Option<String>> {
        if !self.playing() {
            return None;
        }
        let uci = self.solution(self.index())?;
        let (correct, reply) = self.player_move(&uci);
        correct.then_some(reply)
    }

    /// Player moves in the solution.
    pub fn player_moves(&self) -> usize {
        self.puzzle.solution.len().div_ceil(2)
    }
}

fn squares(names: &[String]) -> Option<(shakmaty::Square, shakmaty::Square)> {
    Some((names.first()?.parse().ok()?, names.get(1)?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_domain::variant::Variant;

    const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn line(fen: &str, solution: &[&str]) -> Puzzle {
        Puzzle {
            id: "line".into(),
            fen: fen.into(),
            last_move: None,
            solution: solution.iter().map(|uci| uci.to_string()).collect(),
            rating: 1000.,
            themes: Vec::new(),
            plays: None,
            game_id: None,
            variant: Variant::Standard,
        }
    }

    fn puzzle() -> Puzzle {
        // White mates with Qxf7# after Black's Nf6? (the opponent's move sets the puzzle).
        line(
            "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            &["h5f7"],
        )
    }

    #[test]
    fn the_solution_is_checked_by_the_rules() {
        let mut board = OnBoard::start(puzzle());
        assert_eq!(board.status(), PuzzleStatus::Playing);
        assert_eq!(board.player_color(), shakmaty::Color::White);
        assert_eq!(
            board.next_squares().map(|(from, _)| from.to_string()),
            Some("h5".into())
        );
        assert!(!board.player_move("c4f7").0);
        assert_eq!(board.status(), PuzzleStatus::Playing);
        assert!(!board.solved());
        assert!(board.player_move("h5f7").0);
        assert_eq!(board.status(), PuzzleStatus::Solved);
        assert_eq!(board.last_san(), "Qxf7#");
    }

    /// The rules write the solution index as a float (`2.0`): read as an integer it was 0, and
    /// Solution stopped after the opponent's first reply.
    #[test]
    fn the_solution_is_revealed_a_move_at_a_time() {
        let mut board = OnBoard::start(line(STANDARD_FEN, &["e2e4", "e7e5", "g1f3"]));
        assert!(board.playing());
        assert_eq!(board.reveal_next(), Some(Some("e7e5".into())));
        assert_eq!(board.last_san(), "e4");
        board.opponent_reply("e7e5");
        assert_eq!(board.reveal_next(), Some(None));
        assert_eq!(board.last_san(), "Nf3");
        assert!(board.solved() && !board.playing());
        assert_eq!(board.reveal_next(), None);
    }

    #[test]
    fn the_solution_index_moves_on_after_a_reply() {
        let mut board = OnBoard::start(line(STANDARD_FEN, &["e2e4", "e7e5", "g1f3"]));
        let (correct, reply) = board.player_move("e2e4");
        assert!(correct);
        assert_eq!(reply.as_deref(), Some("e7e5"));
        board.opponent_reply("e7e5");
        assert_eq!(board.index(), 2);
        assert_eq!(board.solution(board.index()).as_deref(), Some("g1f3"));
    }

    #[test]
    fn a_puzzle_shows_its_board_and_its_solution() {
        let mut board = OnBoard::start(puzzle());
        assert_eq!(board.fen(), puzzle().fen);
        assert_eq!(board.variant(), Variant::Standard);
        assert_eq!(board.index(), 0);
        assert_eq!(board.solution(0).as_deref(), Some("h5f7"));
        assert_eq!(board.solution(1), None);
        assert_eq!(board.player_moves(), 1);
        // A move that is no move at all leaves the puzzle as it was.
        assert_eq!(board.player_move("h5h9"), (false, None));
        assert_eq!(board.status(), PuzzleStatus::Playing);
        assert!(board.player_move("h5f7").0);
        assert_eq!(
            board.last_move(),
            Some((shakmaty::Square::H5, shakmaty::Square::F7))
        );
        assert_ne!(board.fen(), puzzle().fen);
    }

    #[test]
    fn black_plays_a_longer_line_in_the_puzzle_s_variant() {
        let black = line(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            &["e7e5", "g1f3", "b8c6"],
        );
        let mut board = OnBoard::start(black.clone());
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
        assert_eq!(board.status(), PuzzleStatus::Playing);
        assert!(board.player_move("b8c6").0);
        assert_eq!(board.status(), PuzzleStatus::Solved);

        let drill = Puzzle {
            variant: Variant::Chess960,
            ..black
        };
        assert_eq!(OnBoard::start(drill).variant(), Variant::Chess960);
    }

    #[test]
    fn a_puzzle_the_rules_cannot_set_up_shows_nothing() {
        let board = OnBoard::start(line("", &[]));
        assert_eq!(board.fen(), "");
        assert_eq!(board.status(), PuzzleStatus::Playing);
        assert_eq!(board.index(), 0);
        assert_eq!(board.last_move(), None);
        assert_eq!(board.last_san(), "");
        assert_eq!(board.next_squares(), None);
        assert_eq!(board.player_moves(), 0);
    }
}
