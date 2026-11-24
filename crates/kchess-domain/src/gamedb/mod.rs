//! A game database's chess rules: a game's main line stored as one byte a move, the key that
//! finds a position whatever the move order that reached it, a move packed into 16 bits, the
//! names a search matches, and what of a PGN game a database keeps. The core's game database
//! (`crates/kchess-core/src/gamedb`) stores what these make; nothing here touches a file.
//!
//! Both encodings outlive the release that wrote them, so neither may depend on anything a
//! shakmaty upgrade could change: a move's byte is its index among the position's legal moves in
//! an order fixed here (`ordered_moves`), and the position key is shakmaty's Zobrist hash, which
//! shakmaty keeps stable (Polyglot's keys; the tests pin the start position's).

use shakmaty::variant::{Variant, VariantPosition};
use shakmaty::zobrist::Zobrist64;
use shakmaty::{Color, EnPassantMode, Move, MoveList, Position, Role};

use crate::pgn::{self, Game};
use crate::position;
use crate::rules;

mod notes;

/// The first byte of a game's `extra` when it holds the notes `notes::pack` wrote.
const EXTRA_NOTES: u8 = 1;
/// The first byte of a game's `extra` when it holds the game's full PGN text.
const EXTRA_TEXT: u8 = 2;

/// The headers a database keeps as columns. A game with no other header, comment, NAG or
/// variation is stored with no `extra` and written again from them (`pgn_of`).
pub const KEPT_HEADERS: [&str; 11] = [
    "Event", "Site", "Date", "Round", "White", "Black", "Result", "WhiteElo", "BlackElo", "ECO",
    "Opening",
];

/// The longest main line a database keeps, in plies; a longer game is skipped.
pub const MAX_PLIES: usize = 1200;

/// The key of a position: equal for equal positions however they were reached, with an en passant
/// square only when a capture there is legal.
pub fn position_key(pos: &VariantPosition) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

/// What only a capture, a pawn move or a promotion changes in a position, in 32 bits: where each
/// side's pawns stand and how many knights, bishops, rooks and queens each has. A game keeps it
/// for many moves at a time, so a database indexes the few a game passes through rather than
/// every position, and a position can only be in the games that passed through its structure.
/// Stored: computed from bitboards (a1 = bit 0) and fixed constants alone, never from anything a
/// shakmaty upgrade could change.
pub fn structure_key(pos: &VariantPosition) -> u32 {
    let board = pos.board();
    let pawns = board.by_role(Role::Pawn);
    let mut counts = 0u64;
    for color in [Color::White, Color::Black] {
        for role in [Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
            let count = (board.by_color(color) & board.by_role(role)).count() as u64;
            counts = counts << 4 | count.min(15);
        }
    }
    let mix = |value: u64, by: u64| {
        let mixed = value.wrapping_mul(by);
        mixed ^ (mixed >> 29)
    };
    let hash = mix(
        (pawns & board.by_color(Color::White)).0,
        0x9E37_79B9_7F4A_7C15,
    ) ^ mix(
        (pawns & board.by_color(Color::Black)).0,
        0xC2B2_AE3D_27D4_EB4F,
    )
    .rotate_left(21)
        ^ mix(counts, 0x1656_67B1_9E37_79F9).rotate_left(42);
    let hash = mix(hash, 0xFF51_AFD7_ED55_8CCD);
    (hash >> 32) as u32
}

/// A move in 16 bits: from (6), to (6), promotion role (3) and whether it castles (1). Positions
/// castle king-takes-rook (`rules`), so a castle's `to` is its rook's square.
pub fn pack_move(m: &Move) -> u16 {
    let from = m.from().map_or(0, |sq| u16::from(u8::from(sq)));
    let to = u16::from(u8::from(m.to()));
    let promotion = m.promotion().map_or(0, |role| role as u16);
    from | (to << 6) | (promotion << 12) | (u16::from(m.is_castle()) << 15)
}

