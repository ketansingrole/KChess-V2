//! Positions as the frontends hold them: a variant and a FEN,
//! with every question about them answered here, as the original `Position` answered it.

use serde::Serialize;
use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Color, KnownOutcome, Outcome, Position, Rank, Role, Square};

use crate::js;
use crate::pgn::{Game, make_pgn, parse_pgn};
use crate::rules::{self, UciMove};

/// Antichess ends only when the side to move has no pieces left.
fn variant_end(pos: &VariantPosition) -> bool {
    match pos {
        VariantPosition::Antichess(_) => pos.us().is_empty(),
        _ => pos.is_variant_end(),
    }
}

/// The legal moves: none once the variant has ended.
fn moves(pos: &VariantPosition) -> shakmaty::MoveList {
    if variant_end(pos) {
        shakmaty::MoveList::new()
    } else {
        pos.legal_moves()
    }
}

/// `Some(None)` is a draw.
fn outcome(pos: &VariantPosition, no_moves: bool) -> Option<Option<Color>> {
    let ended = variant_end(pos);
    if let VariantPosition::Antichess(_) = pos {
        // Antichess's own outcome: no pieces or no moves wins for the side to move.
        if ended || no_moves {
            return Some(Some(pos.turn()));
        }
    } else if ended {
        return match pos.variant_outcome() {
            Outcome::Known(KnownOutcome::Decisive { winner }) => Some(Some(winner)),
            _ => Some(None),
        };
    }
    if pos.is_check() && no_moves {
        Some(Some(!pos.turn()))
    } else if pos.is_insufficient_material() || no_moves {
        Some(None)
    } else {
        None
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub fen: String,
    pub turn: &'static str,
    pub check: bool,
    pub checkmate: bool,
    pub stalemate: bool,
    /// Neither side can win.
    pub insufficient: bool,
    /// Each side that has too little material to win.
    pub cannot_win: CannotWin,
    pub variant_end: bool,
    pub end: bool,
    /// Absent while the game goes on, `{}` for a draw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    pub halfmoves: u32,
    pub fullmoves: u32,
    /// Any castling rights left.
    pub castling: bool,
    /// Pieces on the board.
    pub pieces: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct CannotWin {
    pub white: bool,
    pub black: bool,
}

/// Everything a frontend asks of a position.
pub fn info(pos: &VariantPosition) -> Info {
    let ended = variant_end(pos);
    let no_moves = moves(pos).is_empty();
    let check = pos.is_check();
    Info {
        fen: rules::make_fen(pos),
        turn: crate::color::name(pos.turn()),
        check,
        checkmate: !ended && check && no_moves,
        stalemate: !ended && !check && no_moves,
        insufficient: pos.is_insufficient_material(),
        cannot_win: CannotWin {
            white: pos.has_insufficient_material(Color::White),
            black: pos.has_insufficient_material(Color::Black),
        },
        variant_end: ended,
        end: ended || pos.is_insufficient_material() || no_moves,
        outcome: outcome(pos, no_moves).map(|winner| match winner {
            Some(color) => json!({ "winner": crate::color::name(color) }),
            None => json!({}),
        }),
        halfmoves: pos.halfmoves(),
        fullmoves: pos.fullmoves().get(),
        castling: !pos.castles().is_empty(),
        pieces: pos.board().occupied().count() as u32,
    }
}

/// Where each piece in the side to move's Crazyhouse pocket may be dropped, in role order
/// (pawn to king) and squares ascending; empty in other variants and once the game has ended.
pub fn drop_dests(pos: &VariantPosition) -> Vec<(Role, Vec<Square>)> {
    let mut out: Vec<(Role, Vec<Square>)> = Vec::new();
    for m in &moves(pos) {
        if let shakmaty::Move::Put { role, to } = *m {
            match out.iter_mut().find(|(r, _)| *r == role) {
                Some((_, squares)) => squares.push(to),
                None => out.push((role, vec![to])),
            }
        }
    }
    out.sort_by_key(|(role, _)| *role);
    for (_, squares) in &mut out {
        squares.sort();
    }
    out
}

/// How destinations are listed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DestMode {
    /// Castling onto the rook only.
    Rules,
    /// Standard castling also onto the king's square.
    Board,
}

/// Each piece's destinations, pieces and squares in ascending order (a1, b1, … h8).
pub fn dests(pos: &VariantPosition, mode: DestMode) -> Vec<(Square, Vec<Square>)> {
    let mut by_from = [Bitboard::EMPTY; 64];
    for m in &moves(pos) {
        if let (Some(from), Some(to)) = (m.from(), rules::dest(m)) {
            by_from[usize::from(from)] |= Bitboard::from_square(to);
        }
    }
    let king = pos.board().king_of(pos.turn());
    let mut out = Vec::new();
    for from in pos.us() {
        let squares = by_from[usize::from(from)];
        if squares.is_empty() {
            continue;
        }
        let mut list: Vec<Square> = squares.into_iter().collect();
        if mode == DestMode::Board && Some(from) == king && from.file() == shakmaty::File::E {
            // The board takes both kinds of castling destination.
            if squares.contains(Square::A1) {
                list.push(Square::C1);
            } else if squares.contains(Square::A8) {
                list.push(Square::C8);
            }
            if squares.contains(Square::H1) {
                list.push(Square::G1);
            } else if squares.contains(Square::H8) {
                list.push(Square::G8);
            }
        }
        out.push((from, list));
    }
    out
}

/// One legal move, listed the way the voice commands search them.
#[derive(Debug, Serialize)]
pub struct LegalMove {
    pub from: String,
    pub to: String,
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion: Option<&'static str>,
    pub san: String,
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

/// Every legal move from `dests`, with promotions to queen, rook, bishop and knight.
pub fn legal_moves(pos: &VariantPosition) -> Vec<LegalMove> {
    let mut out = Vec::new();
    for (from, tos) in dests(pos, DestMode::Rules) {
        let Some(role) = pos.board().role_at(from) else {
            continue;
        };
        for to in tos {
            let promotes = role == Role::Pawn && matches!(to.rank(), Rank::First | Rank::Eighth);
            let promotions: &[Option<Role>] = if promotes {
                &[
                    Some(Role::Queen),
                    Some(Role::Rook),
                    Some(Role::Bishop),
                    Some(Role::Knight),
                ]
            } else {
                &[None]
            };
            for &promotion in promotions {
                let uci = UciMove::Normal {
                    from,
                    to,
                    promotion,
                };
                let Some(m) = rules::legal_move(pos, uci) else {
                    continue;
                };
                let mut after = pos.clone();
                out.push(LegalMove {
                    from: from.to_string(),
                    to: to.to_string(),
                    role: role_name(role),
                    promotion: promotion.map(role_name),
                    san: rules::san_and_play(&mut after, m),
                });
            }
        }
    }
    out
}

/// Every legal move's SAN: the board's moves as [`legal_moves`] lists them, then the
/// Crazyhouse drops (`P@e3`), which have no square to come from.
pub fn legal_sans(pos: &VariantPosition) -> Vec<String> {
    let mut sans: Vec<String> = legal_moves(pos).into_iter().map(|m| m.san).collect();
    for (role, squares) in drop_dests(pos) {
        for to in squares {
            let mut after = pos.clone();
            sans.push(rules::san_and_play(
                &mut after,
                shakmaty::Move::Put { role, to },
            ));
        }
    }
    sans
}

/// A move played: its UCI (castling king-to-rook), its SAN and the result.
#[derive(Debug, Serialize)]
pub struct Played {
    pub uci: String,
    pub san: String,
    pub position: Info,
}

fn played(pos: &VariantPosition, m: shakmaty::Move) -> Played {
    let mut after = pos.clone();
    let uci = rules::make_uci(&m);
    let san = rules::san_and_play(&mut after, m);
    Played {
        uci,
        san,
        position: info(&after),
    }
}

/// `uci` played when it is legal (castling onto the king's square or the rook's).
pub fn play_uci(pos: &VariantPosition, uci: &str) -> Option<Played> {
    let m = rules::parse_uci(uci).and_then(|u| rules::legal_move(pos, u))?;
    Some(played(pos, m))
}

/// `parseSan` then `play`.
pub fn play_san(pos: &VariantPosition, san: &str) -> Option<Played> {
    let m = rules::parse_san(pos, san)?;
    Some(played(pos, m))
}

#[derive(Debug, Serialize)]
pub struct LineMove {
    pub uci: String,
    pub san: String,
    pub fen: String,
}

/// Moves (UCI, or SAN with `san`) played one after another until one is not legal.
/// UCI moves are trimmed first.
pub fn line<S: AsRef<str>>(
    mut pos: VariantPosition,
    moves: &[S],
    san: bool,
    trim: bool,
) -> (VariantPosition, Vec<LineMove>) {
    let mut out = Vec::with_capacity(moves.len());
    for text in moves {
        let text = if trim {
            js::trim(text.as_ref())
        } else {
            text.as_ref()
        };
        let m = if san {
            rules::parse_san(&pos, text)
        } else {
            rules::parse_uci(text).and_then(|u| rules::legal_move(&pos, u))
        };
        let Some(m) = m else { break };
        let uci = rules::make_uci(&m);
        let san = rules::san_and_play(&mut pos, m);
        out.push(LineMove {
            uci,
            san,
            fen: rules::make_fen(&pos),
        });
    }
    (pos, out)
}

/* ── Typed positions (the board editor) ── */

/// A FEN's placement field and side to move.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoardSetup {
    pub board: String,
    pub turn: &'static str,
}

/// A FEN read without validation: the placement field and side to move as written.
pub fn fen_setup(fen: &str) -> Option<BoardSetup> {
    let setup = rules::parse_fen(fen)?;
    let written = rules::make_setup_fen(&setup);
    let board = written.split(' ').next().unwrap_or_default().to_string();
    Some(BoardSetup {
        board,
        turn: crate::color::name(setup.turn),
    })
}

/// Why a FEN is not a standard chess position Stockfish can analyse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenProblem {
    /// Not readable as a FEN.
    Fen,
    Empty,
    Kings,
    OppositeCheck,
    PawnsOnBackrank,
    TooManyPieces,
    /// Refused for a reason none of the others names.
    Illegal,
}

