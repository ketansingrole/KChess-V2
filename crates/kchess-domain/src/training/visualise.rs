//! Visualise: a few moves of a real game read as text, not played on the board, then one
//! question on the position they lead to. The round's moves come from a game and its question
//! and answer from the rules, so every question has one answer.

use shakmaty::san::SanPlus;
use shakmaty::{Chess, Color, Move, Piece, Position as _, Role, Square};

/// The plies each level reads ahead, Level 1 first.
pub const LEVEL_PLIES: [usize; 5] = [2, 3, 4, 6, 8];

/// What a round asks about the position after its moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    /// What stands on a square, if anything.
    On(Square),
    /// Where the one piece of its colour and kind stands: asked only while there is one.
    Where(Piece),
    /// Whether a side attacks a square that is empty or the other side's.
    Attacks(Color, Square),
}

/// An answer to a question: what stands on a square, a square, or yes or no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Piece(Option<Piece>),
    Square(Square),
    Yes(bool),
}

const ROLES: [Role; 6] = [
    Role::Pawn,
    Role::Knight,
    Role::Bishop,
    Role::Rook,
    Role::Queen,
    Role::King,
];

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// What stands on a square, as a choice names it: "White knight", or "Empty".
pub fn piece_name(piece: Option<Piece>) -> String {
    match piece {
        None => "Empty".to_string(),
        Some(piece) => format!("{} {}", color_name(piece.color), role_name(piece.role)),
    }
}

impl Question {
    /// The question as it is asked.
    pub fn prompt(self) -> String {
        match self {
            Question::On(square) => format!("What is on {square}?"),
            Question::Where(piece) => format!(
                "Where is the {} {}?",
                color_name(piece.color).to_lowercase(),
                role_name(piece.role)
            ),
            Question::Attacks(color, square) => {
                format!("Does {} attack {square}?", color_name(color))
            }
        }
    }

    /// The rules' answer in `position`.
    pub fn answer(self, position: &Chess) -> Answer {
        let board = position.board();
        match self {
            Question::On(square) => Answer::Piece(board.piece_at(square)),
            Question::Where(piece) => Answer::Square(
                board
                    .by_piece(piece)
                    .first()
                    // Never asked without the piece (`ask`); a1 only stands in.
                    .unwrap_or(Square::A1),
            ),
            Question::Attacks(color, square) => {
                Answer::Yes(board.attacks_to(square, color, board.occupied()).any())
            }
        }
    }
}

impl Answer {
    /// The answer as a choice or a verdict names it: "White knight", "e4", "Yes".
    pub fn label(self) -> String {
        match self {
            Answer::Piece(piece) => piece_name(piece),
            Answer::Square(square) => square.to_string(),
            Answer::Yes(true) => "Yes".to_string(),
            Answer::Yes(false) => "No".to_string(),
        }
    }
}

/// One round: a game's position, its next moves and the question on where they lead.
#[derive(Clone, Debug)]
pub struct Round {
    pub start: Chess,
    pub moves: Vec<Move>,
    /// Each move in SAN, from the position before it.
    pub sans: Vec<String>,
    pub question: Question,
    pub answer: Answer,
    /// The answers to choose from, in the order offered, the right one among them; empty when the
    /// answer is a square, which is found on the board.
    pub choices: Vec<Answer>,
}

impl Round {
    /// The position after the first `ply` moves.
    pub fn position_after(&self, ply: usize) -> Chess {
        let mut position = self.start.clone();
        for m in self.moves.iter().take(ply) {
            position.play_unchecked(*m);
        }
        position
    }

    /// The position after every move: the one the question is about.
    pub fn end(&self) -> Chess {
        self.position_after(self.moves.len())
    }

    pub fn is_right(&self, answer: Answer) -> bool {
        answer == self.answer
    }

    /// The moves with their numbers, as a scoresheet writes them: each White move and a first
    /// Black move carry one ("12." or "12…").
    pub fn numbered(&self) -> Vec<(Option<String>, &str)> {
        crate::tree::numbered(self.plies())
    }