/// The legal move of `pos` that `pack_move` packed as `packed`.
pub fn unpack_move(pos: &VariantPosition, packed: u16) -> Option<Move> {
    pos.legal_moves()
        .into_iter()
        .find(|m| pack_move(m) == packed)
}

/// The legal moves in the order a move's byte counts in: by from square, to square and promotion,
/// which `pack_move` orders the same way.
fn ordered_moves(pos: &VariantPosition) -> MoveList {
    let mut moves = pos.legal_moves();
    moves.sort_unstable_by_key(order_key);
    moves
}

/// The move an encoded line's `byte` names in `pos`: the one at that place in `ordered_moves`,
/// found without sorting them all (no two moves share a place). None when there is no such move.
pub fn coded_move(pos: &VariantPosition, byte: u8) -> Option<Move> {
    let mut moves = pos.legal_moves();
    let index = usize::from(byte);
    if index >= moves.len() {
        return None;
    }
    moves.select_nth_unstable_by_key(index, order_key);
    Some(moves[index])
}

fn order_key(m: &Move) -> (u8, u8, u8) {
    let from = m.from().map_or(64, u8::from);
    let promotion = m.promotion().map_or(0, |role| role as u8);
    (from, u8::from(m.to()), promotion)
}

/// A main line as one byte a move: each its index among the position's ordered legal moves (no
/// position has more than 218, so a byte holds any). None when a move is not legal.
pub fn encode_line(start: &VariantPosition, moves: &[Move]) -> Option<Vec<u8>> {
    let mut pos = start.clone();
    let mut code = Vec::with_capacity(moves.len());
    for m in moves {
        let index = ordered_moves(&pos).iter().position(|legal| legal == m)?;
        code.push(u8::try_from(index).ok()?);
        pos.play_unchecked(*m);
    }
    Some(code)
}

/// Each move of an encoded line in turn, with the position before it; `visit` returns false to
/// stop. False when the code names a move the position does not have (a damaged row).
pub fn walk_line(
    start: &VariantPosition,
    code: &[u8],
    mut visit: impl FnMut(&VariantPosition, &Move) -> bool,
) -> bool {
    let mut pos = start.clone();
    for &byte in code {
        let Some(m) = coded_move(&pos, byte) else {
            return false;
        };
        if !visit(&pos, &m) {
            return true;
        }
        pos.play_unchecked(m);
    }
    true
}

/// An encoded line's moves; None when the code is damaged.
pub fn decode_line(start: &VariantPosition, code: &[u8]) -> Option<Vec<Move>> {
    let mut moves = Vec::with_capacity(code.len());
    walk_line(start, code, |_, m| {
        moves.push(*m);
        true
    })
    .then_some(moves)
}

/// A name as a search matches it: lowercase, letters and digits only, words joined by one space,
/// so "Carlsen, Magnus", "carlsen magnus" and "CARLSEN,MAGNUS" are alike.
pub fn fold_name(name: &str) -> String {
    let mut folded = String::with_capacity(name.len());
    for word in name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if !folded.is_empty() {
            folded.push(' ');
        }
        folded.extend(word.chars().flat_map(char::to_lowercase));
    }
    folded
}

/// A game's result as a database counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    White,
    Draw,
    Black,
    Unknown,
}

impl Outcome {
    pub fn from_header(result: Option<&str>) -> Outcome {
        match result {
            Some("1-0") => Outcome::White,
            Some("0-1") => Outcome::Black,
            Some("1/2-1/2") => Outcome::Draw,
            _ => Outcome::Unknown,
        }
    }

    /// Its column value: 1 White, 0 a draw, -1 Black, NULL unknown.
    pub fn score(self) -> Option<i8> {
        match self {
            Outcome::White => Some(1),
            Outcome::Draw => Some(0),
            Outcome::Black => Some(-1),
            Outcome::Unknown => None,
        }
    }

