//! From the 64 squares as read to a position: which side was at the bottom, a few rules that
//! settle doubtful squares (never inventing a piece), the side to move and castling.

use shakmaty::{Board, Color, Piece, Role, Square};

use crate::Recognised;

/// How far a nearest piece may be from every bundled one before it is doubtful.
const FAR: f32 = 0.45;
/// How much nearer the nearest piece must be than the next for the square not to be doubtful.
const MARGIN: f32 = 0.06;
/// How much further a piece may be than the nearest for a rule to choose it instead.
const RULE_MARGIN: f32 = 0.35;

/// What one square of the picture shows.
pub(crate) enum Read {
    Empty {
        doubtful: bool,
    },
    /// A piece: every bundled piece by how unlike it is, the nearest first.
    Piece {
        ranked: Vec<(Piece, f32)>,
        doubtful: bool,
    },
}

/// Whether the nearest piece of `ranked` is too far, or too little nearer than the next.
pub(crate) fn doubtful_ranking(ranked: &[(Piece, f32)]) -> bool {
    match ranked {
        [(_, best), (_, next), ..] => *best > FAR || next - best < MARGIN,
        [(_, best)] => *best > FAR,
        [] => true,
    }
}

/// A square's reading once placed on the board.
struct Placed {
    square: Square,
    ranked: Vec<(Piece, f32)>,
    /// Which of `ranked` it now is.
    chosen: usize,
    doubtful: bool,
}

impl Placed {
    fn piece(&self) -> Piece {
        self.ranked[self.chosen].0
    }

    /// How much further than the nearest the `index`th choice is.
    fn cost(&self, index: usize) -> f32 {
        self.ranked[index].1 - self.ranked[0].1
    }

    /// Takes the nearest choice `allowed` lets it be, if one is near enough; doubtful either way.
    fn change(&mut self, allowed: impl Fn(Piece) -> bool) {
        self.doubtful = true;
        if let Some(index) = (0..self.ranked.len())
            .filter(|&index| allowed(self.ranked[index].0))
            .find(|&index| self.cost(index) <= RULE_MARGIN)
        {
            self.chosen = index;
        }
    }
}

/// How much the pieces of a printed diagram must tell of White at the top (in rows a pawn
/// stands past the middle) before it is read so: books print their diagrams from White's side,
/// and in an endgame both kings may stand far up the board. A board on a screen is as often
/// seen from Black's side.
const PRINTED_WHITE_BELOW: f32 = 3.;

/// Whether White sat at the bottom: where each side's pawns and king stand, from the picture's
/// top (row 0) down. With nothing to tell, or on a `printed` diagram little, White is at the
/// bottom.
fn white_at_bottom(reads: &[Read], printed: bool) -> bool {
    let mut lower = 0f32;
    for (index, read) in reads.iter().enumerate() {
        let Read::Piece { ranked, .. } = read else {
            continue;
        };
        let Some((piece, _)) = ranked.first() else {
            continue;
        };
        let weight = match piece.role {
            Role::Pawn => 1.,
            Role::King => 1.5,
            _ => 0.2,
        };
        let down = (index / 8) as f32 - 3.5;
        lower += weight * down * if piece.color == Color::White { 1. } else { -1. };
    }
    lower >= if printed { -PRINTED_WHITE_BELOW } else { 0. }
}

/// The position `reads` (the picture's squares from its top left, row by row) show, on a
/// `printed` diagram or not.
pub(crate) fn assemble(reads: &[Read], printed: bool, numbered: Option<bool>) -> Recognised {
    let white_at_bottom = numbered.unwrap_or_else(|| white_at_bottom(reads, printed));
    let square_of = |index: usize| {
        let (column, row) = ((index % 8) as u32, (index / 8) as u32);
        let (file, rank) = if white_at_bottom {
            (column, 7 - row)
        } else {
            (7 - column, row)
        };
        Square::from_coords(shakmaty::File::new(file), shakmaty::Rank::new(rank))
    };
    let mut doubtful = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    for (index, read) in reads.iter().enumerate() {
        match read {
            Read::Empty { doubtful: true } => doubtful.push(square_of(index)),
            Read::Empty { doubtful: false } => {}
            Read::Piece { ranked, doubtful } => placed.push(Placed {
                square: square_of(index),
                ranked: ranked.clone(),
                chosen: 0,
                doubtful: *doubtful,
            }),
        }
    }
    settle(&mut placed);
    let mut board = Board::empty();
    for place in &placed {
        board.set_piece_at(place.square, place.piece());
        if place.doubtful {
            doubtful.push(place.square);
        }
    }
    doubtful.sort();
    let turn = if board
        .king_of(Color::Black)
        .is_some_and(|king| board.attacks_to(king, Color::White, board.occupied()).any())
    {
        Color::Black
    } else {
        Color::White
    };
    let fen = format!(
        "{} {} {} - 0 1",
        board_fen(&board),
        if turn == Color::White { 'w' } else { 'b' },
        castling(&board)
    );
    Recognised {
        board,
        turn,
        fen,
        white_at_bottom,
        doubtful,
    }
}

