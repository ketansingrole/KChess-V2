//! Chess rules with chessops semantics on top of shakmaty's move generation.
//!
//! The TypeScript core uses chessops; everything observable here (accepted FENs, UCI
//! normalization, SAN spelling and suffixes, FEN output) matches it so both implementations
//! produce identical documents. Positions are always built in Chess960 castling mode, as
//! chessops represents castling generically as king-takes-rook.

use shakmaty::variant::{Variant, VariantPosition};
use shakmaty::{
    Bitboard, Board, ByColor, ByRole, CastlingMode, CastlingSide, Color, EnPassantMode, File, Move,
    MoveList, Piece, Position, PositionError, Rank, RemainingChecks, Role, Setup, Square, attacks,
};
use std::num::NonZeroU32;

use crate::js;

/// The Lichess variant keys KChess can play (`crates/kchess-wasm/js/variant.ts`).
pub fn lichess_variant(key: &str) -> Option<Variant> {
    Some(match key {
        "standard" | "chess960" => Variant::Chess,
        "kingOfTheHill" => Variant::KingOfTheHill,
        "threeCheck" => Variant::ThreeCheck,
        "antichess" => Variant::Antichess,
        "atomic" => Variant::Atomic,
        "horde" => Variant::Horde,
        "racingKings" => Variant::RacingKings,
        _ => return None,
    })
}

/// chessops `parseVariant`, for the PGN `Variant` header.
pub fn pgn_variant(name: Option<&str>) -> Option<Variant> {
    Some(match name.unwrap_or("chess").to_lowercase().as_str() {
        "chess" | "chess960" | "chess 960" | "standard" | "from position" | "classical"
        | "normal" | "fischerandom" | "fischerrandom" | "fischer random" | "wild/0" | "wild/1"
        | "wild/2" | "wild/3" | "wild/4" | "wild/5" | "wild/6" | "wild/7" | "wild/8"
        | "wild/8a" => Variant::Chess,
        "crazyhouse" | "crazy house" | "house" | "zh" => Variant::Crazyhouse,
        "king of the hill" | "koth" | "kingofthehill" => Variant::KingOfTheHill,
        "three-check" | "three check" | "threecheck" | "three check chess" | "3-check"
        | "3 check" | "3check" => Variant::ThreeCheck,
        "antichess" | "anti chess" | "anti" => Variant::Antichess,
        "atomic" | "atom" | "atomic chess" => Variant::Atomic,
        "horde" | "horde chess" => Variant::Horde,
        "racing kings" | "racingkings" | "racing" | "race" => Variant::RacingKings,
        _ => return None,
    })
}

/* ── FEN (chessops `parseFen` / `makeFen`) ── */

fn small_uint(s: &str) -> Option<u32> {
    (1..=4).contains(&s.len()).then_some(())?;
    s.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| s.parse().ok())?
}

fn char_to_piece(c: char) -> Option<Piece> {
    let role = Role::from_char(c.to_ascii_lowercase())?;
    Some(Piece {
        role,
        color: if c.is_ascii_lowercase() {
            Color::Black
        } else {
            Color::White
        },
    })
}

fn parse_board(part: &str) -> Option<(Board, Bitboard)> {
    let mut board = Board::empty();
    let mut promoted = Bitboard::EMPTY;
    let (mut rank, mut file) = (7i32, 0i32);
    let mut chars = part.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && file == 8 {
            file = 0;
            rank -= 1;
        } else if let Some(step) = c.to_digit(10).filter(|&d| d > 0) {
            file += step as i32;
        } else {
            if file >= 8 || rank < 0 {
                return None;
            }
            let piece = char_to_piece(c)?;
            let square = Square::new((file + rank * 8) as u32);
            board.set_piece_at(square, piece);
            // A promoted marker belongs to the piece before it.
            if chars.next_if_eq(&'~').is_some() {
                promoted.add(square);
            }
            file += 1;
        }
    }
    (rank == 0 && file == 8).then_some((board, promoted))
}

type Pockets = ByColor<ByRole<u8>>;

