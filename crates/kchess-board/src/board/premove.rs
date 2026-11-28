//! Where a piece may be premoved: Chessground's `premove` with the requirements Lichess adds
//! (lila's `ui/round/src/premove.ts`), so a square is offered only when the move could still be
//! legal once the opponent has moved. A rook behind its own pawn offers nothing; a pawn offers a
//! capture only where something could stand to be taken.

use shakmaty::variant::VariantPosition;
use shakmaty::{Board, CastlingMode, Color, File, Piece, Position as _, Rank, Role, Square};

/// The variant's premove rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PremoveRules {
    #[default]
    Standard,
    /// The king castles by moving onto its rook, wherever they start.
    Chess960,
    /// No castling.
    Antichess,
    /// Atomic and Crazyhouse: anything can happen to the squares between, so any square the
    /// piece's moves reach.
    Unrestricted,
}

impl PremoveRules {
    pub fn of(position: &VariantPosition) -> PremoveRules {
        match position {
            VariantPosition::Atomic(_) | VariantPosition::Crazyhouse(_) => {
                PremoveRules::Unrestricted
            }
            VariantPosition::Antichess(_) => PremoveRules::Antichess,
            _ if position.castles().mode() == CastlingMode::Chess960 => PremoveRules::Chess960,
            _ => PremoveRules::Standard,
        }
    }
}

type Pos = (i32, i32);

fn pos(square: Square) -> Pos {
    (i32::from(square.file()), i32::from(square.rank()))
}

fn square((x, y): Pos) -> Option<Square> {
    ((0..8).contains(&x) && (0..8).contains(&y))
        .then(|| Square::from_coords(File::new(x as u32), Rank::new(y as u32)))
}

/// The squares strictly between two on a line or diagonal; none for any other pair.
fn between((x1, y1): Pos, (x2, y2): Pos) -> Vec<Square> {
    let (dx, dy) = (x2 - x1, y2 - y1);
    if dx != 0 && dy != 0 && dx.abs() != dy.abs() {
        return Vec::new();
    }
    let (sx, sy) = (dx.signum(), dy.signum());
    let mut squares = Vec::new();
    let (mut x, mut y) = (x1 + sx, y1 + sy);
    while (x, y) != (x2, y2) {
        squares.extend(square((x, y)));
        x += sx;
        y += sy;
    }
    squares
}

fn forward(color: Color) -> i32 {
    if color == Color::White { 1 } else { -1 }
}

fn pawn_advance((x1, y1): Pos, (x2, y2): Pos, color: Color) -> bool {
    let step = forward(color);
    let home = if color == Color::White {
        y1 <= 1
    } else {
        y1 >= 6
    };
    x1 == x2 && (y2 == y1 + step || (y2 == y1 + 2 * step && home))
}

fn pawn_capture((x1, y1): Pos, (x2, y2): Pos, color: Color) -> bool {
    (x1 - x2).abs() == 1 && y2 == y1 + forward(color)
}

fn knight((x1, y1): Pos, (x2, y2): Pos) -> bool {
    let (dx, dy) = ((x1 - x2).abs(), (y1 - y2).abs());
    (dx == 1 && dy == 2) || (dx == 2 && dy == 1)
}

fn bishop((x1, y1): Pos, (x2, y2): Pos) -> bool {
    let (dx, dy) = ((x1 - x2).abs(), (y1 - y2).abs());
    dx == dy && dx != 0
}

fn rook((x1, y1): Pos, (x2, y2): Pos) -> bool {
    (x1 == x2) != (y1 == y2)
}

fn king((x1, y1): Pos, (x2, y2): Pos) -> bool {
    let (dx, dy) = ((x1 - x2).abs(), (y1 - y2).abs());
    dx <= 1 && dy <= 1 && dx + dy > 0
}

fn shifted(square: Square, delta: i32) -> Option<Square> {
    let (x, y) = pos(square);
    self::square((x, y + delta))
}

