//! The regular expressions the validators use, as hand-written matchers. Each takes the same
//! strings the TypeScript pattern accepts (anchored, JavaScript `\d` and `\w` are ASCII).

use std::sync::LazyLock;

/// `[a-zA-Z0-9]`
fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// `s` is `min..=max` bytes long and every byte satisfies `ok`; the classes used are ASCII, so
/// bytes and characters agree.
fn run(s: &str, min: usize, max: usize, ok: impl Fn(u8) -> bool) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(ok)
}

/// `^[a-zA-Z0-9_-]{2,30}$` (USERNAME)
pub fn username(s: &str) -> bool {
    run(s, 2, 30, |b| alnum(b) || b == b'_' || b == b'-')
}

/// `^[a-zA-Z0-9]{8,12}$` (GAME_ID)
pub fn game_id(s: &str) -> bool {
    run(s, 8, 12, alnum)
}

/// `^[a-zA-Z0-9]{3,12}$` (PUZZLE_ID)
pub fn puzzle_id(s: &str) -> bool {
    run(s, 3, 12, alnum)
}

/// `^[a-zA-Z0-9_-]{1,60}$` (PUZZLE_ANGLE)
pub fn puzzle_angle(s: &str) -> bool {
    run(s, 1, 60, |b| alnum(b) || b == b'_' || b == b'-')
}

/// `^[a-zA-Z0-9]{8}$`: a Lichess id, tournament id or study id.
pub fn lichess_id(s: &str) -> bool {
    run(s, 8, 8, alnum)
}

/// `^[0-9a-f]{16}$`: a review key.
pub fn review_key(s: &str) -> bool {
    run(s, 16, 16, |b| {
        b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
    })
}

/// `^[a-z0-9-]{1,32}$`: board, colour, piece-set and theme identifiers.
pub fn theme_id(s: &str) -> bool {
    run(s, 1, 32, |b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
    })
}

/// `^[a-zA-Z0-9_]{1,30}$`: a key of a run's detail.
pub fn run_detail_key(s: &str) -> bool {
    run(s, 1, 30, |b| alnum(b) || b == b'_')
}

/// `^[a-zA-Z0-9_-]{0,40}$`: a run's variant.
pub fn run_variant(s: &str) -> bool {
    run(s, 0, 40, |b| alnum(b) || b == b'_' || b == b'-')
}

/// `^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$`
pub fn hex_color(s: &str) -> bool {
    let Some(digits) = s.strip_prefix('#') else {
        return false;
    };
    matches!(digits.len(), 3 | 6) && digits.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `^\d{4}(-\d{2})?$`: a year or a year and month.
pub fn since(s: &str) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    match s.len() {
        4 => digits(s),
        7 => digits(&s[..4]) && s.as_bytes()[4] == b'-' && digits(&s[5..]),
        _ => false,
    }
}

/// `^[\w .()+-]{1,80}$`: a file name to export.
pub fn file_name(s: &str) -> bool {
    run(s, 1, 80, |b| {
        alnum(b) || matches!(b, b'_' | b' ' | b'.' | b'(' | b')' | b'+' | b'-')
    })
}

/// `^[a-zA-Z0-9]*$`-style castling and en-passant pieces of the FEN pattern.
fn castling_letter(b: u8) -> bool {
    matches!(b, b'K' | b'Q' | b'k' | b'q' | b'A'..=b'H' | b'a'..=b'h')
}