fn parse_pockets(part: &str) -> Option<Pockets> {
    if js::utf16_len(part) > 64 {
        return None;
    }
    let mut pockets = Pockets::default();
    for c in part.chars() {
        let piece = char_to_piece(c)?;
        *pockets.get_mut(piece.color).get_mut(piece.role) += 1;
    }
    Some(pockets)
}

fn parse_castling(board: &Board, part: &str) -> Option<Bitboard> {
    let mut rights = Bitboard::EMPTY;
    if part == "-" {
        return Some(rights);
    }
    for c in part.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let color = if c == lower {
            Color::Black
        } else {
            Color::White
        };
        let rank = color.backrank();
        if ('a'..='h').contains(&lower) {
            rights.add(Square::from_coords(
                File::new(lower as u32 - 'a' as u32),
                rank,
            ));
        } else if lower == 'k' || lower == 'q' {
            let candidates = board.by_color(color)
                & Bitboard::from_rank(rank)
                & (board.by_role(Role::Rook) | board.by_role(Role::King));
            let candidate = if lower == 'k' {
                candidates.last()
            } else {
                candidates.first()
            };
            rights.add(
                candidate
                    .filter(|&sq| board.by_role(Role::Rook).contains(sq))
                    .unwrap_or_else(|| {
                        Square::from_coords(File::new(if lower == 'k' { 7 } else { 0 }), rank)
                    }),
            );
        } else {
            return None;
        }
    }
    let too_many = |color: Color| (rights & Bitboard::from_rank(color.backrank())).count() > 2;
    (!too_many(Color::White) && !too_many(Color::Black)).then_some(rights)
}

fn parse_remaining_checks(part: &str) -> Option<ByColor<RemainingChecks>> {
    let parts: Vec<&str> = part.split('+').collect();
    let (white, black, remaining) = match parts.as_slice() {
        ["", w, b] => (small_uint(w)?, small_uint(b)?, false),
        [w, b] => (small_uint(w)?, small_uint(b)?, true),
        _ => return None,
    };
    if white > 3 || black > 3 {
        return None;
    }
    let (white, black) = if remaining {
        (white, black)
    } else {
        (3 - white, 3 - black)
    };
    Some(ByColor {
        white: RemainingChecks::new(white),
        black: RemainingChecks::new(black),
    })
}

/// chessops `parseFen`: lenient about missing fields, strict about everything present.
pub fn parse_fen(fen: &str) -> Option<Setup> {
    let mut parts = split_fen(fen).into_iter();
    let board_part = parts.next().unwrap_or("");
    let ((board, promoted), pockets) = if let Some(stripped) = board_part.strip_suffix(']') {
        let start = stripped.find('[')?;
        let pockets = parse_pockets(&stripped[start + 1..])?;
        (parse_board(&stripped[..start])?, Some(pockets))
    } else {
        match board_part.match_indices('/').nth(7) {
            None => (parse_board(board_part)?, None),
            Some((index, _)) => {
                let pockets = parse_pockets(&board_part[index + 1..])?;
                (parse_board(&board_part[..index])?, Some(pockets))
            }
        }
    };
    let turn = match parts.next() {
        None | Some("w") => Color::White,
        Some("b") => Color::Black,
        Some(_) => return None,
    };
    let castling_rights = match parts.next() {
        Some(part) => parse_castling(&board, part)?,
        None => Bitboard::EMPTY,
    };
    let ep_square = match parts.next() {
        Some(part) if part != "-" => Some(parse_square(part)?),
        _ => None,
    };
    let mut halfmove_part = parts.next();
    let mut early_checks = None;
    if let Some(part) = halfmove_part.filter(|p| p.contains('+')) {
        early_checks = Some(parse_remaining_checks(part));
        halfmove_part = parts.next();
    }
    let halfmoves = match halfmove_part {
        Some(part) => small_uint(part)?,
        None => 0,
    };
    let fullmoves = match parts.next() {
        Some(part) => small_uint(part)?,
        None => 1,
    };
    let remaining_checks = match (parts.next(), early_checks) {
        (Some(_), Some(_)) => return None,
        (Some(part), None) => Some(parse_remaining_checks(part)?),
        (None, Some(early)) => Some(early?),
        (None, None) => None,
    };
    if parts.next().is_some() {
        return None;
    }
    let mut setup = Setup::empty();
    setup.board = board;
    setup.promoted = promoted;
    setup.pockets = pockets;
    setup.turn = turn;
    setup.castling_rights = castling_rights;
    setup.ep_square = ep_square;
    setup.remaining_checks = remaining_checks;
    setup.halfmoves = halfmoves;
    setup.fullmoves = NonZeroU32::new(fullmoves.max(1)).unwrap_or(NonZeroU32::MIN);
    Some(setup)
}