/// One premove being weighed (Chessground's `MobilityContext`).
struct Mobility<'a> {
    pieces: &'a Board,
    color: Color,
    role: Role,
    orig: Square,
    dest: Square,
    last_move: Option<(Square, Square)>,
    rules: PremoveRules,
}

impl Mobility<'_> {
    fn friend(&self, square: Square) -> bool {
        self.pieces.color_at(square) == Some(self.color)
    }

    fn enemy(&self, square: Square) -> bool {
        self.pieces.color_at(square) == Some(!self.color)
    }

    fn enemy_pawn(&self, square: Square) -> bool {
        self.pieces.piece_at(square)
            == Some(Piece {
                color: !self.color,
                role: Role::Pawn,
            })
    }

    fn any_between(&self, a: Pos, b: Pos) -> bool {
        between(a, b)
            .into_iter()
            .any(|square| self.pieces.piece_at(square).is_some())
    }

    fn enemies(&self) -> Vec<Square> {
        self.pieces.by_color(!self.color).into_iter().collect()
    }

    /// An enemy pawn on `start` could step to `dest`.
    fn enemy_pawn_advances_to(&self, start: Square, dest: Square) -> bool {
        if !self.enemy_pawn(start) {
            return false;
        }
        let (x, y) = pos(dest);
        pawn_advance(pos(start), pos(dest), !self.color)
            && !self.any_between(pos(start), (x, y + forward(!self.color)))
    }

    /// An enemy pawn on `start` could take on `dest`.
    fn enemy_pawn_takes_on(&self, start: Square, dest: Square) -> bool {
        self.enemy_pawn(start)
            && pawn_capture(pos(start), pos(dest), !self.color)
            && (self.friend(dest)
                || self.en_passant(shifted(dest, -forward(!self.color)), None, &[]))
    }

    /// One of `enemies` (all of them when none are named) attacks the destination.
    fn attacked(&self, without: Option<Role>, enemies: Option<Square>) -> bool {
        let dest = pos(self.dest);
        let enemies = enemies.map_or_else(|| self.enemies(), |square| vec![square]);
        enemies.into_iter().any(|square| {
            let Some(piece) = self.pieces.piece_at(square) else {
                return false;
            };
            let from = pos(square);
            Some(piece.role) != without
                && match piece.role {
                    Role::Pawn => pawn_capture(from, dest, piece.color),
                    Role::Knight => knight(from, dest),
                    Role::Bishop => bishop(from, dest),
                    Role::Rook => rook(from, dest),
                    Role::Queen => bishop(from, dest) || rook(from, dest),
                    Role::King => king(from, dest),
                }
                && (!matches!(piece.role, Role::Bishop | Role::Rook | Role::Queen)
                    || !self.any_between(from, dest))
        })
    }

    /// A friendly pawn on `pawn`, just pushed two squares, could be taken en passant by one of
    /// `enemies`, leaving its square, unless the square it would be taken on is `forbidden`.
    fn en_passant(
        &self,
        pawn: Option<Square>,
        enemies: Option<Square>,
        forbidden: &[Square],
    ) -> bool {
        let Some(pawn) = pawn else {
            return false;
        };
        if self.last_move.is_some_and(|(_, to)| to != pawn) {
            return false;
        }
        let (x, y) = pos(pawn);
        let fifth = if self.color == Color::White { 3 } else { 4 };
        self.pieces.piece_at(pawn)
            == Some(Piece {
                color: self.color,
                role: Role::Pawn,
            })
            && y == fifth
            && self
                .last_move
                .is_none_or(|(from, _)| (pos(from).1 - y).abs() == 2)
            && [1, -1].into_iter().any(|delta| {
                square((x + delta, y)).is_some_and(|beside| {
                    enemies.is_none_or(|only| only == beside) && self.enemy_pawn(beside)
                })
            })
            && !shifted(pawn, -forward(self.color))
                .is_some_and(|square| forbidden.contains(&square))
    }

    /// What stands between the piece and its destination could be gone by its turn: at most one
    /// piece of each side, an enemy pawn there able to move off the line, and a friend between
    /// only if it could be taken en passant.
    fn path_clear(&self, pawn_advance: bool) -> bool {
        if self.rules == PremoveRules::Unrestricted {
            return true;
        }
        let mut squares = between(pos(self.orig), pos(self.dest));
        if pawn_advance {
            squares.push(self.dest);
        }
        let friends: Vec<Square> = squares
            .iter()
            .copied()
            .filter(|&s| self.friend(s))
            .collect();
        let enemies: Vec<Square> = squares.iter().copied().filter(|&s| self.enemy(s)).collect();
        if friends.len() > 1 || enemies.len() > 1 {
            return false;
        }
        let friend = friends.first().copied();
        let enemy = enemies.first().copied();
        if let Some(enemy) = enemy
            && self.enemy_pawn(enemy)
        {
            let step = forward(!self.color);
            let dests: Vec<Square> = shifted(enemy, step)
                .map(|above| {
                    let (x, y) = pos(above);
                    [square((x - 1, y)), square((x + 1, y))]
                        .into_iter()
                        .flatten()
                        .filter(|&s| self.enemy_pawn_takes_on(enemy, s))
                        .chain(
                            [Some(above), shifted(above, step)]
                                .into_iter()
                                .flatten()
                                .filter(|&s| self.enemy_pawn_advances_to(enemy, s)),
                        )
                        .collect()
                })
                .unwrap_or_default();
            if dests
                .iter()
                .all(|square| squares.contains(square) || *square == self.orig)
            {
                return false;
            }
        }
        if !pawn_advance && self.friend(self.dest) {
            friend.is_none()
                && (self.attacked(None, enemy) || self.en_passant(Some(self.dest), enemy, &squares))
        } else {
            friend.is_none() || self.en_passant(friend, enemy, &squares)
        }
    }

    fn pawn(&self) -> bool {
        let (orig, dest) = (pos(self.orig), pos(self.dest));
        let step = forward(self.color);
        if (orig.0 - dest.0).abs() > 1 {
            return false;
        }
        if orig.0 == dest.0 {
            return pawn_advance(orig, dest, self.color) && self.path_clear(true);
        }
        if dest.1 != orig.1 + step {
            return false;
        }
        if self.rules == PremoveRules::Unrestricted || self.enemy(self.dest) {
            return true;
        }
        if self.friend(self.dest) {
            return self.attacked(None, None);
        }
        self.enemies()
            .into_iter()
            .any(|enemy| self.enemy_pawn_advances_to(enemy, self.dest))
            || self.en_passant(square((dest.0, dest.1 + step)), None, &[])
            || self.attacked(Some(Role::Pawn), None)
    }

    fn king(&self) -> bool {
        let (orig, dest) = (pos(self.orig), pos(self.dest));
        let unrestricted = self.rules == PremoveRules::Unrestricted;
        if king(orig, dest)
            && (unrestricted
                || !self.friend(self.dest)
                || self.en_passant(Some(self.dest), None, &[])
                || self.attacked(None, None))
        {
            return true;
        }
        let home = if self.color == Color::White { 0 } else { 7 };
        let rook_on = |file: i32| {
            square((file, home)).is_some_and(|square| {
                self.pieces.piece_at(square)
                    == Some(Piece {
                        color: self.color,
                        role: Role::Rook,
                    })
            })
        };
        let castles = match self.rules {
            PremoveRules::Antichess => false,
            PremoveRules::Chess960 => rook_on(dest.0),
            // A king on the e-file castles two squares over or onto its rook.
            PremoveRules::Standard | PremoveRules::Unrestricted => {
                orig.0 == 4
                    && ((dest.0 == 2 && rook_on(0))
                        || (dest.0 == 6 && rook_on(7))
                        || rook_on(dest.0))
            }
        };
        castles
            && orig.1 == home
            && dest.1 == home
            && (unrestricted
                || between(orig, (if dest.0 > orig.0 { 7 } else { 1 }, dest.1))
                    .into_iter()
                    .filter_map(|square| self.pieces.piece_at(square))
                    .all(|piece| {
                        piece
                            == Piece {
                                color: self.color,
                                role: Role::Rook,
                            }
                    }))
    }

    fn allowed(&self) -> bool {
        let (orig, dest) = (pos(self.orig), pos(self.dest));
        match self.role {
            Role::Pawn => self.pawn(),
            Role::King => self.king(),
            Role::Knight => knight(orig, dest) && self.path_clear(false),
            Role::Bishop => bishop(orig, dest) && self.path_clear(false),
            Role::Rook => rook(orig, dest) && self.path_clear(false),
            Role::Queen => (bishop(orig, dest) || rook(orig, dest)) && self.path_clear(false),
        }
    }
}