/// The rules a position keeps, applied to what was read: one king a side (the likeliest kept,
/// the others read as their next likeliest piece; a side without one takes it from a piece that
/// came close to being its king), no pawn on the first or last rank, at most eight pawns a side.
fn settle(placed: &mut [Placed]) {
    for color in Color::ALL {
        let king = Piece {
            color,
            role: Role::King,
        };
        let mut kings: Vec<usize> = (0..placed.len())
            .filter(|&at| placed[at].piece() == king)
            .collect();
        kings.sort_by(|&a, &b| {
            placed[a].ranked[placed[a].chosen]
                .1
                .total_cmp(&placed[b].ranked[placed[b].chosen].1)
        });
        for &extra in kings.iter().skip(1) {
            placed[extra].change(|piece| piece.role != Role::King);
        }
        if kings.is_empty() {
            let closest = (0..placed.len())
                .filter_map(|at| {
                    let index = placed[at]
                        .ranked
                        .iter()
                        .position(|(piece, _)| *piece == king)?;
                    Some((at, index, placed[at].cost(index)))
                })
                .filter(|(at, _, cost)| {
                    *cost <= RULE_MARGIN && placed[*at].piece().role != Role::King
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((at, index, _)) = closest {
                placed[at].chosen = index;
                placed[at].doubtful = true;
            }
        }
    }
    for place in placed.iter_mut() {
        let edge = matches!(
            place.square.rank(),
            shakmaty::Rank::First | shakmaty::Rank::Eighth
        );
        if edge && place.piece().role == Role::Pawn {
            place.change(|piece| !matches!(piece.role, Role::Pawn | Role::King));
        }
    }
    for color in Color::ALL {
        let pawn = Piece {
            color,
            role: Role::Pawn,
        };
        let mut pawns: Vec<usize> = (0..placed.len())
            .filter(|&at| placed[at].piece() == pawn)
            .collect();
        // The least certain go first.
        pawns.sort_by(|&a, &b| {
            placed[b].ranked[placed[b].chosen]
                .1
                .total_cmp(&placed[a].ranked[placed[a].chosen].1)
        });
        for &extra in pawns.iter().take(pawns.len().saturating_sub(8)) {
            placed[extra].change(|piece| !matches!(piece.role, Role::Pawn | Role::King));
        }
    }
}

/// `board`'s placement as a FEN writes it.
fn board_fen(board: &Board) -> String {
    let mut fen = String::new();
    for rank in (0..8u32).rev() {
        let mut empty = 0;
        for file in 0..8u32 {
            let square = Square::from_coords(shakmaty::File::new(file), shakmaty::Rank::new(rank));
            match board.piece_at(square) {
                Some(piece) => {
                    if empty > 0 {
                        fen.push_str(&empty.to_string());
                        empty = 0;
                    }
                    fen.push(piece.char());
                }
                None => empty += 1,
            }
        }
        if empty > 0 {
            fen.push_str(&empty.to_string());
        }
        if rank > 0 {
            fen.push('/');
        }
    }
    fen
}

/// The castling rights whose king and rook stand on their starting squares.
fn castling(board: &Board) -> String {
    let rights: String = [
        ('K', Square::E1, Square::H1, Color::White),
        ('Q', Square::E1, Square::A1, Color::White),
        ('k', Square::E8, Square::H8, Color::Black),
        ('q', Square::E8, Square::A8, Color::Black),
    ]
    .into_iter()
    .filter(|&(_, king, rook, color)| {
        board.piece_at(king)
            == Some(Piece {
                color,
                role: Role::King,
            })
            && board.piece_at(rook)
                == Some(Piece {
                    color,
                    role: Role::Rook,
                })
    })
    .map(|(right, ..)| right)
    .collect();
    if rights.is_empty() {
        "-".into()
    } else {
        rights
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(piece: Piece, others: &[(Piece, f32)]) -> Read {
        let mut ranked = vec![(piece, 0.1)];
        ranked.extend_from_slice(others);
        Read::Piece {
            ranked,
            doubtful: false,
        }
    }

    const fn piece(color: Color, role: Role) -> Piece {
        Piece { color, role }
    }

    /// The start position read square by square, from either side.
    #[test]
    fn the_start_position_from_either_side() {
        let start = Board::default();
        let reads = |flipped: bool| -> Vec<Read> {
            (0..64)
                .map(|index| {
                    let (column, row) = ((index % 8) as u32, (index / 8) as u32);
                    let (file, rank) = if flipped {
                        (7 - column, row)
                    } else {
                        (column, 7 - row)
                    };
                    let square =
                        Square::from_coords(shakmaty::File::new(file), shakmaty::Rank::new(rank));
                    match start.piece_at(square) {
                        Some(piece) => read(piece, &[]),
                        None => Read::Empty { doubtful: false },
                    }
                })
                .collect()
        };
        for flipped in [false, true] {
            let recognised = assemble(&reads(flipped), false, None);
            assert_eq!(
                recognised.fen,
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
            );
            assert_eq!(recognised.white_at_bottom, !flipped);
            assert!(recognised.doubtful.is_empty());
        }
    }

    /// An endgame whose kings stand far up the board, White's further than Black's, is read
    /// from White's side when printed, as books print it, and from Black's on a screen.
    #[test]
    fn a_printed_endgame_with_advanced_kings_is_read_from_whites_side() {
        let board: Board = "8/8/2K5/8/P4k2/8/8/8"
            .parse::<shakmaty::fen::Fen>()
            .unwrap()
            .into_setup()
            .board;
        let reads: Vec<Read> = (0..64)
            .map(|index| {
                let (column, row) = ((index % 8) as u32, (index / 8) as u32);
                let square =
                    Square::from_coords(shakmaty::File::new(column), shakmaty::Rank::new(7 - row));
                match board.piece_at(square) {
                    Some(piece) => read(piece, &[]),
                    None => Read::Empty { doubtful: false },
                }
            })
            .collect();
        let recognised = assemble(&reads, true, None);
        assert!(recognised.white_at_bottom);
        assert_eq!(recognised.fen, "8/8/2K5/8/P4k2/8/8/8 w - - 0 1");
        assert!(!assemble(&reads, false, None).white_at_bottom);
    }

    /// A second white king is read as its next likeliest piece, a pawn on the last rank too, and
    /// Black in check is Black to move.
    #[test]
    fn rules_settle_doubtful_squares() {
        let mut reads: Vec<Read> = (0..64).map(|_| Read::Empty { doubtful: false }).collect();
        // a8 (top left): a white pawn that is nearly a white queen.
        reads[0] = read(
            piece(Color::White, Role::Pawn),
            &[(piece(Color::White, Role::Queen), 0.2)],
        );
        // e8: the black king, in check from the queen on a8 once it is one.
        reads[4] = read(piece(Color::Black, Role::King), &[]);
        // e1 and d1: two white kings, the one on d1 nearly a rook.
        reads[60] = read(piece(Color::White, Role::King), &[]);
        reads[59] = Read::Piece {
            ranked: vec![
                (piece(Color::White, Role::King), 0.15),
                (piece(Color::White, Role::Rook), 0.2),
            ],
            doubtful: false,
        };
        reads[52] = read(piece(Color::White, Role::Pawn), &[]);
        let recognised = assemble(&reads, false, None);
        assert_eq!(recognised.fen, "Q3k3/8/8/8/8/8/4P3/3RK3 b - - 0 1");
        assert_eq!(recognised.doubtful, vec![Square::D1, Square::A8]);
        assert_eq!(recognised.turn, Color::Black);
    }

    /// A side read without a king takes it from the piece that came closest to being its king,
    /// a ninth pawn is read as its next likeliest piece, and a square read as doubtfully empty
    /// stays marked.
    #[test]
    fn a_missing_king_and_a_ninth_pawn_are_settled() {
        let mut reads: Vec<Read> = (0..64).map(|_| Read::Empty { doubtful: false }).collect();
        // e8: a black queen that is nearly the black king; b8 a black rook that is less so.
        reads[4] = read(
            piece(Color::Black, Role::Queen),
            &[(piece(Color::Black, Role::King), 0.2)],
        );
        reads[1] = read(
            piece(Color::Black, Role::Rook),
            &[(piece(Color::Black, Role::King), 0.4)],
        );
        reads[60] = read(piece(Color::White, Role::King), &[]);
        // Eight white pawns on the second rank and a ninth on c4, the least certain of them,
        // nearly a white bishop.
        for square in &mut reads[48..56] {
            *square = read(piece(Color::White, Role::Pawn), &[]);
        }
        reads[34] = Read::Piece {
            ranked: vec![
                (piece(Color::White, Role::Pawn), 0.3),
                (piece(Color::White, Role::Bishop), 0.35),
            ],
            doubtful: false,
        };
        // h5: doubtfully empty.
        reads[31] = Read::Empty { doubtful: true };
        let recognised = assemble(&reads, false, None);
        assert_eq!(recognised.fen, "1r2k3/8/8/8/2B5/8/PPPPPPPP/4K3 w - - 0 1");
        assert_eq!(
            recognised.doubtful,
            vec![Square::C4, Square::H5, Square::E8]
        );
        assert!(doubtful_ranking(&[]));
        assert!(!doubtful_ranking(&[(piece(Color::White, Role::Pawn), 0.1)]));
        assert!(doubtful_ranking(&[(piece(Color::White, Role::Pawn), 0.5)]));
    }
}