/// `fen.split(/[\s_]+/)`
fn split_fen(fen: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_sep = false;
    for (i, c) in fen.char_indices() {
        let sep = js::is_space(c) || c == '_';
        if sep && !in_sep {
            parts.push(&fen[start..i]);
        }
        if !sep && in_sep {
            start = i;
        }
        in_sep = sep;
    }
    parts.push(if in_sep { "" } else { &fen[start..] });
    parts
}

/// chessops `parseSquare`: exactly two characters, lowercase file.
pub fn parse_square(s: &str) -> Option<Square> {
    let b = s.as_bytes();
    if s.chars().count() != 2 || b.len() != 2 {
        return None;
    }
    let file = b[0].wrapping_sub(b'a');
    let rank = b[1].wrapping_sub(b'1');
    (file < 8 && rank < 8)
        .then(|| Square::from_coords(File::new(file.into()), Rank::new(rank.into())))
}

/// chessops `setupPosition` under the given rules: the position, or None when not legal.
pub fn setup_position(variant: Variant, setup: Setup) -> Option<VariantPosition> {
    let ep = setup.ep_square;
    let mut setup = setup;
    // chessops silently drops an en passant square that no pawn could have just passed.
    setup.ep_square = ep.filter(|&sq| valid_ep_square(&setup, sq));
    if variant != Variant::ThreeCheck {
        setup.remaining_checks = None;
    }
    if variant == Variant::Crazyhouse {
        // chessops keeps promoted markers only on pieces that can have been promoted.
        let board = &setup.board;
        setup.promoted = setup.promoted
            & board.occupied()
            & !board.by_role(Role::King)
            & !board.by_role(Role::Pawn);
        setup.pockets.get_or_insert_with(Pockets::default);
    } else {
        setup.promoted = Bitboard::EMPTY;
        setup.pockets = None;
    }
    VariantPosition::from_setup(variant, setup, CastlingMode::Chess960)
        .or_else(PositionError::ignore_invalid_castling_rights)
        .or_else(PositionError::ignore_invalid_ep_square)
        .or_else(PositionError::ignore_too_much_material)
        .or_else(PositionError::ignore_impossible_check)
        .ok()
        .filter(chessops_valid)
}

/// chessops `validate()` for each variant, which shakmaty does not always share.
fn chessops_valid(pos: &VariantPosition) -> bool {
    let board = pos.board();
    let turn = pos.turn();
    let kings = board.by_role(Role::King);
    let pawns = board.by_role(Role::Pawn);
    let opposite_check = |king: Square| pos.king_attackers(king, turn, board.occupied()).any();
    let standard = || {
        board.occupied().any()
            && kings.count() == 2
            && board.king_of(turn).is_some()
            && board
                .king_of(!turn)
                .is_some_and(|king| !opposite_check(king))
            && !pawns.intersects(backranks())
    };
    match pos {
        VariantPosition::Chess(_)
        | VariantPosition::KingOfTheHill(_)
        | VariantPosition::ThreeCheck(_) => standard(),
        VariantPosition::Crazyhouse(_) => {
            standard()
                && pos.pockets().is_none_or(|pockets| {
                    let total: usize = Color::ALL
                        .iter()
                        .flat_map(|&c| {
                            Role::ALL
                                .iter()
                                .map(move |&r| usize::from(*pockets.get(c).get(r)))
                        })
                        .sum();
                    *pockets.white.get(Role::King) == 0
                        && *pockets.black.get(Role::King) == 0
                        && total + board.occupied().count() <= 64
                })
        }
        VariantPosition::Atomic(_) => {
            board.occupied().any()
                && kings.count() <= 2
                && board
                    .king_of(!turn)
                    .is_some_and(|king| !opposite_check(king))
                && !pawns.intersects(backranks())
        }
        VariantPosition::Antichess(_) => board.occupied().any() && !pawns.intersects(backranks()),
        VariantPosition::RacingKings(_) => !pos.is_check() && pawns.is_empty() && standard(),
        VariantPosition::Horde(_) => {
            board.occupied().any()
                && kings.count() == 1
                && board
                    .king_of(!turn)
                    .is_none_or(|king| !opposite_check(king))
                && Color::ALL.iter().all(|&color| {
                    let ranks = if (board.by_color(color) & kings).is_empty() {
                        Bitboard::from_rank((!color).backrank())
                    } else {
                        backranks()
                    };
                    !(board.by_color(color) & pawns).intersects(ranks)
                })
        }
    }
}