/// The loose FEN shape of `patterns.ts` (`FEN`): placement, side, castling, en passant and the
/// optional counters.
pub fn fen(s: &str) -> bool {
    let parts: Vec<&str> = s.split(' ').collect();
    if parts.len() != 4 && parts.len() != 6 {
        return false;
    }
    let ranks: Vec<&str> = parts[0].split('/').collect();
    let rank_ok = |r: &str| {
        !r.is_empty()
            && r.bytes().all(|b| {
                matches!(
                    b,
                    b'1'..=b'8'
                        | b'p'
                        | b'n'
                        | b'b'
                        | b'r'
                        | b'q'
                        | b'k'
                        | b'P'
                        | b'N'
                        | b'B'
                        | b'R'
                        | b'Q'
                        | b'K'
                )
            })
    };
    if ranks.len() != 8 || !ranks.iter().all(|r| rank_ok(r)) {
        return false;
    }
    if !matches!(parts[1], "w" | "b") {
        return false;
    }
    let castling = parts[2];
    let castling_ok = castling == "-"
        || (1..=4).contains(&castling.len()) && castling.bytes().all(castling_letter);
    let ep = parts[3].as_bytes();
    let ep_ok = parts[3] == "-"
        || (ep.len() == 2 && (b'a'..=b'h').contains(&ep[0]) && matches!(ep[1], b'3' | b'6'));
    if !castling_ok || !ep_ok {
        return false;
    }
    parts.len() == 4
        || (1..=3).contains(&parts[4].len())
            && parts[4].bytes().all(|b| b.is_ascii_digit())
            && (1..=4).contains(&parts[5].len())
            && parts[5].bytes().all(|b| b.is_ascii_digit())
}

/// `^[a-h][1-8][a-h][1-8][qrbn]?$`: a move in UCI notation.
pub fn uci_move(s: &str) -> bool {
    let b = s.as_bytes();
    if !matches!(b.len(), 4 | 5) {
        return false;
    }
    let square =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    square(b[0], b[1])
        && square(b[2], b[3])
        && (b.len() == 4 || matches!(b[4], b'q' | b'r' | b'b' | b'n'))
}

/// `[\p{L}\p{N}]` ranges, from the Unicode tables `regex-syntax` ships (the same general
/// categories JavaScript's `\p{L}` and `\p{N}` use).
static LETTERS: LazyLock<Vec<(char, char)>> = LazyLock::new(|| general_category('L'));
static NUMBERS: LazyLock<Vec<(char, char)>> = LazyLock::new(|| general_category('N'));

fn general_category(letter: char) -> Vec<(char, char)> {
    use regex_syntax::hir::{Class, HirKind};
    let Ok(hir) = regex_syntax::Parser::new().parse(&format!(r"\p{{{letter}}}")) else {
        return Vec::new();
    };
    match hir.kind() {
        HirKind::Class(Class::Unicode(set)) => set
            .ranges()
            .iter()
            .map(|range| (range.start(), range.end()))
            .collect(),
        _ => Vec::new(),
    }
}

fn in_ranges(ranges: &[(char, char)], c: char) -> bool {
    ranges
        .binary_search_by(|(lo, hi)| {
            if c < *lo {
                std::cmp::Ordering::Greater
            } else if c > *hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `^[\p{L}\p{N} ,.'&()-]*$` (with the `u` flag): a new arena's name.
pub fn arena_name(s: &str) -> bool {
    s.chars().all(|c| {
        matches!(c, ' ' | ',' | '.' | '\'' | '&' | '(' | ')' | '-')
            || in_ranges(&LETTERS, c)
            || in_ranges(&NUMBERS, c)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fen_shapes() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(fen(start));
        assert!(fen("8/8/8/8/8/8/8/8 b - -"));
        assert!(!fen("8/8/8/8/8/8/8 w - -"));
        assert!(!fen("8/8/8/8/8/8/8/8 x - -"));
        assert!(!fen("8/8/8/8/8/8/8/8 w - - 0"));
        assert!(!fen("9/8/8/8/8/8/8/8 w - -"));
        assert!(!fen("8/8/8/8/8/8/8/8 w - e4"));
        assert!(!fen("8/8/8/8/8/8/8/8 w - - 1000 1"));
    }

    #[test]
    fn moves_and_names() {
        assert!(uci_move("e2e4") && uci_move("a7a8q"));
        assert!(!uci_move("e2e9") && !uci_move("e7e8k") && !uci_move("e2e4q1"));
        assert!(arena_name("Éclair (2026) & co.") && arena_name(""));
        assert!(!arena_name("bad;name") && !arena_name("emoji 😀"));
        assert!(since("2026") && since("2026-03") && !since("26") && !since("2026-3"));
    }

    #[test]
    fn usernames_are_bounded() {
        assert!(username("ab") && username(&"a".repeat(30)));
        assert!(!username("a") && !username(&"a".repeat(31)) && !username("a b"));
    }
}