impl FenProblem {
    /// Its code, as the rules corpus records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            FenProblem::Fen => "fen",
            FenProblem::Empty => "empty",
            FenProblem::Kings => "kings",
            FenProblem::OppositeCheck => "oppositeCheck",
            FenProblem::PawnsOnBackrank => "pawnsOnBackrank",
            FenProblem::TooManyPieces => "tooManyPieces",
            FenProblem::Illegal => "illegal",
        }
    }

    /// What the app tells a player about it.
    pub const fn message(self) -> &'static str {
        match self {
            FenProblem::Fen => "That is not a valid FEN",
            FenProblem::Empty => "The board is empty",
            FenProblem::Kings => "Each side needs exactly one king",
            FenProblem::PawnsOnBackrank => "Pawns can’t stand on the first or last rank",
            FenProblem::TooManyPieces => "Each side can have at most 16 pieces",
            FenProblem::OppositeCheck => "The side not to move is in check",
            FenProblem::Illegal => "This position is not legal",
        }
    }
}

/// Why `Chess.fromSetup(parseFen(fen))` refuses a FEN, in its order of checks; a standard
/// position with more than 16 pieces a side is refused too.
pub fn fen_problem(fen: &str) -> Option<FenProblem> {
    let Some(setup) = rules::parse_fen(fen) else {
        return Some(FenProblem::Fen);
    };
    if rules::standard_from_fen(fen).is_some() {
        // Stockfish refuses more than 16 pieces for a side, and quits rather than analyse them.
        let crowded = setup.board.white().count() > 16 || setup.board.black().count() > 16;
        return crowded.then_some(FenProblem::TooManyPieces);
    }
    let board = &setup.board;
    let turn = setup.turn;
    if board.occupied().is_empty() {
        return Some(FenProblem::Empty);
    }
    if board.kings().count() != 2 || board.king_of(turn).is_none() {
        return Some(FenProblem::Kings);
    }
    let Some(other) = board.king_of(!turn) else {
        return Some(FenProblem::Kings);
    };
    if board.attacks_to(other, turn, board.occupied()).any() {
        return Some(FenProblem::OppositeCheck);
    }
    let backranks = Bitboard::from_rank(Rank::First) | Bitboard::from_rank(Rank::Eighth);
    if board.pawns().intersects(backranks) {
        return Some(FenProblem::PawnsOnBackrank);
    }
    Some(FenProblem::Illegal)
}

