//! The board editor's position: its board field, side to move, castling rights and en passant
//! square, and the FEN they make.

use serde::{Deserialize, Serialize};
use shakmaty::Color;

use crate::js;
use crate::voice::FILES;

/// A JavaScript array index: an integer within `0..len` (anything else reads as undefined).
fn at_index(index: f64, len: usize) -> Option<usize> {
    (index.fract() == 0.0 && index >= 0.0 && (index as usize) < len).then_some(index as usize)
}

/// `Number(unit)` for one UTF-16 code unit of a square name (NaN when absent).
pub(crate) fn unit_number(unit: Option<&u16>) -> f64 {
    match unit {
        None => f64::NAN,
        Some(unit) => js::string_to_number(&String::from_utf16_lossy(&[*unit])),
    }
}

/// A square name's first UTF-16 unit minus `a`'s (so `a` is 0); None when empty.
pub(crate) fn file_offset(square: &str) -> Option<i64> {
    square
        .encode_utf16()
        .next()
        .map(|unit| i64::from(unit) - 97)
}

/// The piece letter on a square of a FEN board field (`e1` → `K`), or None.
fn piece_on(board: &str, square: &str) -> Option<char> {
    let units: Vec<u16> = square.encode_utf16().collect();
    let file = file_offset(square);
    let rows: Vec<&str> = board.split('/').collect();
    let row = rows[at_index(8.0 - unit_number(units.get(1)), rows.len())?];
    let mut index: i64 = 0;
    for c in row.chars() {
        if c.is_ascii_digit() {
            index += i64::from(c as u8 - b'0');
        } else {
            let hit = file == Some(index);
            index += 1;
            if hit {
                return Some(c);
            }
        }
        if file.is_some_and(|f| index > f) {
            return None;
        }
    }
    None
}

fn letter_for(role: &str) -> Option<char> {
    Some(match role {
        "king" => 'k',
        "queen" => 'q',
        "rook" => 'r',
        "bishop" => 'b',
        "knight" => 'n',
        "pawn" => 'p',
        _ => return None,
    })
}

/// Each digit of a board row stands for that many empty squares.
fn expand_digits(row: &str) -> String {
    let mut out = String::new();
    for c in row.chars() {
        if c.is_ascii_digit() {
            out.extend(std::iter::repeat_n('.', usize::from(c as u8 - b'0')));
        } else {
            out.push(c);
        }
    }
    out
}

/// Runs of empty squares back to digits.
fn compress_row(row: &str) -> String {
    let mut out = String::new();
    let mut run = 0usize;
    for c in row.chars() {
        if c == '.' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.push_str(&run.to_string());
            run = 0;
        }
        out.push(c);
    }
    if run > 0 {
        out.push_str(&run.to_string());
    }
    out
}

/// The board field with `square` holding `piece` (its colour and role), or emptied when `piece`
/// is None.
pub fn with_piece(
    board: &str,
    square: &str,
    piece: Option<(&str, &str)>,
) -> Result<String, String> {
    let mut rows: Vec<String> = board.split('/').map(expand_digits).collect();
    let units: Vec<u16> = square.encode_utf16().collect();
    let rank =
        at_index(8.0 - unit_number(units.get(1)), rows.len()).ok_or("the board has no such row")?;
    let letter = match piece {
        None => '.',
        Some((color, role)) => {
            let letter = letter_for(role).ok_or("unknown piece role")?;
            if color == "white" {
                letter.to_ascii_uppercase()
            } else {
                letter
            }
        }
    };
    let mut cells: Vec<Option<char>> = rows[rank].chars().map(Some).collect();
    // A file past the board's end leaves holes, which `join` writes as nothing.
    if let Some(file) = file_offset(square).filter(|file| *file >= 0) {
        let file = file as usize;
        if file >= cells.len() {
            cells.resize(file + 1, None);
        }
        cells[file] = Some(letter);
    }
    rows[rank] = cells.iter().flatten().collect();
    Ok(rows
        .iter()
        .map(|row| compress_row(row))
        .collect::<Vec<_>>()
        .join("/"))
}

/// Castling rights, written as X-FEN's `KQkq`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastlingRights {
    #[serde(rename = "K", default)]
    pub white_short: bool,
    #[serde(rename = "Q", default)]
    pub white_long: bool,
    #[serde(rename = "k", default)]
    pub black_short: bool,
    #[serde(rename = "q", default)]
    pub black_long: bool,
}

impl CastlingRights {
    /// The right a FEN letter names: `K`, `Q`, `k` or `q`.
    pub fn has(self, letter: &str) -> bool {
        match letter {
            "K" => self.white_short,
            "Q" => self.white_long,
            "k" => self.black_short,
            "q" => self.black_long,
            _ => false,
        }
    }