fn valid_ep_square(setup: &Setup, square: Square) -> bool {
    let (ep_rank, forward) = match setup.turn {
        Color::White => (Rank::Sixth, 8),
        Color::Black => (Rank::Third, -8),
    };
    if square.rank() != ep_rank {
        return false;
    }
    let ahead = square.offset(forward);
    let pawn = square.offset(-forward);
    match (ahead, pawn) {
        (Some(ahead), Some(pawn)) => {
            !setup.board.occupied().contains(ahead)
                && setup.board.piece_at(pawn)
                    == Some(Piece {
                        role: Role::Pawn,
                        color: !setup.turn,
                    })
        }
        _ => false,
    }
}

/// A FEN text as a position under a Lichess variant key (`setupStart`).
pub fn setup_start(variant: &str, fen: &str) -> Option<VariantPosition> {
    setup_position(lichess_variant(variant)?, parse_fen(fen)?)
}

/// chessops `Chess.fromSetup(parseFen(fen))`: standard rules.
pub fn standard_from_fen(fen: &str) -> Option<VariantPosition> {
    setup_position(Variant::Chess, parse_fen(fen)?)
}

/// chessops `makeFen(pos.toSetup())`: the legal en passant square and at most 150 halfmoves.
pub fn make_fen(pos: &VariantPosition) -> String {
    write_fen(&FenParts {
        board: pos.board(),
        promoted: pos.promoted(),
        pockets: pos.pockets(),
        turn: pos.turn(),
        castling_rights: pos.castles().castling_rights(),
        ep_square: pos.ep_square(EnPassantMode::Legal),
        remaining_checks: pos.remaining_checks(),
        halfmoves: pos.halfmoves().min(150),
        fullmoves: pos.fullmoves().get(),
    })
}

/// chessops `makeFen(setup)` for a parsed, not yet validated, setup: everything as written.
pub fn make_setup_fen(setup: &Setup) -> String {
    write_fen(&FenParts {
        board: &setup.board,
        promoted: setup.promoted,
        pockets: setup.pockets.as_ref(),
        turn: setup.turn,
        castling_rights: setup.castling_rights,
        ep_square: setup.ep_square,
        remaining_checks: setup.remaining_checks.as_ref(),
        halfmoves: setup.halfmoves,
        fullmoves: setup.fullmoves.get(),
    })
}

struct FenParts<'a> {
    board: &'a Board,
    promoted: Bitboard,
    pockets: Option<&'a Pockets>,
    turn: Color,
    castling_rights: Bitboard,
    ep_square: Option<Square>,
    remaining_checks: Option<&'a ByColor<RemainingChecks>>,
    halfmoves: u32,
    fullmoves: u32,
}