/* ── PGN games ── */

pub fn starting_position(game: &Game) -> Option<VariantPosition> {
    let variant = rules::pgn_variant(game.header("Variant"))?;
    match game.header("FEN").filter(|fen| !fen.is_empty()) {
        Some(fen) => rules::setup_position(variant, rules::parse_fen(fen)?),
        None => Some(rules::default_position(variant)),
    }
}

/// Why a game cannot be replayed: `start` (its starting position) or `move` (an illegal move,
/// in any variation); None when every move is legal.
fn game_problem(game: &Game) -> Option<&'static str> {
    let Some(start) = starting_position(game) else {
        return Some("start");
    };
    let mut stack: Vec<(usize, VariantPosition)> =
        game.children(0).map(|c| (c, start.clone())).collect();
    while let Some((node, mut pos)) = stack.pop() {
        let Some(m) = rules::parse_san(&pos, game.nodes[node].san.as_str()) else {
            return Some("move");
        };
        pos.play_unchecked(m);
        for child in game.children(node) {
            stack.push((child, pos.clone()));
        }
    }
    None
}

/// One game of a PGN document.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PgnGame {
    /// Its headers, in order, as name and value.
    pub headers: Vec<(String, String)>,
    /// Its text as the PGN writer writes it.
    pub pgn: String,
    /// Why it cannot be replayed, if it cannot.
    pub problem: Option<&'static str>,
}