    /// Tick or untick the right a FEN letter names; any other letter changes nothing.
    pub fn set(&mut self, letter: &str, ticked: bool) {
        match letter {
            "K" => self.white_short = ticked,
            "Q" => self.white_long = ticked,
            "k" => self.black_short = ticked,
            "q" => self.black_long = ticked,
            _ => {}
        }
    }

    /// In `KQkq` order.
    fn flags(self) -> [bool; 4] {
        [
            self.white_short,
            self.white_long,
            self.black_short,
            self.black_long,
        ]
    }
}

/// Which castling rights the pieces allow, by the Chess960 rule that includes the standard one:
/// the king on its back rank (b to g), and a rook on that rank on the side castled to. The FEN
/// writes them as X-FEN's `KQkq`, which name the outermost rook on each side.
pub fn castling_available(board: &str) -> CastlingRights {
    let side = |rank: char, king: char, rook: char| {
        let at = |file: usize, piece: char| {
            piece_on(board, &format!("{}{rank}", FILES[file])) == Some(piece)
        };
        let Some(king_file) = (1..7).find(|file| at(*file, king)) else {
            return [false, false];
        };
        [
            (king_file + 1..8).any(|file| at(file, rook)),
            (0..king_file).any(|file| at(file, rook)),
        ]
    };
    let [white_short, white_long] = side('1', 'K', 'R');
    let [black_short, black_long] = side('8', 'k', 'r');
    CastlingRights {
        white_short,
        white_long,
        black_short,
        black_long,
    }
}

/// Squares a pawn could just have skipped with a double step, so the side to move might capture
/// en passant there.
pub fn en_passant_squares(board: &str, turn: Color) -> Vec<String> {
    let (pawn, rank, passed, start) = if turn == Color::White {
        ('p', '5', '6', '7')
    } else {
        ('P', '4', '3', '2')
    };
    FILES
        .iter()
        .filter(|file| {
            piece_on(board, &format!("{file}{rank}")) == Some(pawn)
                && piece_on(board, &format!("{file}{passed}")).is_none()
                && piece_on(board, &format!("{file}{start}")).is_none()
        })
        .map(|file| format!("{file}{passed}"))
        .collect()
}

/// The board editor's position: the board field, the side to move, the castling rights ticked
/// and the en passant square chosen (empty for none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSetup {
    pub board: String,
    pub turn: Color,
    pub castling: CastlingRights,
    pub ep: String,
}

impl EditorSetup {
    /// The standard starting position.
    pub fn start() -> EditorSetup {
        EditorSetup {
            board: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR".into(),
            turn: Color::White,
            castling: CastlingRights {
                white_short: true,
                white_long: true,
                black_short: true,
                black_long: true,
            },
            ep: String::new(),
        }
    }

    /// The position a FEN writes; none when it is no FEN. Shredder-FEN and X-FEN castling
    /// (`HAha`) is ticked as the side its rook stands on, and only a third- or sixth-rank square
    /// is kept for en passant.
    pub fn from_fen(text: &str) -> Option<EditorSetup> {
        let trimmed = text.trim();
        let parsed = crate::position::fen_setup(trimmed)?;
        // Split as `fen_setup` read it: Lichess writes a FEN's fields with `_` between them.
        let fields = crate::rules::split_fen(trimmed);
        let rights = castling_letters(&parsed.board, fields.get(2).copied().unwrap_or("-"));
        // `fen_setup` read the square, so it is `-` or a square: kept on the third or sixth rank.
        let ep = fields
            .get(3)
            .copied()
            .filter(|ep| ep.ends_with(['3', '6']))
            .unwrap_or_default();
        Some(EditorSetup {
            turn: crate::color::from_name(parsed.turn)?,
            board: parsed.board,
            castling: CastlingRights {
                white_short: rights.contains('K'),
                white_long: rights.contains('Q'),
                black_short: rights.contains('k'),
                black_long: rights.contains('q'),
            },
            ep: ep.to_string(),
        })
    }

    /// Its FEN: castling rights and en passant only where the pieces allow them.
    pub fn fen(&self) -> String {
        let (board, ep) = (self.board.as_str(), self.ep.as_str());
        let rights: String = ["K", "Q", "k", "q"]
            .iter()
            .zip(castling_available(board).flags())
            .zip(self.castling.flags())
            .filter(|((_, allowed), ticked)| *allowed && *ticked)
            .map(|((right, _), _)| *right)
            .collect();
        let placement = if self.turn == Color::White { "w" } else { "b" };
        let ep = if en_passant_squares(board, self.turn).iter().any(|s| s == ep) {
            ep
        } else {
            "-"
        };
        format!(
            "{board} {placement} {} {ep} 0 1",
            if rights.is_empty() { "-" } else { &rights }
        )
    }
}