fn write_fen(parts: &FenParts) -> String {
    let board = parts.board;
    let mut fen = match board.board_fen_with_promoted(parts.promoted) {
        Ok(fen) => fen.to_string(),
        Err(lossy) => lossy.ignore().to_string(),
    };
    if let Some(pockets) = parts.pockets {
        fen.push('[');
        for color in Color::ALL {
            for role in Role::ALL {
                for _ in 0..*pockets.get(color).get(role) {
                    fen.push(if color.is_white() {
                        role.upper_char()
                    } else {
                        role.char()
                    });
                }
            }
        }
        fen.push(']');
    }
    fen.push(' ');
    fen.push(parts.turn.char());
    fen.push(' ');
    let before = fen.len();
    for color in Color::ALL {
        let backrank = Bitboard::from_rank(color.backrank());
        let king = board.king_of(color).filter(|&k| backrank.contains(k));
        let candidates = board.by_piece(Piece {
            color,
            role: Role::Rook,
        }) & backrank;
        for rook in (parts.castling_rights & backrank).into_iter().rev() {
            let c = match king {
                Some(k) if Some(rook) == candidates.first() && rook < k => 'q',
                Some(k) if Some(rook) == candidates.last() && k < rook => 'k',
                _ => rook.file().char(),
            };
            fen.push(if color.is_white() {
                c.to_ascii_uppercase()
            } else {
                c
            });
        }
    }
    if fen.len() == before {
        fen.push('-');
    }
    fen.push(' ');
    match parts.ep_square {
        Some(sq) => fen.push_str(&sq.to_string()),
        None => fen.push('-'),
    }
    if let Some(checks) = parts.remaining_checks {
        fen.push_str(&format!(
            " {}+{}",
            u32::from(checks.white),
            u32::from(checks.black)
        ));
    }
    let halfmoves = parts.halfmoves.min(9999);
    let fullmoves = parts.fullmoves.clamp(1, 9999);
    fen.push_str(&format!(" {halfmoves} {fullmoves}"));
    fen
}

/* ── Moves (chessops `parseUci`, `normalizeMove`, `isLegal`, SAN) ── */

/// A move as chessops writes it: castling is king-to-rook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UciMove {
    Normal {
        from: Square,
        to: Square,
        promotion: Option<Role>,
    },
    Drop {
        role: Role,
        to: Square,
    },
}

/// chessops `parseUci`.
pub fn parse_uci(s: &str) -> Option<UciMove> {
    let chars: Vec<char> = s.chars().collect();
    let len = utf16_len_chars(&chars);
    if chars.get(1) == Some(&'@') && len == 4 {
        let role = Role::from_char(chars[0].to_ascii_lowercase())?;
        let to = parse_square(&s[chars[0].len_utf8() + 1..])?;
        return Some(UciMove::Drop { role, to });
    }
    if (len != 4 && len != 5) || !s.is_ascii() {
        return None;
    }
    let from = parse_square(&s[0..2])?;
    let to = parse_square(&s[2..4])?;
    let promotion = if len == 5 {
        Some(Role::from_char(chars[4].to_ascii_lowercase())?)
    } else {
        None
    };
    Some(UciMove::Normal {
        from,
        to,
        promotion,
    })
}

fn utf16_len_chars(chars: &[char]) -> usize {
    chars.iter().map(|c| c.len_utf16()).sum()
}

/// Where chessops's `dests` puts a legal move: castling lands on the rook.
pub(crate) fn dest(m: &Move) -> Option<Square> {
    match *m {
        Move::Castle { rook, .. } => Some(rook),
        Move::Put { .. } => None,
        _ => Some(m.to()),
    }
}

/// chessops `makeUci` for a legal move.
pub fn make_uci(m: &Move) -> String {
    let mut s = String::with_capacity(5);
    if let Move::Put { role, to } = *m {
        s.push(role.upper_char());
        s.push('@');
        s.push_str(&to.to_string());
        return s;
    }
    if let (Some(from), Some(to)) = (m.from(), dest(m)) {
        s.push_str(&from.to_string());
        s.push_str(&to.to_string());
    }
    if let Some(role) = m.promotion() {
        s.push(role.char());
    }
    s
}

fn backranks() -> Bitboard {
    Bitboard::from_rank(Rank::First) | Bitboard::from_rank(Rank::Eighth)
}

