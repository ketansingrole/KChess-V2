//! The board from the keyboard: Tab reaches it, the arrows move a cursor square as the viewer
//! sees the board (up is toward the opponent), Enter or Space presses the square under it as a
//! click does, and Escape lets go. Each square is named for a screen reader ("e4, white
//! knight"), and the cursor's square is the one it reads.

use shakmaty::{Color, File, Piece, Rank, Role, Square};

/// The key context the board's element carries while it has the keys.
pub(super) const CONTEXT: &str = "ChessBoard";

/// The promotion picker's pieces, in the order it shows them.
pub(super) const PROMOTION: [(Role, &str); 4] = [
    (Role::Queen, "Queen"),
    (Role::Rook, "Rook"),
    (Role::Bishop, "Bishop"),
    (Role::Knight, "Knight"),
];

/// Where an arrow key takes the cursor from `square`, on a board seen from `orientation`'s side;
/// the edge holds it. `None` for any other key.
pub(super) fn step(square: Square, orientation: Color, key: &str) -> Option<Square> {
    let (file, rank) = match key {
        "up" => (0, 1),
        "down" => (0, -1),
        "left" => (-1, 0),
        "right" => (1, 0),
        _ => return None,
    };
    let (file, rank) = match orientation {
        Color::White => (file, rank),
        Color::Black => (-file, -rank),
    };
    let file = i32::from(square.file()) + file;
    let rank = i32::from(square.rank()) + rank;
    if !(0..8).contains(&file) || !(0..8).contains(&rank) {
        return Some(square);
    }
    Some(Square::from_coords(
        File::new(file as u32),
        Rank::new(rank as u32),
    ))
}

/// Where the cursor starts: on the last move's square, or else in front of the viewer's king.
pub(super) fn start(orientation: Color, last_move: Option<(Square, Square)>) -> Square {
    match (last_move, orientation) {
        (Some((_, to)), _) => to,
        (None, Color::White) => Square::E2,
        (None, Color::Black) => Square::E7,
    }
}

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

/// A square's name for a screen reader: the square and what stands on it, which blindfold play
/// keeps to itself, and whether the piece picked can go there.
pub(super) fn square_name(
    square: Square,
    piece: Option<Piece>,
    blindfold: bool,
    target: bool,
) -> String {
    let mut name = square.to_string();
    if !blindfold {
        name.push_str(", ");
        match piece {
            Some(piece) => {
                name.push_str(if piece.color == Color::White {
                    "white "
                } else {
                    "black "
                });
                name.push_str(role_name(piece.role));
            }
            None => name.push_str("empty"),
        }
    }
    if target {
        name.push_str(", legal move");
    }
    name
}

/// A key while the promotion picker is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromotionKey {
    /// The arrows move between the pieces.
    Highlight(usize),
    /// Enter or Space takes the highlighted piece; Q, R, B or N takes that piece.
    Pick(Role),
    Cancel,
}

/// What `key` does in the promotion picker with the piece at `highlighted` marked.
pub(super) fn promotion_key(highlighted: usize, key: &str) -> Option<PromotionKey> {
    let count = PROMOTION.len();
    let highlighted = highlighted % count;
    Some(match key {
        "left" | "up" => PromotionKey::Highlight((highlighted + count - 1) % count),
        "right" | "down" => PromotionKey::Highlight((highlighted + 1) % count),
        "enter" | "space" => PromotionKey::Pick(PROMOTION[highlighted].0),
        "q" => PromotionKey::Pick(Role::Queen),
        "r" => PromotionKey::Pick(Role::Rook),
        "b" => PromotionKey::Pick(Role::Bishop),
        "n" => PromotionKey::Pick(Role::Knight),
        "escape" => PromotionKey::Cancel,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(from: Square, orientation: Color, keys: &[&str]) -> Square {
        keys.iter().fold(from, |square, key| {
            step(square, orientation, key).expect("an arrow")
        })
    }

    /// Up is toward the opponent and right is the viewer's right, whichever side is at the bottom.
    #[test]
    fn the_arrows_move_the_cursor_as_the_viewer_sees_the_board() {
        assert_eq!(step(Square::E2, Color::White, "up"), Some(Square::E3));
        assert_eq!(step(Square::E2, Color::White, "right"), Some(Square::F2));
        assert_eq!(step(Square::E2, Color::White, "down"), Some(Square::E1));
        assert_eq!(step(Square::E2, Color::White, "left"), Some(Square::D2));
        assert_eq!(step(Square::E7, Color::Black, "up"), Some(Square::E6));
        assert_eq!(step(Square::E7, Color::Black, "right"), Some(Square::D7));
        assert_eq!(step(Square::E7, Color::Black, "down"), Some(Square::E8));
        assert_eq!(step(Square::E7, Color::Black, "left"), Some(Square::F7));
        assert_eq!(
            walk(Square::E2, Color::White, &["up", "up", "left"]),
            Square::D4
        );
        assert_eq!(
            walk(Square::E7, Color::Black, &["up", "up", "left"]),
            Square::F5
        );
        assert_eq!(step(Square::E2, Color::White, "home"), None);
    }

    /// The edges hold the cursor rather than wrap it to the other side.
    #[test]
    fn the_edges_hold_the_cursor() {
        assert_eq!(step(Square::H8, Color::White, "up"), Some(Square::H8));
        assert_eq!(step(Square::H8, Color::White, "right"), Some(Square::H8));
        assert_eq!(step(Square::A1, Color::White, "down"), Some(Square::A1));
        assert_eq!(step(Square::A1, Color::Black, "up"), Some(Square::A1));
        assert_eq!(step(Square::A1, Color::Black, "right"), Some(Square::A1));
        assert_eq!(step(Square::H8, Color::Black, "left"), Some(Square::H8));
    }

    #[test]
    fn the_cursor_starts_on_the_last_move_or_before_the_king() {
        assert_eq!(start(Color::White, None), Square::E2);
        assert_eq!(start(Color::Black, None), Square::E7);
        assert_eq!(
            start(Color::Black, Some((Square::G1, Square::F3))),
            Square::F3
        );
    }

    #[test]
    fn a_square_is_named_with_its_piece() {
        let knight = Piece {
            color: Color::White,
            role: Role::Knight,
        };
        assert_eq!(
            square_name(Square::E4, Some(knight), false, false),
            "e4, white knight"
        );
        assert_eq!(square_name(Square::E4, None, false, false), "e4, empty");
        assert_eq!(
            square_name(Square::E4, None, false, true),
            "e4, empty, legal move"
        );
        // Blindfold play does not tell what stands there.
        assert_eq!(square_name(Square::E4, Some(knight), true, false), "e4");
    }

    #[test]
    fn the_promotion_picker_takes_arrows_letters_and_escape() {
        assert_eq!(promotion_key(0, "right"), Some(PromotionKey::Highlight(1)));
        assert_eq!(promotion_key(0, "left"), Some(PromotionKey::Highlight(3)));
        assert_eq!(promotion_key(3, "down"), Some(PromotionKey::Highlight(0)));
        assert_eq!(
            promotion_key(3, "enter"),
            Some(PromotionKey::Pick(Role::Knight))
        );
        assert_eq!(
            promotion_key(0, "space"),
            Some(PromotionKey::Pick(Role::Queen))
        );
        assert_eq!(promotion_key(0, "r"), Some(PromotionKey::Pick(Role::Rook)));
        assert_eq!(promotion_key(1, "escape"), Some(PromotionKey::Cancel));
        assert_eq!(promotion_key(1, "f"), None);
    }
}