    /// The moves as one line of text ("12… Nc6 13. Bb5 a6").
    pub fn moves_text(&self) -> String {
        crate::tree::numbered_line(self.plies())
    }

    /// Each move with the half-moves before it.
    fn plies(&self) -> impl Iterator<Item = (u32, &str)> {
        let first = crate::rules::ply_of(&self.start);
        (first..).zip(self.sans.iter().map(String::as_str))
    }
}

/// An index under `len` from a draw in [0, 1).
fn pick(draw: &mut dyn FnMut() -> f64, len: usize) -> usize {
    ((draw() * len as f64).floor().max(0.) as usize).min(len.saturating_sub(1))
}

/// A round of `plies` moves from one of `games` (each its moves from the standard starting
/// position), at a place the draws pick; none when no game is that long, or no question fits
/// the places tried.
pub fn round(games: &[Vec<Move>], plies: usize, draw: &mut dyn FnMut() -> f64) -> Option<Round> {
    let starts: usize = games
        .iter()
        .map(|game| (game.len() + 1).saturating_sub(plies))
        .sum();
    if plies == 0 || starts == 0 {
        return None;
    }
    for _ in 0..16 {
        let mut index = pick(draw, starts);
        let Some((game, at)) = games.iter().find_map(|game| {
            let here = (game.len() + 1).saturating_sub(plies);
            if index < here {
                Some((game, index))
            } else {
                index -= here;
                None
            }
        }) else {
            continue;
        };
        let mut start = Chess::default();
        for m in &game[..at] {
            start.play_unchecked(*m);
        }
        let moves = game[at..at + plies].to_vec();
        if let Some(round) = ask(start, moves, draw) {
            return Some(round);
        }
    }
    None
}

/// The squares the moves touch, each once in the order first touched: where each piece left and
/// arrived, both of a castling, and the pawn taken en passant.
fn touched(moves: &[Move], start: &Chess) -> Vec<Square> {
    let mut squares = Vec::new();
    let mut position = start.clone();
    for m in moves {
        let turn = position.turn();
        let mut add = |square: Square| {
            if !squares.contains(&square) {
                squares.push(square);
            }
        };
        match *m {
            Move::Castle { king, rook } => {
                let side = shakmaty::CastlingSide::from_queen_side(rook < king);
                add(king);
                add(rook);
                add(side.king_to(turn));
                add(side.rook_to(turn));
            }
            Move::EnPassant { from, to } => {
                add(from);
                add(to);
                add(Square::from_coords(to.file(), from.rank()));
            }
            _ => {
                if let Some(from) = m.from() {
                    add(from);
                }
                add(m.to());
            }
        }
        position.play_unchecked(*m);
    }
    squares
}

/// The pieces the moves move, as they stand after moving (a pawn promoted is its new piece), each
/// once.
fn moved(moves: &[Move], start: &Chess) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut position = start.clone();
    for m in moves {
        let color = position.turn();
        let mut add = |role: Role| {
            let piece = Piece { color, role };
            if !pieces.contains(&piece) {
                pieces.push(piece);
            }
        };
        match *m {
            Move::Castle { .. } => {
                add(Role::King);
                add(Role::Rook);
            }
            _ => add(m.promotion().unwrap_or(m.role())),
        }
        position.play_unchecked(*m);
    }
    pieces
}