/// The legal moves that chessops's `dests` would list as reaching `to` with a piece of `role`:
/// moves and drops onto `to`, plus castling with the rook on `to` for the king.
/// Generating only these is much cheaper than every legal move.
fn reaching(pos: &VariantPosition, role: Role, to: Square) -> MoveList {
    let mut moves = pos.san_candidates(role, to);
    if role == Role::King {
        for side in [CastlingSide::KingSide, CastlingSide::QueenSide] {
            if pos.castles().rook(pos.turn(), side) == Some(to) {
                moves.extend(pos.castling_moves(side));
            }
        }
    }
    moves
}

/// The legal move chessops would play for `normalizeMove(pos, uci)` when `isLegal` accepts it.
pub fn legal_move(pos: &VariantPosition, uci: UciMove) -> Option<Move> {
    let (from, to, promotion) = match uci {
        UciMove::Drop { role, to } => return legal_drop(pos, role, to),
        UciMove::Normal {
            from,
            to,
            promotion,
        } => (from, to, promotion),
    };
    let board = pos.board();
    match promotion {
        Some(Role::Pawn) => return None,
        Some(Role::King) if !matches!(pos, VariantPosition::Antichess(_)) => return None,
        _ => {}
    }
    let promotes = board.by_role(Role::Pawn).contains(from) && backranks().contains(to);
    if promotion.is_some() != promotes {
        return None;
    }
    let role = board.role_at(from).filter(|_| pos.us().contains(from))?;
    // castlingSide/normalizeMove: a king moving two files or onto its own piece castles.
    let delta = to as i32 - from as i32;
    if role == Role::King && (delta.abs() == 2 || pos.us().contains(to)) {
        let side = if delta > 0 {
            CastlingSide::KingSide
        } else {
            CastlingSide::QueenSide
        };
        let rook = pos.castles().rook(pos.turn(), side)?;
        return pos
            .castling_moves(side)
            .into_iter()
            .find(|m| m.from() == Some(from) && dest(m) == Some(rook));
    }
    pos.san_candidates(role, to)
        .into_iter()
        .find(|m| m.from() == Some(from) && m.promotion() == promotion)
}

/// chessops `isLegal` for a drop: in the pocket, no pawn on a back rank, an allowed square.
fn legal_drop(pos: &VariantPosition, role: Role, to: Square) -> Option<Move> {
    let pockets = pos.pockets()?;
    if *pockets.get(pos.turn()).get(role) == 0 || (role == Role::Pawn && backranks().contains(to)) {
        return None;
    }
    pos.san_candidates(role, to)
        .into_iter()
        .find(|m| *m == Move::Put { role, to })
}

/// chessops `makeSanWithoutSuffix`.
fn san_without_suffix(pos: &VariantPosition, m: &Move) -> String {
    let board = pos.board();
    if let Move::Put { role, to } = *m {
        let mut san = String::with_capacity(4);
        if role != Role::Pawn {
            san.push(role.upper_char());
        }
        san.push('@');
        san.push_str(&to.to_string());
        return san;
    }
    let (Some(from), Some(to)) = (m.from(), dest(m)) else {
        return "--".into();
    };
    let Some(role) = board.role_at(from) else {
        return "--".into();
    };
    if role == Role::King
        && (board.by_color(pos.turn()).contains(to) || (to as i32 - from as i32).abs() == 2)
    {
        return if to > from {
            "O-O".into()
        } else {
            "O-O-O".into()
        };
    }
    let mut san = String::with_capacity(7);
    let capture = board.occupied().contains(to) || (role == Role::Pawn && from.file() != to.file());
    if role != Role::Pawn {
        san.push(role.upper_char());
        let occupied = board.occupied();
        let attackers = match role {
            Role::King => attacks::king_attacks(to),
            Role::Queen => attacks::queen_attacks(to, occupied),
            Role::Rook => attacks::rook_attacks(to, occupied),
            Role::Bishop => attacks::bishop_attacks(to, occupied),
            _ => attacks::knight_attacks(to),
        };
        let mut others = attackers & board.by_role(role) & board.by_color(pos.turn());
        others.discard(from);
        if others.any() {
            let reach = reaching(pos, role, to);
            others = others
                .into_iter()
                .filter(|&sq| {
                    reach
                        .iter()
                        .any(|m| m.from() == Some(sq) && dest(m) == Some(to))
                })
                .collect();
        }
        if others.any() {
            let mut column = others.intersects(Bitboard::from_rank(from.rank()));
            let row = others.intersects(Bitboard::from_file(from.file()));
            if !row {
                column = true;
            }
            if column {
                san.push(from.file().char());
            }
            if row {
                san.push(from.rank().char());
            }
        }
    } else if capture {
        san.push(from.file().char());
    }
    if capture {
        san.push('x');
    }
    san.push_str(&to.to_string());
    if let Some(promotion) = m.promotion() {
        san.push('=');
        san.push(promotion.upper_char());
    }
    san
}

