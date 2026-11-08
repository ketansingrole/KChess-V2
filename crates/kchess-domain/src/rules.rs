//! Chess rules on top of shakmaty's move generation.
//!
//! Everything observable here (accepted FENs, UCI normalization, SAN spelling and suffixes, FEN
//! output) keeps the semantics earlier releases had, so stored documents stay identical.
//! Positions are always built in Chess960 castling mode, as castling is represented generically
//! as king-takes-rook.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shakmaty::variant::{Variant, VariantPosition};
use shakmaty::{
    Bitboard, Board, ByColor, ByRole, CastlingMode, CastlingSide, Color, EnPassantMode, File, Move,
    MoveList, Piece, Position, PositionError, Rank, RemainingChecks, Role, Setup, Square, attacks,
};
use std::fmt::Write as _;
use std::num::NonZeroU32;

use crate::js;

/// The standard start.
pub const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// `GameSetup`: the rules and the position before the first move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameSetup {
    pub variant: String,
    pub fen: String,
}

impl GameSetup {
    /// The ordinary game from the usual start.
    pub fn standard() -> GameSetup {
        GameSetup {
            variant: "standard".into(),
            fen: STANDARD_FEN.into(),
        }
    }

    pub(crate) fn json(&self) -> Value {
        json!({ "variant": self.variant, "fen": self.fen })
    }

    /// `isStandardStart`: the ordinary game, from the usual start.
    pub(crate) fn is_standard_start(&self) -> bool {
        self.variant == "standard" && self.fen == STANDARD_FEN
    }
}

/// The Lichess variant keys KChess can play.
pub fn lichess_variant(key: &str) -> Option<Variant> {
    crate::variant::info(key).map(|variant| variant.rules)
}

/// `parseVariant`, for the PGN `Variant` header.
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

/* ── FEN (`parseFen` / `makeFen`) ── */

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

/// `parseFen`: lenient about missing fields, strict about everything present.
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

/// `parseSquare`: exactly two characters, lowercase file.
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

/// `setupPosition` under the given rules: the position, or None when not legal.
pub fn setup_position(variant: Variant, setup: Setup) -> Option<VariantPosition> {
    let ep = setup.ep_square;
    let mut setup = setup;
    // An en passant square that no pawn could have just passed is silently dropped.
    setup.ep_square = ep.filter(|&sq| valid_ep_square(&setup, sq));
    if variant != Variant::ThreeCheck {
        setup.remaining_checks = None;
    }
    if variant == Variant::Crazyhouse {
        // Promoted markers are kept only on pieces that can have been promoted.
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

/// The `validate()` rules of each variant, which shakmaty does not always share.
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

/// `Chess.fromSetup(parseFen(fen))`: standard rules.
pub fn standard_from_fen(fen: &str) -> Option<VariantPosition> {
    setup_position(Variant::Chess, parse_fen(fen)?)
}

/// `makeFen(pos.toSetup())`: the legal en passant square and at most 150 halfmoves.
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

/// `makeFen(setup)` for a parsed, not yet validated, setup: everything as written.
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
        Some(sq) => {
            fen.push(sq.file().char());
            fen.push(sq.rank().char());
        }
        None => fen.push('-'),
    }
    if let Some(checks) = parts.remaining_checks {
        // Formatting straight into the FEN avoids a temporary String on a path every move takes.
        let _ = write!(
            fen,
            " {}+{}",
            u32::from(checks.white),
            u32::from(checks.black)
        );
    }
    let halfmoves = parts.halfmoves.min(9999);
    let fullmoves = parts.fullmoves.clamp(1, 9999);
    let _ = write!(fen, " {halfmoves} {fullmoves}");
    fen
}

/* ── Moves (`parseUci`, `normalizeMove`, `isLegal`, SAN) ── */

/// A move in KChess's form: castling is king-to-rook.
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

/// `parseUci`.
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

/// Where `dests` puts a legal move: castling lands on the rook.
pub(crate) fn dest(m: &Move) -> Option<Square> {
    match *m {
        Move::Castle { rook, .. } => Some(rook),
        Move::Put { .. } => None,
        _ => Some(m.to()),
    }
}

/// `makeUci` for a legal move.
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

