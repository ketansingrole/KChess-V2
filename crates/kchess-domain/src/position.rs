//! Positions as the frontends hold them (`crates/kchess-wasm/js/position.ts`): a variant and a FEN,
//! with every question about them answered here, as chessops answered it on its `Position`.

use serde::Serialize;
use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Color, KnownOutcome, Outcome, Position, Rank, Role, Square};

use crate::js;
use crate::pgn::{Game, make_pgn, parse_pgn};
use crate::rules::{self, UciMove};

/// chessops `isVariantEnd()`. Antichess ends only when the side to move has no pieces left.
fn variant_end(pos: &VariantPosition) -> bool {
    match pos {
        VariantPosition::Antichess(_) => pos.us().is_empty(),
        _ => pos.is_variant_end(),
    }
}

/// chessops `allDests()`'s moves: none once the variant has ended.
fn moves(pos: &VariantPosition) -> shakmaty::MoveList {
    if variant_end(pos) {
        shakmaty::MoveList::new()
    } else {
        pos.legal_moves()
    }
}

/// chessops `outcome()`: `Some(None)` is a draw.
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

fn color_name(color: Color) -> &'static str {
    rules::turn_name(color)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub fen: String,
    pub turn: &'static str,
    pub check: bool,
    pub checkmate: bool,
    pub stalemate: bool,
    /// `isInsufficientMaterial()`: neither side can win.
    pub insufficient: bool,
    /// `hasInsufficientMaterial(color)` for each side.
    pub cannot_win: CannotWin,
    pub variant_end: bool,
    pub end: bool,
    /// `outcome()`: absent while the game goes on, `{}` for a draw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    pub halfmoves: u32,
    pub fullmoves: u32,
    /// Any castling rights left.
    pub castling: bool,
    /// Pieces on the board.
    pub pieces: u32,
}

#[derive(Debug, Serialize)]
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
        turn: color_name(pos.turn()),
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
            Some(color) => json!({ "winner": color_name(color) }),
            None => json!({}),
        }),
        halfmoves: pos.halfmoves(),
        fullmoves: pos.fullmoves().get(),
        castling: !pos.castles().is_empty(),
        pieces: pos.board().occupied().count() as u32,
    }
}

/// How destinations are listed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DestMode {
    /// chessops `allDests()`: castling onto the rook only.
    Rules,
    /// chessops `chessgroundDests(pos)`: standard castling also onto the king's square.
    Board,
    /// chessops `chessgroundDests(pos, { chess960: true })`.
    Board960,
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
            // Chessground needs both kinds of castling destination.
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

/// A move played: chessops `makeUci` of the move (castling king-to-rook), its SAN and the result.
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

/// `uci` played as chessops `normalizeMove` + `isLegal` + `play` would (castling either way).
pub fn play_uci(pos: &VariantPosition, uci: &str) -> Option<Played> {
    let m = rules::parse_uci(uci).and_then(|u| rules::legal_move(pos, u))?;
    Some(played(pos, m))
}

/// chessops `parseSan` then `play`.
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
/// UCI moves are trimmed first with `trim`, as `replaySetup` reads them.
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

/// chessops `parseFen` without validation: the placement field and side to move as written.
pub fn fen_setup(fen: &str) -> Option<Value> {
    let setup = rules::parse_fen(fen)?;
    let written = rules::make_setup_fen(&setup);
    let board = written.split(' ').next().unwrap_or_default().to_string();
    Some(json!({ "board": board, "turn": color_name(setup.turn) }))
}

/// Why chessops `Chess.fromSetup(parseFen(fen))` refuses a FEN, in its order of checks:
/// `fen` (unreadable), `empty`, `kings`, `oppositeCheck`, `pawnsOnBackrank` or `illegal`.
pub fn fen_problem(fen: &str) -> Option<&'static str> {
    let Some(setup) = rules::parse_fen(fen) else {
        return Some("fen");
    };
    if rules::standard_from_fen(fen).is_some() {
        return None;
    }
    let board = &setup.board;
    let turn = setup.turn;
    if board.occupied().is_empty() {
        return Some("empty");
    }
    if board.kings().count() != 2 || board.king_of(turn).is_none() {
        return Some("kings");
    }
    let Some(other) = board.king_of(!turn) else {
        return Some("kings");
    };
    if board.attacks_to(other, turn, board.occupied()).any() {
        return Some("oppositeCheck");
    }
    let backranks = Bitboard::from_rank(Rank::First) | Bitboard::from_rank(Rank::Eighth);
    if board.pawns().intersects(backranks) {
        return Some("pawnsOnBackrank");
    }
    Some("illegal")
}