/// Each game of a PGN document.
pub fn pgn_games(text: &str) -> Vec<PgnGame> {
    parse_pgn(text)
        .iter()
        .map(|game| PgnGame {
            headers: game.headers.clone(),
            pgn: make_pgn(game),
            problem: game_problem(game),
        })
        .collect()
}

/// The value of the last `[%clk h:mm:ss]` in a comment.
pub fn comment_clock(comment: &str) -> Option<f64> {
    let mut clock = None;
    let mut rest = comment;
    while let Some(at) = rest.find("[%clk") {
        let after = &rest[at + 5..];
        rest = after;
        let mut chars = after.chars();
        if !chars.next().is_some_and(js::is_space) {
            continue;
        }
        if let Some(value) = clock_value(chars.as_str()) {
            clock = Some(value);
        }
    }
    clock
}

/// `(\d{1,5}):(\d{1,2}):(\d{1,2}(?:\.\d{0,3})?)\]`
fn clock_value(text: &str) -> Option<f64> {
    let digits = |s: &str, max: usize| -> Option<usize> {
        let n = s.bytes().take_while(u8::is_ascii_digit).count();
        (1..=max).contains(&n).then_some(n)
    };
    let h = digits(text, 5)?;
    let rest = text[h..].strip_prefix(':')?;
    let m = digits(rest, 2)?;
    let secs = rest[m..].strip_prefix(':')?;
    let s = digits(secs, 2)?;
    let mut end = s;
    if secs[s..].starts_with('.') {
        let fraction = secs[s + 1..].bytes().take_while(u8::is_ascii_digit).count();
        if fraction > 3 {
            return None;
        }
        end = s + 1 + fraction;
    }
    secs[end..].strip_prefix(']')?;
    let hours: f64 = text[..h].parse().ok()?;
    let minutes: f64 = rest[..m].parse().ok()?;
    let seconds: f64 = secs[..end].trim_end_matches('.').parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// One move of a game's main line.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MainlineMove {
    pub san: String,
    pub uci: String,
    pub fen: String,
    /// The colour that played it.
    pub mover: &'static str,
    /// Whether it gives check.
    pub check: bool,
    /// The clock its comment records, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<f64>,
}

/// A game's main line, from its start.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mainline {
    pub headers: Vec<(String, String)>,
    /// The start's FEN; none when the game's own start cannot be set up.
    pub start: Option<String>,
    /// Whether the side to move is in check at the start.
    pub start_check: bool,
    pub moves: Vec<MainlineMove>,
}