/// Castling rights as `KQkq`: Shredder-FEN and X-FEN name a rook by its file (`HAha`), which is
/// the short side when it stands right of its king.
fn castling_letters(board: &str, rights: &str) -> String {
    let king_file = |rank: usize, king: char| {
        let mut file = 0;
        for c in board.split('/').nth(rank).unwrap_or_default().chars() {
            match c.to_digit(10) {
                Some(skip) => file += skip as usize,
                None if c == king => return Some(file),
                None => file += 1,
            }
        }
        None
    };
    rights
        .chars()
        .filter_map(|c| match c {
            'K' | 'Q' | 'k' | 'q' => Some(c),
            'A'..='H' | 'a'..='h' => {
                let white = c.is_ascii_uppercase();
                let king = king_file(if white { 7 } else { 0 }, if white { 'K' } else { 'k' })?;
                let file = (c.to_ascii_lowercase() as u8 - b'a') as usize;
                let short = file > king;
                Some(match (white, short) {
                    (true, true) => 'K',
                    (true, false) => 'Q',
                    (false, true) => 'k',
                    (false, false) => 'q',
                })
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: CastlingRights = CastlingRights {
        white_short: true,
        white_long: true,
        black_short: true,
        black_long: true,
    };

    #[test]
    fn the_start_writes_the_standard_fen() {
        assert_eq!(EditorSetup::start().fen(), crate::rules::INITIAL_FEN);
        let mut setup = EditorSetup::start();
        setup.board = "4k3/8/8/8/8/8/8/4K3".into();
        setup.turn = Color::Black;
        assert_eq!(setup.fen(), "4k3/8/8/8/8/8/8/4K3 b - - 0 1");
    }

    /// A Chess960 position pasted into the board editor keeps its castling, written as `KQkq`.
    #[test]
    fn a_chess960_positions_castling_is_kept() {
        for pasted in [
            "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1",
            "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w KQkq - 0 1",
        ] {
            let setup = EditorSetup::from_fen(pasted).expect("a position");
            assert_eq!(setup.castling, ALL, "{pasted}");
            assert_eq!(
                setup.fen(),
                "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w KQkq - 0 1"
            );
        }
        // Only the rights written: the long side's rook (F) on its own.
        let setup = EditorSetup::from_fen("bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w F - 0 1");
        assert_eq!(
            setup.expect("a position").castling,
            CastlingRights {
                white_long: true,
                ..CastlingRights::default()
            }
        );
    }

    #[test]
    fn en_passant_is_kept_where_a_pawn_could_take() {
        let setup = EditorSetup::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").expect("a position");
        assert_eq!((setup.turn, setup.ep.as_str()), (Color::White, "d6"));
        assert_eq!(setup.fen(), "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1");
        let mut setup = EditorSetup::from_fen("4k3/8/8/8/8/8/8/4K3 b - - 0 1").expect("a position");
        assert_eq!(setup.ep, "");
        // A square no pawn could just have skipped is left out of the FEN.
        setup.ep = "e3".into();
        assert_eq!(setup.fen(), "4k3/8/8/8/8/8/8/4K3 b - - 0 1");
        let mut rights = CastlingRights::default();
        rights.set("q", true);
        rights.set("x", true);
        assert!(rights.has("q") && !rights.has("K"));
        assert_eq!(EditorSetup::from_fen("not a fen"), None);
    }

    /// Each letter ticks and reads its own right and no other.
    #[test]
    fn each_castling_letter_names_its_own_right() {
        for (letter, right) in [
            (
                "K",
                CastlingRights {
                    white_short: true,
                    ..CastlingRights::default()
                },
            ),
            (
                "Q",
                CastlingRights {
                    white_long: true,
                    ..CastlingRights::default()
                },
            ),
            (
                "k",
                CastlingRights {
                    black_short: true,
                    ..CastlingRights::default()
                },
            ),
            (
                "q",
                CastlingRights {
                    black_long: true,
                    ..CastlingRights::default()
                },
            ),
        ] {
            let mut rights = CastlingRights::default();
            rights.set(letter, true);
            assert_eq!(rights, right, "{letter}");
            for other in ["K", "Q", "k", "q", "x"] {
                assert_eq!(right.has(other), other == letter, "{letter} has {other}");
            }
            let mut all = ALL;
            all.set(letter, false);
            assert!(!all.has(letter), "{letter}");
        }
    }

    /// A FEN written with `_` between its fields, as Lichess writes it in a link, lost its
    /// castling rights and en passant square in the board editor: they were read from fields
    /// split at spaces only.
    #[test]
    fn a_fen_with_underscores_keeps_its_castling_and_en_passant() {
        let start = EditorSetup::from_fen(&crate::rules::INITIAL_FEN.replace(' ', "_"));
        assert_eq!(start, Some(EditorSetup::start()));
        let setup = EditorSetup::from_fen("4k3/8/8/3pP3/8/8/8/4K3_w_-_d6_0_1").expect("a position");
        assert_eq!(setup.ep, "d6");
        let setup = EditorSetup::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d5 0 1").expect("a position");
        assert_eq!(setup.ep, "");
    }
}