/// A question on where `moves` lead from `start`, of a kind and on a square the draws pick among
/// those that fit: a square the moves touch, or a piece they move that is the only one of its kind.
pub fn ask(start: Chess, moves: Vec<Move>, draw: &mut dyn FnMut() -> f64) -> Option<Round> {
    // The moves must be the game's: each legal where it is played.
    let mut position = start.clone();
    let mut sans = Vec::with_capacity(moves.len());
    for m in &moves {
        if !position.is_legal(*m) {
            return None;
        }
        sans.push(SanPlus::from_move_and_play_unchecked(&mut position, *m).to_string());
    }
    let end = position;
    let board = end.board();
    let squares = touched(&moves, &start);
    let unique: Vec<Piece> = moved(&moves, &start)
        .into_iter()
        .filter(|piece| board.by_piece(*piece).count() == 1)
        .collect();
    let attacker = if pick(draw, 2) == 0 {
        Color::White
    } else {
        Color::Black
    };
    let open = |color: Color| -> Vec<Square> {
        squares
            .iter()
            .copied()
            .filter(|square| {
                board
                    .piece_at(*square)
                    .is_none_or(|piece| piece.color != color)
            })
            .collect()
    };
    let (attacker, targets) = match open(attacker) {
        targets if !targets.is_empty() => (attacker, targets),
        _ => (!attacker, open(!attacker)),
    };
    let mut kinds: Vec<u8> = Vec::new();
    if !squares.is_empty() {
        kinds.push(0);
    }
    if !unique.is_empty() {
        kinds.push(1);
    }
    if !targets.is_empty() {
        kinds.push(2);
    }
    if kinds.is_empty() {
        return None;
    }
    let question = match kinds[pick(draw, kinds.len())] {
        0 => Question::On(squares[pick(draw, squares.len())]),
        1 => Question::Where(unique[pick(draw, unique.len())]),
        _ => Question::Attacks(attacker, targets[pick(draw, targets.len())]),
    };
    let answer = question.answer(&end);
    let choices = match question {
        Question::On(square) => piece_choices(&start, &moves, square, answer, draw),
        Question::Where(_) => Vec::new(),
        Question::Attacks(..) => vec![Answer::Yes(true), Answer::Yes(false)],
    };
    Some(Round {
        start,
        moves,
        sans,
        question,
        answer,
        choices,
    })
}