    pub fn from_score(score: Option<i64>) -> Outcome {
        match score {
            Some(1) => Outcome::White,
            Some(0) => Outcome::Draw,
            Some(-1) => Outcome::Black,
            _ => Outcome::Unknown,
        }
    }

    pub fn header(self) -> &'static str {
        match self {
            Outcome::White => "1-0",
            Outcome::Draw => "1/2-1/2",
            Outcome::Black => "0-1",
            Outcome::Unknown => "*",
        }
    }
}

/// Why a game cannot go into a database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// A variant other than standard chess (Chess960 included is standard here).
    Variant,
    /// Its starting position is not legal.
    Start,
    /// A main line move is not legal.
    Move,
    /// Longer than `MAX_PLIES`.
    TooLong,
}

/// What a database keeps of a game.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub event: Option<String>,
    pub site: Option<String>,
    /// As written, `YYYY.MM.DD` with `??` for what is unknown.
    pub date: Option<String>,
    pub year: Option<i32>,
    pub month: Option<u8>,
    pub round: Option<String>,
    pub white: String,
    pub black: String,
    pub white_elo: Option<u16>,
    pub black_elo: Option<u16>,
    pub outcome: Outcome,
    pub eco: Option<String>,
    pub opening: Option<String>,
    /// The starting FEN when the game does not start from the standard position.
    pub fen: Option<String>,
    pub plies: usize,
    pub moves: Vec<u8>,
    /// What the columns and moves do not hold (other headers, comments, NAGs, a set-up start):
    /// none for a plain game, its notes (`notes`), or its full text when it has variations.
    pub extra: Option<Vec<u8>>,
}

/// The value of a kept header, or None when it is empty or PGN's "unknown" (`?`).
fn known(game: &Game, name: &str) -> Option<String> {
    game.header(name)
        .map(str::trim)
        .filter(|value| !value.chars().all(|c| matches!(c, '?' | '-' | '.')))
        .map(str::to_string)
}

fn elo(game: &Game, name: &str) -> Option<u16> {
    game.header(name)?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|&elo| (1..=4000).contains(&elo))
}

/// `YYYY.MM.DD`'s year and month, as far as they are known.
pub fn date_parts(date: &str) -> (Option<i32>, Option<u8>) {
    let mut parts = date.split(['.', '-', '/']);
    let year = parts
        .next()
        .and_then(|year| year.parse::<i32>().ok())
        .filter(|year| (1000..=9999).contains(year));
    let month = year.and(
        parts
            .next()
            .and_then(|month| month.parse::<u8>().ok())
            .filter(|month| (1..=12).contains(month)),
    );
    (year, month)
}

/// Whether the columns write the game again as it was: only kept headers, and no comment, NAG,
/// variation or set-up start.
fn is_plain(game: &Game) -> bool {
    game.headers
        .iter()
        .all(|(name, _)| KEPT_HEADERS.contains(&name.as_str()))
        && game.comments.is_none()
        && game.nodes.iter().enumerate().all(|(id, node)| {
            node.comments.is_none()
                && node.starting_comments.is_none()
                && node.nags.is_none()
                && game.children(id).nth(1).is_none()
        })
}