/// chessops `makeSanAndPlay`: plays the move and returns its SAN with `+`/`#`.
pub fn san_and_play(pos: &mut VariantPosition, m: Move) -> String {
    let mut san = san_without_suffix(pos, &m);
    pos.play_unchecked(m);
    let variant = pos.variant_outcome();
    if variant.is_known() {
        if variant.winner().is_some() {
            san.push('#');
        } else if pos.is_check() {
            san.push('+');
        }
    } else if pos.is_check() {
        san.push(if pos.legal_moves().is_empty() {
            '#'
        } else {
            '+'
        });
    }
    san
}

/// chessops `parseSan`.
pub fn parse_san(pos: &VariantPosition, san: &str) -> Option<Move> {
    let Some(parsed) = SanPattern::parse(san) else {
        let side = match san {
            "O-O" | "O-O+" | "O-O#" => CastlingSide::KingSide,
            "O-O-O" | "O-O-O+" | "O-O-O#" => CastlingSide::QueenSide,
            _ => return parse_drop_san(pos, san),
        };
        let king = pos.board().king_of(pos.turn())?;
        let rook = pos.castles().rook(pos.turn(), side)?;
        return pos
            .castling_moves(side)
            .into_iter()
            .find(|m| m.from() == Some(king) && dest(m) == Some(rook));
    };
    let promotes = parsed.role == Role::Pawn && backranks().contains(parsed.to);
    if parsed.promotion.is_some() != promotes {
        return None;
    }
    if parsed.promotion == Some(Role::King) && !matches!(pos, VariantPosition::Antichess(_)) {
        return None;
    }
    let board = pos.board();
    let mut candidates = board.by_piece(Piece {
        color: pos.turn(),
        role: parsed.role,
    });
    match parsed.file {
        None if parsed.role == Role::Pawn => candidates &= Bitboard::from_file(parsed.to.file()),
        Some(file) => candidates &= Bitboard::from_file(file),
        None => {}
    }
    if let Some(rank) = parsed.rank {
        candidates &= Bitboard::from_rank(rank);
    }
    let pawn_advance = if parsed.role == Role::Pawn {
        Bitboard::from_file(parsed.to.file())
    } else {
        Bitboard::EMPTY
    };
    let reach = attacks::attacks(
        parsed.to,
        Piece {
            color: !pos.turn(),
            role: parsed.role,
        },
        board.occupied(),
    );
    candidates &= pawn_advance | reach;
    if candidates.is_empty() {
        return None;
    }
    let moves = reaching(pos, parsed.role, parsed.to);
    let mut found: Option<Square> = None;
    for candidate in candidates {
        if moves
            .iter()
            .any(|m| m.from() == Some(candidate) && dest(m) == Some(parsed.to))
        {
            if found.is_some() {
                return None; // ambiguous
            }
            found = Some(candidate);
        }
    }
    let from = found?;
    moves.into_iter().find(|m| {
        m.from() == Some(from) && dest(m) == Some(parsed.to) && m.promotion() == parsed.promotion
    })
}