/// Four answers to what stands on `square`, `answer` among them at a place the draws pick: what
/// stood there before the moves and what the moves took through it first, as the likeliest
/// mistakes, then empty, then pieces the draws pick.
fn piece_choices(
    start: &Chess,
    moves: &[Move],
    square: Square,
    answer: Answer,
    draw: &mut dyn FnMut() -> f64,
) -> Vec<Answer> {
    let mut near = vec![Answer::Piece(start.board().piece_at(square))];
    let mut position = start.clone();
    for m in moves {
        let color = position.turn();
        if m.from() == Some(square) || m.to() == square {
            near.push(Answer::Piece(Some(Piece {
                color,
                role: m.role(),
            })));
        }
        position.play_unchecked(*m);
    }
    near.push(Answer::Piece(None));
    let mut others: Vec<Answer> = [Color::White, Color::Black]
        .into_iter()
        .flat_map(|color| {
            ROLES
                .into_iter()
                .map(move |role| Answer::Piece(Some(Piece { color, role })))
        })
        .collect();
    let mut wrong: Vec<Answer> = Vec::new();
    for choice in near {
        if choice != answer && !wrong.contains(&choice) && wrong.len() < 3 {
            wrong.push(choice);
        }
    }
    while wrong.len() < 3 && !others.is_empty() {
        let choice = others.remove(pick(draw, others.len()));
        if choice != answer && !wrong.contains(&choice) {
            wrong.push(choice);
        }
    }
    let at = pick(draw, wrong.len() + 1);
    wrong.insert(at, answer);
    wrong
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_testkit::random::{self, Rng};
    use shakmaty::CastlingMode;
    use shakmaty::fen::Fen;
    use shakmaty::uci::UciMove;

    fn position(fen: &str) -> Chess {
        fen.parse::<Fen>()
            .expect("a FEN")
            .into_position(CastlingMode::Standard)
            .expect("a legal position")
    }

    fn moves(start: &Chess, ucis: &[&str]) -> Vec<Move> {
        let mut position = start.clone();
        ucis.iter()
            .map(|uci| {
                let m = uci
                    .parse::<UciMove>()
                    .expect("a UCI move")
                    .to_move(&position)
                    .expect("a legal move");
                position.play_unchecked(m);
                m
            })
            .collect()
    }

    /// Draws that always give the first choice.
    fn first() -> impl FnMut() -> f64 {
        || 0.
    }

    fn piece(color: Color, role: Role) -> Option<Piece> {
        Some(Piece { color, role })
    }

    #[test]
    fn each_question_is_answered_from_the_position() {
        // 1. e4 e5 2. Nf3 Nc6 3. Bb5
        let start = Chess::default();
        let mut end = start.clone();
        for m in moves(&start, &["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"]) {
            end.play_unchecked(m);
        }
        assert_eq!(
            Question::On(Square::F3).answer(&end),
            Answer::Piece(piece(Color::White, Role::Knight))
        );
        assert_eq!(Question::On(Square::G1).answer(&end), Answer::Piece(None));
        assert_eq!(
            Question::Where(Piece {
                color: Color::Black,
                role: Role::Queen
            })
            .answer(&end),
            Answer::Square(Square::D8)
        );
        // The bishop on b5 attacks c6; nothing of Black's attacks e4 yet, but the knight on f3
        // attacks e5.
        assert_eq!(
            Question::Attacks(Color::White, Square::C6).answer(&end),
            Answer::Yes(true)
        );
        assert_eq!(
            Question::Attacks(Color::Black, Square::E4).answer(&end),
            Answer::Yes(false)
        );
        assert_eq!(
            Question::Attacks(Color::White, Square::E5).answer(&end),
            Answer::Yes(true)
        );
        assert_eq!(
            Question::Attacks(Color::White, Square::C6).prompt(),
            "Does White attack c6?"
        );
        assert_eq!(
            Question::Where(Piece {
                color: Color::Black,
                role: Role::Queen
            })
            .prompt(),
            "Where is the black queen?"
        );
        assert_eq!(Question::On(Square::E4).prompt(), "What is on e4?");
    }

    #[test]
    fn a_piece_is_asked_for_only_while_it_is_the_only_one_of_its_kind() {
        // Both white knights stay on the board, so the knight that moves is never asked for;
        // the queen is.
        let start = Chess::default();
        let line = moves(&start, &["e2e4", "e7e5", "g1f3", "b8c6", "d1e2", "g8f6"]);
        rounds_of(&start, &line, 200, |round| {
            if let Question::Where(piece) = round.question {
                assert!(
                    matches!(
                        piece,
                        Piece {
                            role: Role::Queen,
                            color: Color::White
                        }
                    ),
                    "{piece:?}"
                );
            }
        });
    }

    /// Every round asked from `line` under `count` seeded draws.
    fn rounds_of(start: &Chess, line: &[Move], count: u64, mut check: impl FnMut(&Round)) {
        for seed in 0..count {
            let mut rng = Rng::new(seed);
            let mut draw = move || rng.below(1_000_000) as f64 / 1_000_000.;
            let round = ask(start.clone(), line.to_vec(), &mut draw).expect("a question");
            check(&round);
        }
    }

    #[test]
    fn castling_and_en_passant_touch_every_square_they_change() {
        let start = position("r3k2r/8/8/8/3pP3/8/8/R3K2R b KQkq e3 0 1");
        let line = moves(&start, &["d4e3", "e1g1"]);
        let squares = touched(&line, &start);
        for square in [
            Square::D4,
            Square::E3,
            Square::E4,
            Square::E1,
            Square::H1,
            Square::G1,
            Square::F1,
        ] {
            assert!(squares.contains(&square), "{square} in {squares:?}");
        }
        let pieces = moved(&line, &start);
        assert!(pieces.contains(&Piece {
            color: Color::White,
            role: Role::Rook
        }));
        // The pawn taken en passant is gone from e4.
        let mut draw = first();
        let round = ask(start.clone(), line, &mut draw).expect("a question");
        assert_eq!(
            Question::On(Square::E4).answer(&round.end()),
            Answer::Piece(None)
        );
        assert_eq!(
            Question::On(Square::F1).answer(&round.end()),
            Answer::Piece(piece(Color::White, Role::Rook))
        );
    }

    #[test]
    fn a_piece_question_offers_four_choices_with_its_answer_once() {
        let start = Chess::default();
        let line = moves(&start, &["e2e4", "d7d5", "e4d5"]);
        let mut draw = first();
        let round = ask(start, line, &mut draw).expect("a question");
        // The first kind, and the first square touched: e2, now empty.
        assert_eq!(round.question, Question::On(Square::E2));
        assert_eq!(round.answer, Answer::Piece(None));
        assert_eq!(round.choices.len(), 4);
        assert_eq!(round.choices[0], Answer::Piece(None));
        // What stood there, and the piece that left it, are among the wrong ones.
        assert!(
            round
                .choices
                .contains(&Answer::Piece(piece(Color::White, Role::Pawn)))
        );
        assert_eq!(round.moves_text(), "1. e4 d5 2. exd5");
        assert!(round.is_right(Answer::Piece(None)));
        assert!(!round.is_right(Answer::Piece(piece(Color::White, Role::Pawn))));
    }

    #[test]
    fn black_to_move_numbers_its_first_move() {
        let start = position("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        let line = moves(&start, &["e7e5", "g1f3", "b8c6"]);
        let mut draw = first();
        let round = ask(start, line, &mut draw).expect("a question");
        assert_eq!(round.moves_text(), "1… e5 2. Nf3 Nc6");
    }

    #[test]
    fn rounds_have_their_levels_length_and_stay_in_the_game() {
        let start = Chess::default();
        let game = moves(
            &start,
            &[
                "e2e4", "e7e5", "g1f3", "d7d6", "d2d4", "c8g4", "d4e5", "g4f3", "d1f3", "d6e5",
            ],
        );
        for plies in LEVEL_PLIES {
            let mut draw = first();
            let round = round(std::slice::from_ref(&game), plies, &mut draw).expect("a round");
            assert_eq!(round.moves.len(), plies);
            assert_eq!(round.sans.len(), plies);
            // The first place: the game's start.
            assert_eq!(&round.moves[..], &game[..plies]);
        }
        let mut draw = first();
        assert!(
            round(&[game], 11, &mut draw).is_none(),
            "longer than the game"
        );
    }

    /// Random games, random rounds: every round's moves replay legally from its position, and its
    /// answer is the rules' answer, among its choices once.
    #[test]
    fn random_rounds_replay_legally_and_answer_by_the_rules() {
        random::check(64, |rng| {
            let mut position = Chess::default();
            let mut game = Vec::new();
            for _ in 0..(20 + rng.below(60)) {
                let legal = position.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                position.play_unchecked(m);
                game.push(m);
            }
            let mut draws = Rng::new(rng.next_u64());
            let mut draw = move || draws.below(1_000_000) as f64 / 1_000_000.;
            for plies in LEVEL_PLIES {
                let Some(round) = round(std::slice::from_ref(&game), plies, &mut draw) else {
                    assert!(game.len() < plies, "a round of {plies} from {}", game.len());
                    continue;
                };
                let mut replay = round.start.clone();
                for (m, san) in round.moves.iter().zip(&round.sans) {
                    assert!(replay.is_legal(*m), "{m:?} at {:?}", replay.board());
                    assert_eq!(&SanPlus::from_move(replay.clone(), *m).to_string(), san);
                    replay.play_unchecked(*m);
                }
                let end = round.end();
                assert_eq!(replay.board(), end.board());
                assert_eq!(round.answer, round.question.answer(&end));
                match round.question {
                    Question::Where(piece) => {
                        assert_eq!(end.board().by_piece(piece).count(), 1);
                        assert!(round.choices.is_empty());
                    }
                    Question::Attacks(color, square) => {
                        assert!(
                            end.board()
                                .piece_at(square)
                                .is_none_or(|p| p.color != color)
                        );
                        assert_eq!(round.choices.len(), 2);
                    }
                    Question::On(_) => assert_eq!(round.choices.len(), 4),
                }
                if !round.choices.is_empty() {
                    assert_eq!(
                        round.choices.iter().filter(|c| **c == round.answer).count(),
                        1,
                        "{:?}",
                        round.choices
                    );
                    for (index, choice) in round.choices.iter().enumerate() {
                        assert!(
                            !round.choices[..index].contains(choice),
                            "{:?}",
                            round.choices
                        );
                    }
                }
            }
        });
    }
}