/// The squares the piece on `from` may be premoved to.
pub fn targets(
    pieces: &Board,
    from: Square,
    last_move: Option<(Square, Square)>,
    rules: PremoveRules,
) -> Vec<Square> {
    let Some(piece) = pieces.piece_at(from) else {
        return Vec::new();
    };
    Square::ALL
        .into_iter()
        .filter(|&dest| {
            dest != from
                && Mobility {
                    pieces,
                    color: piece.color,
                    role: piece.role,
                    orig: from,
                    dest,
                    last_move,
                    rules,
                }
                .allowed()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{PremoveRules, targets};
    use shakmaty::Square;

    fn premoves(fen: &str, from: Square, last: Option<(Square, Square)>) -> Vec<Square> {
        targets(
            &super::super::board_of(fen),
            from,
            last,
            PremoveRules::Standard,
        )
    }

    /// The squares lila's rules offer, worked through square by square: behind its own pawn a
    /// rook offers nothing, a pawn takes only where something could stand, a bishop sees past
    /// one enemy but not two, the king castles only through empty squares.
    #[test]
    fn premoves_offer_only_squares_that_could_still_be_legal() {
        use Square::*;
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        assert!(
            premoves(start, A1, None).is_empty(),
            "the a-pawn holds the rook in"
        );
        assert_eq!(
            premoves(start, E2, None),
            [E3, E4],
            "nothing to take on d3 or f3"
        );
        assert_eq!(premoves(start, G1, None), [F3, H3]);
        // After 1.e4 the bishop sees down the diagonal its pawn left, to the edge.
        let bishop = premoves(after_e4, F1, Some((E2, E4)));
        assert_eq!(bishop, [E2, D3, C4, B5, A6]);
        // The e4 pawn takes on d5 or f5 only because a black pawn could step there.
        let pawn = premoves(after_e4, E4, Some((E2, E4)));
        assert_eq!(pawn, [D5, E5, F5]);
        // Castling with the f1 bishop and g1 knight in the way is not offered; clear of them it is.
        assert!(!premoves(start, E1, None).contains(&G1));
        let clear = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        let king = premoves(clear, E1, None);
        assert!(king.contains(&G1) && king.contains(&C1) && king.contains(&H1));
    }

    /// In Atomic and Crazyhouse anything may come off the line first, and Antichess never castles.
    #[test]
    fn the_variants_premove_by_their_own_rules() {
        use Square::*;
        let start = super::super::board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w - - 0 1");
        let rook = targets(&start, A1, None, PremoveRules::Unrestricted);
        assert!(rook.contains(&A8) && rook.contains(&H1));
        let clear = super::super::board_of("r3k2r/8/8/8/8/8/8/R3K2R w - - 0 1");
        assert!(!targets(&clear, E1, None, PremoveRules::Antichess).contains(&G1));
        // Chess960 castles by moving onto the rook, not two squares over.
        let castles = targets(&clear, E1, None, PremoveRules::Chess960);
        assert!(castles.contains(&H1) && castles.contains(&A1));
    }
}