/// The first game's main line, played from its start (the usual start when its own cannot be
/// set up) until a move is illegal, at most `limit` moves.
pub fn pgn_mainline(text: &str, limit: usize) -> Option<Mainline> {
    let games = parse_pgn(text);
    let game = games.first()?;
    let start = starting_position(game);
    let mut pos = start
        .clone()
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess));
    let start_check = pos.is_check();
    let mut moves = Vec::new();
    for child in game.main_line().take(limit) {
        let data = &game.nodes[child];
        let Some(m) = rules::parse_san(&pos, data.san.as_str()) else {
            break;
        };
        let mover = crate::color::name(pos.turn());
        let uci = rules::make_uci(&m);
        pos.play_unchecked(m);
        let clock = data
            .comments
            .as_ref()
            .and_then(|comments| comment_clock(&comments.join(" ")));
        moves.push(MainlineMove {
            san: data.san.as_str().to_string(),
            uci,
            fen: rules::make_fen(&pos),
            mover,
            check: pos.is_check(),
            clock,
        });
    }
    Some(Mainline {
        headers: game.headers.clone(),
        start: start.as_ref().map(rules::make_fen),
        start_check,
        moves,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn describes_mate_and_stalemate() {
        let pos = rules::standard_from_fen("k7/1Q6/1K6/8/8/8/8/8 b - - 0 1").unwrap();
        let info = info(&pos);
        assert!(info.checkmate && info.end && info.check);
        assert_eq!(info.outcome, Some(json!({ "winner": "white" })));
        let pos = rules::standard_from_fen("k7/8/1QK5/8/8/8/8/8 b - - 0 1").unwrap();
        let info = super::info(&pos);
        assert!(info.stalemate && !info.check);
        assert_eq!(info.outcome, Some(json!({})));
    }

    /// Every legal move's SAN, a Crazyhouse pocket's drops after the board's moves.
    #[test]
    fn lists_every_legal_san_with_the_drops_last() {
        let pos = rules::standard_from_fen(START).unwrap();
        let sans = legal_sans(&pos);
        assert_eq!(sans.len(), 20);
        assert!(sans.contains(&"Nf3".to_string()) && sans.contains(&"e4".to_string()));
        let crazyhouse = rules::setup_position(
            shakmaty::variant::Variant::Crazyhouse,
            rules::parse_fen("k7/8/8/8/8/8/8/7K[N] w - - 0 1").unwrap(),
        )
        .unwrap();
        let sans = legal_sans(&crazyhouse);
        assert_eq!(sans.len(), 3 + 62, "{sans:?}");
        assert_eq!(sans.last().map(String::as_str), Some("N@h8"));
        assert!(sans[..3].iter().all(|san| san.starts_with('K')));
    }

    /// Bare kings: moves remain, yet the game is drawn and over.
    #[test]
    fn bare_kings_end_the_game_in_a_draw() {
        let pos = rules::standard_from_fen("k7/8/8/8/8/8/8/7K w - - 0 1").unwrap();
        let info = super::info(&pos);
        assert!(
            info.insufficient && info.end && !info.variant_end,
            "{info:?}"
        );
        assert!(!info.stalemate && !info.checkmate);
        assert_eq!(info.outcome, Some(json!({})));
    }

    /// Antichess ends when the side to move has no pieces, which wins it: the other side
    /// running out of pieces first does not end it.
    #[test]
    fn antichess_ends_when_the_side_to_move_has_no_pieces() {
        let antichess = |fen| {
            rules::setup_position(
                shakmaty::variant::Variant::Antichess,
                rules::parse_fen(fen).unwrap(),
            )
            .unwrap()
        };
        let info = super::info(&antichess("8/8/8/8/8/8/8/k7 b - - 0 1"));
        assert!(!info.variant_end && !info.end, "{info:?}");
        let info = super::info(&antichess("8/8/8/8/8/8/8/K7 b - - 0 1"));
        assert!(info.variant_end && info.end, "{info:?}");
        assert_eq!(info.outcome, Some(json!({ "winner": "black" })));
    }

    #[test]
    fn lists_castling_both_ways_for_the_board() {
        let pos = rules::standard_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let king = |mode| {
            dests(&pos, mode)
                .into_iter()
                .find(|(from, _)| *from == Square::E1)
                .unwrap()
                .1
                .iter()
                .map(Square::to_string)
                .collect::<Vec<_>>()
        };
        assert!(king(DestMode::Rules).contains(&"h1".to_string()));
        assert!(!king(DestMode::Rules).contains(&"g1".to_string()));
        assert!(king(DestMode::Board).ends_with(&["c1".to_string(), "g1".to_string()]));
    }

    #[test]
    fn plays_castling_either_way() {
        let pos = rules::standard_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        assert_eq!(play_uci(&pos, "e1g1").unwrap().uci, "e1h1");
        assert_eq!(play_uci(&pos, "e1h1").unwrap().san, "O-O");
        assert!(play_uci(&pos, "e1e3").is_none());
        assert_eq!(play_san(&pos, "O-O-O").unwrap().uci, "e1a1");
    }

    /// `%clk` times: up to milliseconds, as Lichess writes them.
    #[test]
    fn reads_clock_times_to_the_millisecond() {
        assert_eq!(clock_value("1:02:03]"), Some(3723.0));
        assert_eq!(clock_value("0:00:05.]"), Some(5.0));
        assert_eq!(clock_value("0:00:05.125]"), Some(5.125));
        assert_eq!(clock_value("0:00:05.1250]"), None);
        assert_eq!(clock_value("0:00:05"), None);
        assert_eq!(clock_value("123456:00:05]"), None);
    }

    #[test]
    fn explains_unplayable_fens() {
        assert_eq!(fen_problem(START).map(FenProblem::as_str), None);
        assert_eq!(fen_problem("nonsense").map(FenProblem::as_str), Some("fen"));
        assert_eq!(
            fen_problem("8/8/8/8/8/8/8/8 w - - 0 1").map(FenProblem::as_str),
            Some("empty")
        );
        assert_eq!(
            fen_problem("8/8/8/8/8/8/8/4K3 w - - 0 1").map(FenProblem::as_str),
            Some("kings")
        );
        assert_eq!(
            fen_problem("k7/8/8/8/8/8/8/K6K w - - 0 1").map(FenProblem::as_str),
            Some("kings")
        );
        assert_eq!(
            fen_problem("K7/8/8/8/8/8/8/8 b - - 0 1").map(FenProblem::as_str),
            Some("kings")
        );
        // Black's seventeenth piece counts as much as White's.
        assert_eq!(
            fen_problem("K7/8/8/8/8/1q6/pppppppp/qqqqqqqk b - - 0 1").map(FenProblem::as_str),
            Some("tooManyPieces")
        );
        assert_eq!(
            fen_problem("4k3/4Q3/8/8/8/8/8/4K3 w - - 0 1").map(FenProblem::as_str),
            Some("oppositeCheck")
        );
        assert_eq!(
            fen_problem("P3k3/8/8/8/8/8/8/4K3 w - - 0 1").map(FenProblem::as_str),
            Some("pawnsOnBackrank")
        );
        // A queen added to a full set: Stockfish quit on it ("More than 32 pieces").
        assert_eq!(
            fen_problem("rnbqkbnr/pppppppp/8/8/3q3N/4K3/1PPPPPPP/RNBQ1BNR w kq - 0 1")
                .map(FenProblem::as_str),
            Some("tooManyPieces")
        );
        assert_eq!(
            fen_problem("4k3/8/8/8/8/PPPPPPPP/PPPPPPPP/RNBQKBNR w - - 0 1").map(FenProblem::as_str),
            Some("tooManyPieces")
        );
    }

    #[test]
    fn reads_clocks_like_chessops() {
        assert_eq!(comment_clock("[%clk 1:02:03]"), Some(3723.0));
        assert_eq!(
            comment_clock("a [%clk 0:00:05.5] b [%clk 0:00:04.]"),
            Some(4.0)
        );
        assert_eq!(comment_clock("[%clk 0:00:05.1234]"), None);
        assert_eq!(comment_clock("[%clk  0:00:05]"), None);
        assert_eq!(comment_clock("[%clk 123456:00:05]"), None);
    }

    #[test]
    fn reads_the_main_line() {
        let line = pgn_mainline(
            "1. e4 { [%clk 0:01:00] } e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# *",
            3,
        )
        .unwrap();
        assert_eq!(line.moves.len(), 3);
        assert_eq!(line.moves[0].clock, Some(60.0));
        assert_eq!(line.start.as_deref(), Some(START));
    }

    /// A board's destinations castle onto the king's square too, a Chess960 board's onto the
    /// rook only, as the rules do.
    #[test]
    fn dests_castle_as_the_board_asks() {
        let pos = rules::setup_start(
            crate::variant::Variant::Standard,
            "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
        )
        .unwrap();
        let king = |mode| {
            dests(&pos, mode)
                .into_iter()
                .find(|(from, _)| *from == Square::E1)
                .unwrap()
                .1
        };
        assert!(king(DestMode::Board).contains(&Square::G1));
        let squares = king(DestMode::Rules);
        assert!(squares.contains(&Square::H1));
        assert!(!squares.contains(&Square::G1));
    }

    #[test]
    fn crazyhouse_drops_list_each_pocket_piece_and_its_squares() {
        let pos = crate::rules::setup_start(
            crate::variant::Variant::Crazyhouse,
            "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3",
        )
        .unwrap();
        let drops = super::drop_dests(&pos);
        assert_eq!(drops.len(), 1);
        let (role, squares) = &drops[0];
        assert_eq!(*role, Role::Pawn);
        // Pawns never land on the first or last rank, nor on a piece.
        assert!(squares.contains(&Square::E4));
        assert!(!squares.contains(&Square::E1) && !squares.contains(&Square::E8));
        assert!(!squares.contains(&Square::A2));
        // Black's pawn waits for Black's turn; standard chess has no pockets.
        assert!(
            super::drop_dests(
                &crate::rules::setup_start(
                    crate::variant::Variant::Standard,
                    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
                )
                .unwrap()
            )
            .is_empty()
        );
    }
}