/// What a database keeps of `game`.
pub fn store(game: &Game) -> Result<Stored, Unusable> {
    if rules::pgn_variant(game.header("Variant")) != Some(Variant::Chess) {
        return Err(Unusable::Variant);
    }
    let Some(start) = position::starting_position(game) else {
        return Err(Unusable::Start);
    };
    let mut pos = start.clone();
    let mut moves = Vec::new();
    for node in game.main_line() {
        if moves.len() >= MAX_PLIES {
            return Err(Unusable::TooLong);
        }
        let Some(m) = rules::parse_san(&pos, game.node(node).san.as_str()) else {
            return Err(Unusable::Move);
        };
        pos.play_unchecked(m);
        moves.push(m);
    }
    let code = encode_line(&start, &moves).ok_or(Unusable::Move)?;
    let fen = known(game, "FEN");
    let date = known(game, "Date");
    let (year, month) = date.as_deref().map_or((None, None), date_parts);
    let mut stored = Stored {
        event: known(game, "Event"),
        site: known(game, "Site"),
        year,
        month,
        date,
        round: known(game, "Round"),
        white: known(game, "White").unwrap_or_default(),
        black: known(game, "Black").unwrap_or_default(),
        white_elo: elo(game, "WhiteElo"),
        black_elo: elo(game, "BlackElo"),
        outcome: Outcome::from_header(game.header("Result")),
        eco: known(game, "ECO"),
        opening: known(game, "Opening"),
        extra: None,
        fen,
        plies: moves.len(),
        moves: code,
    };
    if stored.fen.is_some() || !is_plain(game) {
        let sans = sans_of(&start, &moves);
        stored.extra = Some(match notes::pack(game, &stored, &sans) {
            Some(packed) => packed,
            None => {
                let mut text = vec![EXTRA_TEXT];
                text.extend_from_slice(pgn::make_pgn(game).as_bytes());
                text
            }
        });
    }
    Ok(stored)
}

/// The main line in standard SAN, as a stored game is written.
fn sans_of(start: &VariantPosition, moves: &[Move]) -> Vec<String> {
    let mut pos = start.clone();
    moves
        .iter()
        .map(|m| rules::san_and_play(&mut pos, *m))
        .collect()
}

/// A game's starting position: the FEN it was stored with, or the standard start.
pub fn start_of(fen: Option<&str>) -> Option<VariantPosition> {
    match fen {
        Some(fen) => rules::standard_from_fen(fen),
        None => Some(rules::default_position(Variant::Chess)),
    }
}

