//! What each piece is called and worth, and the material each side has.

use serde::{Serialize, Serializer};
use shakmaty::{Board, Color, Role};

/// The piece's name, as a player says it: `knight`.
pub fn name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

/// The piece a name stands for.
pub fn from_name(name: &str) -> Option<Role> {
    Role::ALL.into_iter().find(|&role| self::name(role) == name)
}

/// The piece's worth in pawns; the king has none, as it is never traded.
pub fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

/// `color`'s material on the board, in pawns.
pub fn of(board: &Board, color: Color) -> i32 {
    Role::ALL
        .iter()
        .map(|&role| value(role) * (board.by_color(color) & board.by_role(role)).count() as i32)
        .sum()
}

/// The pieces each side has beyond the other's, most valuable first, and `score`, the balance
/// in pawns (positive when White is ahead), as Lichess shows them beside the players.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diff {
    #[serde(serialize_with = "names")]
    pub white: Vec<Role>,
    #[serde(serialize_with = "names")]
    pub black: Vec<Role>,
    pub score: i64,
}

fn names<S: Serializer>(roles: &[Role], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(roles.iter().map(|&role| name(role)))
}

/// The material difference of a FEN: every piece letter of its first field counts.
pub fn diff(fen: &str) -> Diff {
    let mut counts = [[0i64; 6]; 2];
    for c in fen.split(' ').next().unwrap_or("").chars() {
        if let Some(role) = Role::from_char(c.to_ascii_lowercase()) {
            counts[usize::from(c.is_ascii_lowercase())][role as usize - 1] += 1;
        }
    }
    let [white_counts, black_counts] = counts;
    let mut diff = Diff {
        white: Vec::new(),
        black: Vec::new(),
        score: 0,
    };
    for role in Role::ALL.into_iter().rev() {
        let (ours, theirs) = (
            white_counts[role as usize - 1],
            black_counts[role as usize - 1],
        );
        let (side, extra) = if ours > theirs {
            (&mut diff.white, ours - theirs)
        } else {
            (&mut diff.black, theirs - ours)
        };
        side.extend(std::iter::repeat_n(role, extra as usize));
        diff.score += (ours - theirs) * i64::from(value(role));
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_back_to_their_pieces() {
        for role in Role::ALL {
            assert_eq!(from_name(name(role)), Some(role));
        }
        assert_eq!(from_name("Knight"), None);
    }

    #[test]
    fn a_diff_lists_the_extra_pieces_most_valuable_first() {
        let diff = diff("4k3/8/8/8/8/8/PP6/QN2K3 w - - 0 1");
        assert_eq!(
            diff.white,
            [Role::Queen, Role::Knight, Role::Pawn, Role::Pawn]
        );
        assert!(diff.black.is_empty());
        assert_eq!(diff.score, 14);
        let black = super::diff("4k1nr/8/8/8/8/8/8/4K3 b - - 0 1");
        assert_eq!(black.black, [Role::Rook, Role::Knight]);
        assert_eq!(black.score, -8);
    }
}