/// `/^([pnbrqkPNBRQK])?@([a-h][1-8])[+#]?$/`
fn parse_drop_san(pos: &VariantPosition, san: &str) -> Option<Move> {
    let body = san.strip_suffix(['+', '#']).unwrap_or(san);
    let (role, square) = body.split_once('@')?;
    let role = match role.chars().collect::<Vec<_>>().as_slice() {
        [] => Role::Pawn,
        [c] if "pnbrqkPNBRQK".contains(*c) => Role::from_char(c.to_ascii_lowercase())?,
        _ => return None,
    };
    legal_drop(pos, role, parse_square(square)?)
}

/// `/^([NBRQK])?([a-h])?([1-8])?[-x]?([a-h][1-8])(?:=?([nbrqkNBRQK]))?[+#]?$/`
struct SanPattern {
    role: Role,
    file: Option<File>,
    rank: Option<Rank>,
    to: Square,
    promotion: Option<Role>,
}

impl SanPattern {
    fn parse(san: &str) -> Option<SanPattern> {
        let b = san.as_bytes();
        if !san.is_ascii() {
            return None;
        }
        let mut i = 0;
        let role = match b.first() {
            Some(b'N') => Some(Role::Knight),
            Some(b'B') => Some(Role::Bishop),
            Some(b'R') => Some(Role::Rook),
            Some(b'Q') => Some(Role::Queen),
            Some(b'K') => Some(Role::King),
            _ => None,
        };
        if role.is_some() {
            i += 1;
        }
        // Optional disambiguation, then the destination: try the longest reading first,
        // matching the regex's greedy-with-backtracking behaviour.
        let is_file = |c: u8| (b'a'..=b'h').contains(&c);
        let is_rank = |c: u8| (b'1'..=b'8').contains(&c);
        for take_file in [true, false] {
            for take_rank in [true, false] {
                let mut j = i;
                let file = if take_file {
                    match b.get(j) {
                        Some(&c) if is_file(c) => {
                            j += 1;
                            Some(File::new(u32::from(c - b'a')))
                        }
                        _ => continue,
                    }
                } else {
                    None
                };
                let rank = if take_rank {
                    match b.get(j) {
                        Some(&c) if is_rank(c) => {
                            j += 1;
                            Some(Rank::new(u32::from(c - b'1')))
                        }
                        _ => continue,
                    }
                } else {
                    None
                };
                if matches!(b.get(j), Some(b'-' | b'x')) {
                    // `[-x]?` is optional; try with it consumed, then without.
                    if let Some(p) = Self::tail(b, j + 1, role, file, rank) {
                        return Some(p);
                    }
                }
                if let Some(p) = Self::tail(b, j, role, file, rank) {
                    return Some(p);
                }
            }
        }
        None
    }

    fn tail(
        b: &[u8],
        mut j: usize,
        role: Option<Role>,
        file: Option<File>,
        rank: Option<Rank>,
    ) -> Option<SanPattern> {
        let (&f, &r) = (b.get(j)?, b.get(j + 1)?);
        if !(b'a'..=b'h').contains(&f) || !(b'1'..=b'8').contains(&r) {
            return None;
        }
        let to = Square::from_coords(
            File::new(u32::from(f - b'a')),
            Rank::new(u32::from(r - b'1')),
        );
        j += 2;
        let mut promotion = None;
        let promo =
            |c: u8| Role::from_char((c as char).to_ascii_lowercase()).filter(|r| *r != Role::Pawn);
        if b.get(j) == Some(&b'=') {
            if let Some(p) = b.get(j + 1).and_then(|&c| promo(c)) {
                promotion = Some(p);
                j += 2;
            }
        } else if let Some(p) = b.get(j).and_then(|&c| promo(c)) {
            promotion = Some(p);
            j += 1;
        }
        if matches!(b.get(j), Some(b'+' | b'#')) {
            j += 1;
        }
        (j == b.len()).then_some(SanPattern {
            role: role.unwrap_or(Role::Pawn),
            file,
            rank,
            to,
            promotion,
        })
    }
}

pub fn variant_of(pos: &VariantPosition) -> Variant {
    pos.variant()
}

pub fn turn_name(color: Color) -> &'static str {
    if color.is_white() { "white" } else { "black" }
}

pub fn default_position(variant: Variant) -> VariantPosition {
    VariantPosition::new(variant)
}