/// A stored game's PGN: written from its columns and moves, with its notes when it has any, or
/// its text as imported when it has variations. None when its moves or notes are damaged.
pub fn pgn_of(stored: &Stored) -> Option<String> {
    let extra = stored.extra.as_deref();
    if let Some((&EXTRA_TEXT, text)) = extra.and_then(<[u8]>::split_first) {
        return String::from_utf8(text.to_vec()).ok();
    }
    let start = start_of(stored.fen.as_deref())?;
    let sans = sans_of(&start, &decode_line(&start, &stored.moves)?);
    let game = match extra {
        Some(notes) if notes.first() == Some(&EXTRA_NOTES) => notes::unpack(notes, stored, &sans)?,
        Some(_) => return None,
        None => {
            let mut game = Game::empty();
            for name in KEPT_HEADERS {
                if let Some(value) = notes::column(stored, name) {
                    game.put_header(name, value);
                }
            }
            let mut parent = 0;
            for san in &sans {
                parent = game.add_move(parent, pgn::PgnNode::with(san, None, None, None));
            }
            game
        }
    };
    Some(pgn::make_pgn(&game))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_testkit::random;

    fn game(text: &str) -> Game {
        pgn::parse_pgn(text).remove(0)
    }

    #[test]
    fn the_start_positions_key_is_polyglots() {
        let start = rules::default_position(Variant::Chess);
        assert_eq!(position_key(&start), 0x463b96181691fc9c);
    }

    #[test]
    fn transpositions_share_a_key_and_a_dead_en_passant_square_is_ignored() {
        let after = |sans: &[&str]| {
            let mut pos = rules::default_position(Variant::Chess);
            for san in sans {
                let m = rules::parse_san(&pos, san).unwrap();
                pos.play_unchecked(m);
            }
            position_key(&pos)
        };
        assert_eq!(
            after(&["e4", "e5", "Nf3", "Nc6"]),
            after(&["Nf3", "Nc6", "e4", "e5"])
        );
        // 1.Nf3 Nf6 2.e4 leaves an en passant square no black pawn can use: it meets 1.e4 Nf6 2.Nf3.
        assert_eq!(after(&["Nf3", "Nf6", "e4"]), after(&["e4", "Nf6", "Nf3"]));
        assert_ne!(after(&["e4"]), after(&["e3"]));
    }

    #[test]
    fn random_lines_round_trip_through_their_code() {
        random::check(200, |rng| {
            let start = rules::default_position(Variant::Chess);
            let mut pos = start.clone();
            let mut moves = Vec::new();
            for _ in 0..rng.below(160) {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                pos.play_unchecked(m);
                moves.push(m);
            }
            let code = encode_line(&start, &moves).unwrap();
            assert_eq!(code.len(), moves.len());
            assert_eq!(decode_line(&start, &code).unwrap(), moves);
            let mut pos = start.clone();
            for m in &moves {
                assert_eq!(unpack_move(&pos, pack_move(m)).as_ref(), Some(m));
                pos.play_unchecked(*m);
            }
        });
    }

    #[test]
    fn packed_moves_unpack_in_their_position() {
        let pos =
            rules::standard_from_fen("r3k2r/pPpp1ppp/8/8/8/8/PPPP1PPP/R3K2R w KQkq - 0 1").unwrap();
        for m in pos.legal_moves() {
            assert_eq!(unpack_move(&pos, pack_move(&m)), Some(m));
        }
    }

    #[test]
    fn a_damaged_code_is_refused_not_misread() {
        let start = rules::default_position(Variant::Chess);
        assert_eq!(decode_line(&start, &[255]), None);
        assert_eq!(decode_line(&start, &[]), Some(Vec::new()));
    }

    #[test]
    fn names_fold_to_the_same_search_text() {
        assert_eq!(fold_name("Carlsen, Magnus"), "carlsen magnus");
        assert_eq!(fold_name("CARLSEN,MAGNUS"), "carlsen magnus");
        assert_eq!(fold_name("  Nepomniachtchi,  Ian "), "nepomniachtchi ian");
        assert_eq!(fold_name("Ding Liren"), "ding liren");
        assert_eq!(fold_name("Höbel, Ä."), "höbel ä");
    }

    #[test]
    fn a_plain_game_is_stored_without_its_text_and_written_again() {
        let text = "[Event \"Tata Steel\"]\n[Site \"Wijk aan Zee\"]\n[Date \"2024.01.20\"]\n\
            [Round \"7\"]\n[White \"Carlsen, Magnus\"]\n[Black \"Gukesh D\"]\n[Result \"1-0\"]\n\
            [WhiteElo \"2830\"]\n[BlackElo \"2725\"]\n[ECO \"C65\"]\n\n\
            1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6 4. O-O 1-0\n";
        let stored = store(&game(text)).unwrap();
        assert_eq!(stored.extra, None);
        assert_eq!((stored.year, stored.month), (Some(2024), Some(1)));
        assert_eq!(stored.white_elo, Some(2830));
        assert_eq!(stored.outcome, Outcome::White);
        assert_eq!(stored.plies, 7);
        assert_eq!(pgn_of(&stored).unwrap(), pgn::make_pgn(&game(text)));
    }

    #[test]
    fn a_game_with_variations_keeps_its_text() {
        let text = "[White \"A\"]\n[Black \"B\"]\n[Result \"*\"]\n[Annotator \"Me\"]\n\n\
            1. e4 { best by test } e5 (1... c5 2. Nf3) 2. Nf3 $1 *\n";
        let stored = store(&game(text)).unwrap();
        let extra = stored.extra.as_deref().unwrap();
        assert_eq!(extra.first(), Some(&EXTRA_TEXT));
        assert_eq!(pgn_of(&stored).unwrap(), pgn::make_pgn(&game(text)));
        assert_eq!(stored.plies, 3);
    }

    const BROADCAST: &str = "[Event \"South Island Chess Championships 2026\"]\n\
        [Site \"https://lichess.org/broadcast/2026-nz-south-island-championship/round-4/6W8sy3kg/gET308sb\"]\n\
        [Date \"2026.09.29\"]\n[Round \"04\"]\n[White \"Wang, Justin Zhide\"]\n\
        [Black \"Mohammadi, Arvin\"]\n[Result \"1/2-1/2\"]\n[WhiteElo \"2103\"]\n\
        [WhiteTitle \"CM\"]\n[WhiteFideId \"4312830\"]\n[BlackElo \"1996\"]\n\
        [BlackFideId \"12528617\"]\n[TimeControl \"90+30\"]\n[Board \"1\"]\n\
        [Variant \"Standard\"]\n[ECO \"B23\"]\n[Opening \"Sicilian Defense: Closed\"]\n\
        [UTCDate \"2026.09.29\"]\n[UTCTime \"00:34:40\"]\n\
        [BroadcastName \"2026 NZ South Island Championship\"]\n\
        [BroadcastURL \"https://lichess.org/broadcast/2026-nz-south-island-championship/round-4/6W8sy3kg\"]\n\
        [GameURL \"https://lichess.org/broadcast/2026-nz-south-island-championship/round-4/6W8sy3kg/gET308sb\"]\n\n\
        1. e4 { [%eval 0.18] [%clk 1:30:58] } 1... c5 { [%eval 0.32] [%clk 1:30:35] } \
        2. Nc3 { [%eval 0.19] [%clk 1:31:18] } 2... a6 { [%eval 0.32] [%clk 1:30:33] } \
        3. g3 { [%eval -0.18] [%clk 0:00:05.3] } 3... b5 { [%eval #-3] [%clk 1:30:57] } \
        4. Bg2 { [%eval 0.27] } { Inaccuracy. Qe5 was best. } { [%clk 1:32:08] } \
        4... Bb7?! $13 { [%eval -0.0] [%clk 1:31:21] } 5. Nge2 { [%clk 1:32:35] [%eval 0.25] } \
        { A comment of its own } 5... e6 { [%clk 1:30:30] } 1/2-1/2\n";

    #[test]
    fn a_broadcast_game_is_kept_in_its_notes_exactly() {
        let original = game(BROADCAST);
        let stored = store(&original).unwrap();
        let extra = stored.extra.as_deref().unwrap();
        assert_eq!(extra.first(), Some(&EXTRA_NOTES));
        let text = pgn::make_pgn(&original);
        assert_eq!(pgn_of(&stored).unwrap(), text);
        // The headers' text and the URLs they share, and four bytes or so a move.
        assert!(
            extra.len() * 4 < text.len(),
            "{} of {}",
            extra.len(),
            text.len()
        );
    }

    #[test]
    fn a_set_up_game_with_comments_is_kept_in_its_notes() {
        let text = "[SetUp \"1\"]\n[FEN \"4k3/8/8/8/8/8/4P3/4K3 b - - 0 7\"]\n\n\
            { Black to move } 7... Kd7 { [%clk 0:01:00] } 8. e4 $1 *\n";
        let stored = store(&game(text)).unwrap();
        assert_eq!(stored.extra.as_deref().unwrap().first(), Some(&EXTRA_NOTES));
        assert_eq!(pgn_of(&stored).unwrap(), pgn::make_pgn(&game(text)));
    }

    #[test]
    fn a_damaged_note_is_refused_not_misread() {
        let mut stored = store(&game(BROADCAST)).unwrap();
        let extra = stored.extra.as_mut().unwrap();
        extra.truncate(extra.len() / 2);
        assert_eq!(pgn_of(&stored), None);
        stored.extra = Some(vec![EXTRA_NOTES, 200, 200, 200]);
        assert_eq!(pgn_of(&stored), None);
        stored.extra = Some(vec![9]);
        assert_eq!(pgn_of(&stored), None);
    }

    /// Random games with random evals, clocks, comments and NAGs read back as they were written.
    #[test]
    fn random_annotated_games_read_back_exactly() {
        random::check(300, |rng| {
            let mut pos = rules::default_position(Variant::Chess);
            let mut text = String::from("[White \"A\"]\n[Black \"B\"]\n[Result \"*\"]\n");
            if rng.below(2) == 0 {
                text.push_str("[GameURL \"https://example.org/game/abcdef\"]\n");
                text.push_str("[Site \"https://example.org/game/abcdef/x\"]\n");
            }
            text.push('\n');
            if rng.below(4) == 0 {
                text.push_str("{ before } ");
            }
            for ply in 0..rng.below(80) {
                let legal = pos.legal_moves();
                let Some(m) = rng.pick(&legal).copied() else {
                    break;
                };
                if ply % 2 == 0 {
                    text.push_str(&format!("{}. ", ply / 2 + 1));
                }
                text.push_str(&rules::san_and_play(&mut pos, m));
                if rng.below(6) == 0 {
                    text.push_str(&format!(" ${}", rng.below(140)));
                }
                let eval = match rng.below(5) {
                    0 => format!("#{}", rng.below(20) as i64 - 10),
                    1 => format!("{}", rng.below(30) as i64 - 15),
                    _ => format!("{}.{:02}", rng.below(9) as i64 - 4, rng.below(100)),
                };
                let clock = format!(
                    "{}:{:02}:{:02}{}",
                    rng.below(3),
                    rng.below(60),
                    rng.below(60),
                    if rng.below(4) == 0 { ".7" } else { "" }
                );
                match rng.below(7) {
                    0 => text.push_str(&format!(" {{ [%eval {eval}] [%clk {clock}] }}")),
                    1 => text.push_str(&format!(" {{ [%clk {clock}] }}")),
                    2 => text.push_str(&format!(" {{ [%eval {eval}] }}")),
                    3 => text.push_str(&format!(" {{ [%clk {clock}] [%eval {eval}] }}")),
                    4 => text.push_str(&format!(" {{ [%eval {eval}] }} {{ Note {ply} }}")),
                    _ => {}
                }
                text.push(' ');
            }
            text.push_str("*\n");
            let original = game(&text);
            let stored = store(&original).unwrap();
            assert_eq!(pgn_of(&stored).unwrap(), pgn::make_pgn(&original), "{text}");
        });
    }

    #[test]
    fn a_set_up_start_is_kept_with_its_fen() {
        let fen = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
        let text = format!("[SetUp \"1\"]\n[FEN \"{fen}\"]\n\n1. e4 Kd7 *\n");
        let stored = store(&game(&text)).unwrap();
        assert_eq!(stored.fen.as_deref(), Some(fen));
        let start = start_of(stored.fen.as_deref()).unwrap();
        assert_eq!(decode_line(&start, &stored.moves).unwrap().len(), 2);
    }

    #[test]
    fn unusable_games_say_why() {
        assert_eq!(
            store(&game("[Variant \"Atomic\"]\n\n1. e4 *\n")).unwrap_err(),
            Unusable::Variant
        );
        assert_eq!(
            store(&game("1. e4 e5 2. Ke3 *\n")).unwrap_err(),
            Unusable::Move
        );
        assert_eq!(
            store(&game("[FEN \"8/8/8/8/8/8/8/8 w - - 0 1\"]\n\n*\n")).unwrap_err(),
            Unusable::Start
        );
    }

    #[test]
    fn unknown_headers_are_not_kept_as_values() {
        let stored = store(&game(
            "[Event \"?\"]\n[Date \"????.??.??\"]\n[WhiteElo \"-\"]\n\n*\n",
        ))
        .unwrap();
        assert_eq!(
            (stored.event, stored.date, stored.white_elo),
            (None, None, None)
        );
    }
}