/// The legal moves that `dests` would list as reaching `to` with a piece of `role`:
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

/// The legal move to play for `normalizeMove(pos, uci)` when `isLegal` accepts it.
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

/// `isLegal` for a drop: in the pocket, no pawn on a back rank, an allowed square.
fn legal_drop(pos: &VariantPosition, role: Role, to: Square) -> Option<Move> {
    let pockets = pos.pockets()?;
    if *pockets.get(pos.turn()).get(role) == 0 || (role == Role::Pawn && backranks().contains(to)) {
        return None;
    }
    pos.san_candidates(role, to)
        .into_iter()
        .find(|m| *m == Move::Put { role, to })
}

/// `makeSanWithoutSuffix`, except that a pawn drop names its pawn (`P@e4`), as Lichess writes it.
fn san_without_suffix(pos: &VariantPosition, m: &Move) -> String {
    let board = pos.board();
    if let Move::Put { role, to } = *m {
        let mut san = String::with_capacity(4);
        san.push(role.upper_char());
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

/// `makeSanAndPlay`: plays the move and returns its SAN with `+`/`#`.
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

/// `parseSan`.
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

/// The Chess960 starting position numbered `n` in the Scharnagl scheme (any whole number, taken
/// modulo 960; 518 is the standard start). Castling rights use X-FEN letters so every engine and
/// Lichess can read it.
pub fn chess960_fen(n: i64) -> String {
    let mut rest = n.rem_euclid(960) as usize;
    let mut rank: [Option<char>; 8] = [None; 8];
    let light = rest % 4;
    rest /= 4;
    rank[light * 2 + 1] = Some('b');
    let dark = rest % 4;
    rest /= 4;
    rank[dark * 2] = Some('b');
    let free = |rank: &[Option<char>; 8]| -> Vec<usize> {
        (0..8).filter(|&square| rank[square].is_none()).collect()
    };
    let queen = rest % 6;
    rest /= 6;
    let queen_square = free(&rank)[queen];
    rank[queen_square] = Some('q');
    const KNIGHTS: [(usize, usize); 10] = [
        (0, 1),
        (0, 2),
        (0, 3),
        (0, 4),
        (1, 2),
        (1, 3),
        (1, 4),
        (2, 3),
        (2, 4),
        (3, 4),
    ];
    let (first, second) = KNIGHTS[rest];
    let open = free(&rank);
    rank[open[first]] = Some('n');
    rank[open[second]] = Some('n');
    let last = free(&rank);
    rank[last[0]] = Some('r');
    rank[last[1]] = Some('k');
    rank[last[2]] = Some('r');
    let black: String = rank.iter().map(|piece| piece.unwrap_or('r')).collect();
    format!(
        "{black}/pppppppp/8/8/8/8/PPPPPPPP/{} w KQkq - 0 1",
        black.to_uppercase()
    )
}

#[cfg(test)]
mod chess960_tests {
    use super::chess960_fen;

    #[test]
    fn the_scharnagl_numbers_give_the_known_starts() {
        assert_eq!(
            chess960_fen(518),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        assert!(chess960_fen(0).starts_with("bbqnnrkr/"));
        assert_eq!(chess960_fen(959), chess960_fen(-1));
        assert_eq!(chess960_fen(960), chess960_fen(0));
    }

    #[test]
    fn every_start_has_bishops_on_both_colors_and_the_king_between_the_rooks() {
        let mut seen = std::collections::HashSet::new();
        for n in 0..960 {
            let fen = chess960_fen(n);
            let back = &fen[..8];
            assert!(seen.insert(back.to_string()), "{n} repeats {back}");
            let at = |c: char| back.match_indices(c).map(|(i, _)| i).collect::<Vec<_>>();
            let bishops = at('b');
            assert_eq!(bishops.len(), 2);
            assert_ne!(bishops[0] % 2, bishops[1] % 2, "{n}");
            let rooks = at('r');
            let king = at('k')[0];
            assert!(rooks[0] < king && king < rooks[1], "{n}");
        }
    }
}

#[cfg(test)]
mod drop_tests {
    use super::{parse_san, san_and_play};
    use shakmaty::fen::Fen;
    use shakmaty::variant::{Variant, VariantPosition};
    use shakmaty::{CastlingMode, Move, Role, Square};

    fn crazyhouse(fen: &str) -> VariantPosition {
        let setup = fen.parse::<Fen>().expect("a FEN").into_setup();
        VariantPosition::from_setup(Variant::Crazyhouse, setup, CastlingMode::Standard)
            .expect("a legal position")
    }

    /// Drops are written as Lichess writes them, pawns included; `@e4` from older games still reads.
    #[test]
    fn a_drop_names_its_piece_and_either_pawn_form_reads() {
        // 1. e4 d5 2. exd5 Qxd5: a pawn each in hand.
        let start = crazyhouse("rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3");
        // A knight each in hand instead.
        let knights = crazyhouse("r1bqkbnr/pppppppp/8/8/8/8/PPPPPPPP/R1BQKBNR[Nn] w KQkq - 0 1");
        let pawn = Move::Put {
            role: Role::Pawn,
            to: Square::E4,
        };
        let knight = Move::Put {
            role: Role::Knight,
            to: Square::E4,
        };
        assert_eq!(san_and_play(&mut start.clone(), pawn), "P@e4");
        assert_eq!(san_and_play(&mut knights.clone(), knight), "N@e4");
        for written in ["P@e4", "@e4", "p@e4"] {
            assert_eq!(parse_san(&start, written), Some(pawn), "{written}");
        }
    }
}

#[cfg(test)]
mod fen_tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    /// A FEN's board is eight ranks of eight files; a `0` is no count of empty squares.
    #[test]
    fn a_board_is_eight_full_ranks() {
        assert!(parse_fen(START).is_some());
        assert!(parse_fen("rnbqkbnr/pppppppp/80/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").is_none());
        assert!(parse_fen("rnbqkbnr/pppppppp/7/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").is_none());
        assert!(parse_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP w KQkq - 0 1").is_none());
    }

    /// A pocket is written in brackets or, as some sites write it, after a ninth `/`.
    #[test]
    fn a_pocket_reads_in_either_form() {
        let board = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        for fen in [
            format!("{board}[PPn] w - - 0 1"),
            format!("{board}/PPn w - - 0 1"),
        ] {
            let pockets = parse_fen(&fen).unwrap().pockets.unwrap();
            assert_eq!(pockets.white.pawn, 2, "{fen}");
            assert_eq!(pockets.black.knight, 1, "{fen}");
        }
    }

    /// A square is a lowercase file `a`–`h` and a rank `1`–`8`.
    #[test]
    fn a_square_is_on_the_board() {
        assert_eq!(parse_square("a1"), Some(Square::A1));
        assert_eq!(parse_square("h8"), Some(Square::H8));
        for off in ["i1", "a9", "a0", "`1", "A1", "a", "a10"] {
            assert_eq!(parse_square(off), None, "{off}");
        }
    }

    /// Only a piece that can have been promoted keeps its Crazyhouse `~` marker.
    #[test]
    fn only_promotable_pieces_keep_their_promoted_marker() {
        let fen = "k~7/8/8/8/8/8/P~7/Q~6K~[] w - - 0 1";
        let pos = setup_position(Variant::Crazyhouse, parse_fen(fen).unwrap()).unwrap();
        assert_eq!(make_fen(&pos), "k7/8/8/8/8/8/P7/Q~6K[] w - - 0 1");
        let pos = setup_position(Variant::Chess, parse_fen(fen).unwrap()).unwrap();
        assert_eq!(make_fen(&pos), "k7/8/8/8/8/8/P7/Q6K w - - 0 1");
    }

    /// Each variant's own rules for a playable position, as chessops checks them.
    #[test]
    fn each_variant_refuses_what_chessops_refuses() {
        use Variant::*;
        let cases: &[(Variant, &str, bool)] = &[
            (Chess, START, true),
            (Chess, "8/8/8/8/8/8/8/8 w - - 0 1", false),
            (Chess, "k7/8/8/8/8/8/8/K6K w - - 0 1", false),
            (Chess, "8/8/8/8/8/8/8/K7 w - - 0 1", false),
            (Chess, "k7/8/8/8/8/8/8/K7 b - - 0 1", true),
            (Chess, "k7/8/8/8/8/8/8/RK6 w - - 0 1", false),
            (Chess, "k6P/8/8/8/8/8/8/K7 w - - 0 1", false),
            (KingOfTheHill, "k7/8/8/8/8/8/8/RK6 w - - 0 1", false),
            (ThreeCheck, "k6P/8/8/8/8/8/8/K7 w - - 0 1", false),
            (Crazyhouse, "k7/8/8/8/8/8/8/K7[Q] w - - 0 1", true),
            (Crazyhouse, "k7/8/8/8/8/8/8/K7[K] w - - 0 1", false),
            (Crazyhouse, "k7/8/8/8/8/8/8/K7[k] w - - 0 1", false),
            (Crazyhouse, "k7/8/8/8/8/8/8/RK6[] w - - 0 1", false),
            (Atomic, "k7/8/8/8/8/8/8/K7 w - - 0 1", true),
            (Atomic, "8/8/8/8/8/8/8/8 w - - 0 1", false),
            (Atomic, "k7/8/8/8/8/8/8/RK6 w - - 0 1", false),
            (Atomic, "k6P/8/8/8/8/8/8/K7 w - - 0 1", false),
            (Antichess, "8/8/8/8/8/8/8/N7 w - - 0 1", true),
            (Antichess, "8/8/8/8/8/8/8/8 w - - 0 1", false),
            (Antichess, "7P/8/8/8/8/8/8/N7 w - - 0 1", false),
            (RacingKings, "8/8/8/8/8/8/krbnNBRK/qrbnNBRQ w - - 0 1", true),
            (
                RacingKings,
                "8/8/8/8/8/8/krbnNBRK/qrbnNBRP w - - 0 1",
                false,
            ),
            (RacingKings, "8/8/8/8/8/8/k5rK/7R w - - 0 1", false),
            (
                Horde,
                "rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/PPPPPPPP/PPPPPPPP/PPPPPPPP w kq - 0 1",
                true,
            ),
            (Horde, "rnbqkbnr/pppppppp/8/8/8/8/8/P7 w kq - 0 1", true),
            (Horde, "rnbqkbnP/pppppppp/8/8/8/8/8/P7 w kq - 0 1", false),
            (Horde, "rnbqkbnr/pppppppp/8/8/8/8/8/K7 w kq - 0 1", false),
            (Horde, "8/8/8/8/8/8/8/P7 w - - 0 1", false),
        ];
        // A Crazyhouse game holds 64 pieces at most, on the board and in hand.
        let kings = "k7/8/8/8/8/8/8/K7";
        for (in_hand, valid) in [(62, true), (63, false)] {
            let fen = format!("{kings}[{}] w - - 0 1", "Q".repeat(in_hand));
            let position = parse_fen(&fen).and_then(|setup| setup_position(Crazyhouse, setup));
            assert_eq!(position.is_some(), valid, "{in_hand} in hand");
        }
        for &(variant, fen, valid) in cases {
            let position = parse_fen(fen).and_then(|setup| setup_position(variant, setup));
            assert_eq!(position.is_some(), valid, "{variant:?} {fen}");
        }
    }

    /// A Crazyhouse pocket holds at most 64 pieces.
    #[test]
    fn a_pocket_holds_at_most_64_pieces() {
        let board = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
        let with = |n: usize| parse_fen(&format!("{board}[{}] w KQkq - 0 1", "P".repeat(n)));
        assert!(with(64).is_some());
        assert!(with(65).is_none());
    }

    /// `K` and `Q` name the outermost rook, or the corner when there is none.
    #[test]
    fn castling_rights_name_the_outer_rooks() {
        let rights = |fen: &str| {
            let rights = parse_fen(fen).unwrap().castling_rights;
            rights
                .into_iter()
                .map(|sq| sq.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(rights("4k3/8/8/8/8/8/8/R2RK1RR w KQ - 0 1"), ["a1", "h1"]);
        assert_eq!(rights("4k3/8/8/8/8/8/8/4K3 w KQ - 0 1"), ["a1", "h1"]);
        assert_eq!(rights("4k3/8/8/8/8/8/8/4K3 w K - 0 1"), ["h1"]);
        assert_eq!(rights("1r2k1r1/8/8/8/8/8/8/4K3 w kq - 0 1"), ["b8", "g8"]);
        assert_eq!(rights("4k3/8/8/8/8/8/8/1R2K1R1 w BG - 0 1"), ["b1", "g1"]);
        assert!(parse_fen("4k3/8/8/8/8/8/8/RR2K2R w KQB - 0 1").is_none());
    }

    /// Three-check counts: checks remaining (`3+3`) or given (`+0+0`), three at most.
    #[test]
    fn reads_three_check_counts_either_way() {
        let read = |part: &str| {
            parse_remaining_checks(part)
                .map(|checks| (u32::from(checks.white), u32::from(checks.black)))
        };
        assert_eq!(read("3+2"), Some((3, 2)));
        assert_eq!(read("+1+0"), Some((2, 3)));
        assert_eq!(read("4+3"), None);
        assert_eq!(read("3+4"), None);
        assert_eq!(read("+0+4"), None);
        assert_eq!(read("3"), None);
    }

    fn chess(fen: &str) -> VariantPosition {
        standard_from_fen(fen).unwrap_or_else(|| panic!("{fen}"))
    }

    fn uci(pos: &VariantPosition, text: &str) -> Option<String> {
        legal_move(pos, parse_uci(text)?).map(|m| make_uci(&m))
    }

    /// An en passant square is kept only behind a pawn that has just passed it.
    #[test]
    fn an_en_passant_square_needs_the_pawn_that_passed_it() {
        let ep = |fen: &str| {
            parse_fen(fen)
                .and_then(|s| setup_position(Variant::Chess, s))
                .map(|p| p.ep_square(shakmaty::EnPassantMode::Always))
        };
        let after_e4 = "4k3/8/8/8/4P3/8/8/4K3 b - e3 0 1";
        assert_eq!(ep(after_e4), Some(Some(Square::E3)));
        // White to move: e3 is not behind a black pawn that just moved.
        assert_eq!(ep("4k3/8/8/8/4P3/8/8/4K3 w - e3 0 1"), Some(None));
        assert_eq!(ep("4k3/8/8/8/8/8/8/4K3 b - e3 0 1"), Some(None));
        assert_eq!(ep("4k3/8/8/8/4P3/8/4N3/4K3 b - e3 0 1"), Some(None));
        assert_eq!(ep("4k3/8/8/8/4p3/8/8/4K3 b - e3 0 1"), Some(None));
        let after_d5 = "4k3/8/8/3p4/8/8/8/4K3 w - d6 0 1";
        assert_eq!(ep(after_d5), Some(Some(Square::D6)));
        assert_eq!(ep("4k3/3n4/8/3p4/8/8/8/4K3 w - d6 0 1"), Some(None));
    }

    /// Castling rights are written `KQkq` for the outermost rooks and by file otherwise.
    #[test]
    fn castling_rights_are_written_as_lichess_writes_them() {
        let written = |fen: &str| {
            let pos = setup_position(Variant::Chess, parse_fen(fen).unwrap()).unwrap();
            make_fen(&pos).split(' ').nth(2).unwrap().to_string()
        };
        assert_eq!(written(START), "KQkq");
        assert_eq!(written("4k3/8/8/8/8/8/8/RKR4R w KQ - 0 1"), "KQ");
        assert_eq!(written("4k3/8/8/8/8/8/8/RKR4R w C - 0 1"), "C");
        assert_eq!(written("rkr4r/8/8/8/8/8/8/4K3 w c - 0 1"), "c");
        assert_eq!(written("4k3/8/8/8/8/8/8/4K3 w - - 0 1"), "-");
    }

    /// A UCI move is four squares' characters, or five with a promotion, or a drop.
    #[test]
    fn reads_and_writes_uci_moves() {
        assert!(parse_uci("e2e4").is_some());
        assert!(parse_uci("e7e8q").is_some());
        assert!(parse_uci("N@f3").is_some());
        for bad in ["e2e", "e2e4qq", "e2é4", "e2e4x", ""] {
            assert!(parse_uci(bad).is_none(), "{bad}");
        }
        let pos = chess("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        // Castling is written king to rook, either way it was given.
        assert_eq!(uci(&pos, "e1g1").as_deref(), Some("e1h1"));
        assert_eq!(uci(&pos, "e1c1").as_deref(), Some("e1a1"));
        assert_eq!(uci(&pos, "e1a1").as_deref(), Some("e1a1"));
        let castle = legal_move(&pos, parse_uci("e1h1").unwrap()).unwrap();
        assert_eq!(dest(&castle), Some(Square::H1));
        assert_eq!(
            dest(&Move::Put {
                role: Role::Pawn,
                to: Square::E4
            }),
            None
        );
    }

    /// Promotion is required on the last rank, never to a pawn, and to a king only in Antichess.
    #[test]
    fn promotions_follow_each_variants_rules() {
        let pos = chess("4k3/P7/8/8/8/8/8/4K3 w - - 0 1");
        assert_eq!(uci(&pos, "a7a8q").as_deref(), Some("a7a8q"));
        assert_eq!(uci(&pos, "a7a8n").as_deref(), Some("a7a8n"));
        assert_eq!(uci(&pos, "a7a8"), None);
        assert_eq!(uci(&pos, "a7a8p"), None);
        assert_eq!(uci(&pos, "a7a8k"), None);
        let pos = chess("4k3/8/8/8/8/8/8/R3K3 w - - 0 1");
        assert_eq!(uci(&pos, "a1a2q"), None);
        let antichess = setup_position(
            Variant::Antichess,
            parse_fen("8/P7/8/8/8/8/8/8 w - - 0 1").unwrap(),
        )
        .unwrap();
        assert_eq!(uci(&antichess, "a7a8k").as_deref(), Some("a7a8k"));
        assert_eq!(uci(&antichess, "a7a8p"), None);
        assert!(parse_san(&antichess, "a8=K").is_some());
        assert!(parse_san(&pos, "a2").is_none());
        let pos = chess("4k3/P7/8/8/8/8/8/4K3 w - - 0 1");
        assert!(parse_san(&pos, "a8=K").is_none());
        assert!(parse_san(&pos, "a8").is_none());
        assert_eq!(
            parse_san(&pos, "a8=Q").map(|m| make_uci(&m)).as_deref(),
            Some("a7a8q")
        );
        assert_eq!(
            parse_san(&pos, "a8N").map(|m| make_uci(&m)).as_deref(),
            Some("a7a8n")
        );
    }

    /// A drop needs the piece in the pocket, and a pawn is never dropped on a back rank.
    #[test]
    fn drops_need_the_piece_in_hand() {
        let pos = setup_position(
            Variant::Crazyhouse,
            parse_fen("4k3/8/8/8/8/8/8/4K3[Pn] w - - 0 1").unwrap(),
        )
        .unwrap();
        assert_eq!(uci(&pos, "P@e4").as_deref(), Some("P@e4"));
        assert_eq!(uci(&pos, "P@e8"), None);
        assert_eq!(uci(&pos, "P@e1"), None);
        assert_eq!(uci(&pos, "N@e4"), None, "the knight is Black's");
        assert_eq!(uci(&pos, "Q@e4"), None);
        assert!(parse_san(&pos, "P@e4").is_some());
        assert!(parse_san(&pos, "p@e4").is_some());
        assert!(parse_san(&pos, "@e4").is_some());
        assert!(parse_san(&pos, "X@e4").is_none());
        assert!(parse_san(&pos, "PP@e4").is_none());
        assert!(parse_san(&chess(START), "P@e4").is_none());
    }

    /// SAN names the moving piece's file, rank or both when another could go there.
    #[test]
    fn san_disambiguates_only_when_another_piece_can_move_there() {
        let san = |fen: &str, text: &str| {
            let mut pos = chess(fen);
            let m = legal_move(&pos, parse_uci(text).unwrap()).unwrap();
            san_and_play(&mut pos, m)
        };
        let knights = "4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1";
        assert_eq!(san(knights, "b1d2"), "Nbd2");
        assert_eq!(san(knights, "f1d2"), "Nfd2");
        assert_eq!(san(knights, "f1g3"), "Ng3");
        let rooks = "4k3/8/8/R7/8/8/8/R3K3 w - - 0 1";
        assert_eq!(san(rooks, "a1a3"), "R1a3");
        assert_eq!(san(rooks, "a5a3"), "R5a3");
        let queens = "k7/8/8/8/8/2Q1Q3/8/2Q1K3 w - - 0 1";
        assert_eq!(san(queens, "c3d2"), "Qc3d2");
        // A pinned knight is no rival.
        assert_eq!(san("4k3/4r3/8/8/8/8/4N3/1N2K3 w - - 0 1", "b1c3"), "Nc3");
        assert_eq!(san("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1g1"), "O-O");
        assert_eq!(san("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1c1"), "O-O-O");
        assert_eq!(san("4k3/8/8/8/8/8/8/4K3 w - - 0 1", "e1f1"), "Kf1");
        assert_eq!(san("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1", "e4d5"), "exd5");
        assert_eq!(san("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"), "exd6");
        assert_eq!(san("4k3/8/8/8/8/8/8/R3K3 w - - 0 1", "a1a8"), "Ra8+");
        assert_eq!(san("k7/8/1K6/8/8/8/8/7R w - - 0 1", "h1h8"), "Rh8#");
        // Antichess has no royal king, and two of them can reach one square.
        let mut kings = setup_position(
            Variant::Antichess,
            parse_fen("8/7p/8/8/8/8/8/K1K5 w - - 0 1").unwrap(),
        )
        .unwrap();
        let m = legal_move(&kings, parse_uci("a1b2").unwrap()).unwrap();
        assert_eq!(san_and_play(&mut kings, m), "Kab2");
    }

    /// SAN parsing finds the one piece that can make the move, in any spelling Lichess accepts.
    #[test]
    fn parses_san_to_the_one_legal_move() {
        let parsed = |fen: &str, text: &str| parse_san(&chess(fen), text).map(|m| make_uci(&m));
        let knights = "4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1";
        assert_eq!(parsed(knights, "Nd2"), None, "ambiguous");
        assert_eq!(parsed(knights, "Nbd2").as_deref(), Some("b1d2"));
        assert_eq!(parsed(knights, "N1d2"), None, "still ambiguous");
        assert_eq!(parsed(knights, "Nf1d2").as_deref(), Some("f1d2"));
        assert_eq!(parsed(knights, "Nf1-d2").as_deref(), Some("f1d2"));
        assert_eq!(parsed(START, "e4").as_deref(), Some("e2e4"));
        assert_eq!(parsed(START, "e2e4").as_deref(), Some("e2e4"));
        assert_eq!(parsed(START, "e2-e4").as_deref(), Some("e2e4"));
        assert_eq!(parsed(START, "e5"), None);
        assert_eq!(parsed(START, "Nf3+").as_deref(), Some("g1f3"));
        assert_eq!(parsed(START, "Nxf3"), Some("g1f3".to_string()));
        assert_eq!(parsed(START, "Kf3"), None);
        assert_eq!(
            parsed("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1", "exd5").as_deref(),
            Some("e4d5")
        );
        assert_eq!(parsed("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1", "d5"), None);
        let castles = "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1";
        assert_eq!(parsed(castles, "O-O").as_deref(), Some("e1h1"));
        assert_eq!(parsed(castles, "O-O-O+").as_deref(), Some("e1a1"));
        assert_eq!(
            parsed(castles, "Kg1"),
            None,
            "castling is written as castling"
        );
        assert_eq!(parsed("4k3/8/8/8/8/8/8/4K3 w - - 0 1", "O-O"), None);
        assert_eq!(parsed(START, "Nf3é"), None);
        assert_eq!(parsed(START, "Ni3"), None);
        assert_eq!(parsed(START, "Nf9"), None);
        // A Chess960 king beside its rook castles by moving onto it.
        let beside = setup_position(
            Variant::Chess,
            parse_fen("4k3/8/8/8/8/8/8/5KR1 w G - 0 1").unwrap(),
        )
        .unwrap();
        assert_eq!(
            parse_san(&beside, "Kg1").map(|m| make_uci(&m)).as_deref(),
            Some("f1g1")
        );
    }
}