/* ── PGN games ── */

/// chessops `startingPosition(headers)`.
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

fn headers(game: &Game) -> Value {
    Value::Array(game.headers.iter().map(|(k, v)| json!([k, v])).collect())
}

/// chessops `parsePgn`: each game's headers, its text as `makePgn` writes it, and why it cannot
/// be replayed (`game_problem`), if it cannot.
pub fn pgn_games(text: &str) -> Value {
    Value::Array(
        parse_pgn(text)
            .iter()
            .map(|game| {
                json!({
                    "headers": headers(game),
                    "pgn": make_pgn(game),
                    "problem": game_problem(game),
                })
            })
            .collect(),
    )
}

/// The value of the last `[%clk h:mm:ss]` in a comment, as chessops `parseComment` reads it.
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

/// The first game's main line, played from its start (the usual start when its own cannot be
/// set up, so `start` is null) until a move is illegal, at most `limit` moves. `check` is
/// chessops `isCheck()` after each move (`startCheck` before the first).
pub fn pgn_mainline(text: &str, limit: usize) -> Value {
    let games = parse_pgn(text);
    let Some(game) = games.first() else {
        return Value::Null;
    };
    let start = starting_position(game);
    let mut pos = start
        .clone()
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess));
    let start_check = pos.is_check();
    let mut moves = Vec::new();
    let mut node = 0;
    while let Some(child) = game.children(node).next() {
        if moves.len() >= limit {
            break;
        }
        node = child;
        let data = &game.nodes[child];
        let Some(m) = rules::parse_san(&pos, data.san.as_str()) else {
            break;
        };
        let mover = color_name(pos.turn());
        let uci = rules::make_uci(&m);
        rules::san_and_play(&mut pos, m);
        let clock = data
            .comments
            .as_ref()
            .and_then(|comments| comment_clock(&comments.join(" ")));
        let mut entry = json!({
            "san": data.san.as_str(),
            "uci": uci,
            "fen": rules::make_fen(&pos),
            "mover": mover,
            "check": pos.is_check(),
        });
        if let Some(clock) = clock {
            entry["clock"] = json!(clock);
        }
        moves.push(entry);
    }
    json!({
        "headers": headers(game),
        "start": start.as_ref().map(rules::make_fen),
        "startCheck": start_check,
        "moves": moves,
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
        assert!(!king(DestMode::Board960).contains(&"g1".to_string()));
    }

    #[test]
    fn plays_castling_either_way() {
        let pos = rules::standard_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        assert_eq!(play_uci(&pos, "e1g1").unwrap().uci, "e1h1");
        assert_eq!(play_uci(&pos, "e1h1").unwrap().san, "O-O");
        assert!(play_uci(&pos, "e1e3").is_none());
        assert_eq!(play_san(&pos, "O-O-O").unwrap().uci, "e1a1");
    }

    #[test]
    fn explains_unplayable_fens() {
        assert_eq!(fen_problem(START), None);
        assert_eq!(fen_problem("nonsense"), Some("fen"));
        assert_eq!(fen_problem("8/8/8/8/8/8/8/8 w - - 0 1"), Some("empty"));
        assert_eq!(fen_problem("8/8/8/8/8/8/8/4K3 w - - 0 1"), Some("kings"));
        assert_eq!(
            fen_problem("4k3/4Q3/8/8/8/8/8/4K3 w - - 0 1"),
            Some("oppositeCheck")
        );
        assert_eq!(
            fen_problem("P3k3/8/8/8/8/8/8/4K3 w - - 0 1"),
            Some("pawnsOnBackrank")
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
        );
        assert_eq!(line["moves"].as_array().unwrap().len(), 3);
        assert_eq!(line["moves"][0]["clock"], json!(60.0));
        assert_eq!(line["start"], json!(START));
    }
}
