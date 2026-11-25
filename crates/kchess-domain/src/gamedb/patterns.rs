//! What a deep search of a game database looks for in a game's moves: a material balance, a pawn
//! structure or pieces on given squares that it reaches and keeps, and the speed its time
//! control was played at. Each is
//! checked by replaying the game; a balance or a structure counts once it lasts `HOLD` plies (a
//! capture made and taken back does not reach one), or holds when the game ends.

use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Board, Color, File, Position, Rank, Role, Square};

/// The plies a balance or a structure must last.
pub const HOLD: usize = 4;

/// Each side's pieces of a material balance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Side {
    queens: u8,
    rooks: u8,
    bishops: u8,
    knights: u8,
    pawns: u8,
}

/// A material balance: one side's pieces against the other's, with either colour on either side.
/// The pawns count only when it names some ("RP vs R"); otherwise any pawns go ("R vs B").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    sides: [Side; 2],
    pawns: bool,
}

impl Material {
    /// "R vs B", "KRP v KR", "Q-RR", "rvb": the pieces each side has besides its king. None
    /// for anything else, or for two empty sides.
    pub fn parse(text: &str) -> Option<Material> {
        let text = text.trim().to_ascii_uppercase().replace("VS", "V");
        let text: String = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| if c == '-' { 'V' } else { c })
            .collect();
        let (first, second) = text.split_once('V')?;
        let side = |letters: &str| {
            let mut side = Side::default();
            for letter in letters.chars() {
                let count = match letter {
                    'K' => continue,
                    'Q' => &mut side.queens,
                    'R' => &mut side.rooks,
                    'B' => &mut side.bishops,
                    'N' => &mut side.knights,
                    'P' => &mut side.pawns,
                    _ => return None,
                };
                *count = count.checked_add(1).filter(|count| *count <= 10)?;
            }
            Some(side)
        };
        let sides = [side(first)?, side(second)?];
        if sides == [Side::default(); 2] && !text.contains('K') {
            return None;
        }
        let pawns = text.contains('P');
        Some(Material { sides, pawns })
    }

    fn side(board: &Board, color: Color) -> Side {
        let count = |role: Role| board.by_piece(role.of(color)).count() as u8;
        Side {
            queens: count(Role::Queen),
            rooks: count(Role::Rook),
            bishops: count(Role::Bishop),
            knights: count(Role::Knight),
            pawns: count(Role::Pawn),
        }
    }

    /// Whether `board` has this balance, either colour on either side.
    pub fn on(&self, board: &Board) -> bool {
        let same = |want: &Side, has: &Side| {
            (want.queens, want.rooks, want.bishops, want.knights)
                == (has.queens, has.rooks, has.bishops, has.knights)
                && (!self.pawns || want.pawns == has.pawns)
        };
        let (white, black) = (
            Self::side(board, Color::White),
            Self::side(board, Color::Black),
        );
        (same(&self.sides[0], &white) && same(&self.sides[1], &black))
            || (same(&self.sides[0], &black) && same(&self.sides[1], &white))
    }
}

/// A pawn structure, of either colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structure {
    /// A d-pawn with no pawn of its colour on the c- or e-file.
    IsolatedQueenPawn,
    /// c- and d-pawns side by side on the fourth rank (fifth for Black), with no pawn of their
    /// colour on the b- or e-file.
    HangingPawns,
    /// The Queen's Gambit Exchange's: d4 against c6 and d5, with no White c-pawn and no Black
    /// e-pawn (or the same with the colours swapped).
    Carlsbad,
    /// Pawns on c4 and e4 and none on the d-file, against a side with no c-pawn.
    MaroczyBind,
    /// Pawns on c3, d4, e3 and f4 (c6, d5, e6 and f5 for Black).
    Stonewall,
}

/// The square on `file` and White's `rank`, seen from `color`'s side.
fn at(color: Color, file: File, rank: Rank) -> Square {
    let square = Square::from_coords(file, rank);
    if color.is_white() {
        square
    } else {
        square.flip_vertical()
    }
}

impl Structure {
    pub const ALL: [Structure; 5] = [
        Structure::IsolatedQueenPawn,
        Structure::HangingPawns,
        Structure::Carlsbad,
        Structure::MaroczyBind,
        Structure::Stonewall,
    ];

    /// Whether `board` has it, with `color` as the side the structure names first.
    fn of(self, board: &Board, color: Color) -> bool {
        let ours = board.by_piece(Role::Pawn.of(color));
        let theirs = board.by_piece(Role::Pawn.of(!color));
        let on_file = |pawns: Bitboard, file: File| !(pawns & Bitboard::from_file(file)).is_empty();
        let has = |pawns: Bitboard, file: File, rank: Rank| pawns.contains(at(color, file, rank));
        // Their squares are named from our side too: c6 and d5 against our d4.
        let has_theirs = |file: File, rank: Rank| theirs.contains(at(color, file, rank));
        match self {
            Structure::IsolatedQueenPawn => {
                on_file(ours, File::D) && !on_file(ours, File::C) && !on_file(ours, File::E)
            }
            Structure::HangingPawns => {
                has(ours, File::C, Rank::Fourth)
                    && has(ours, File::D, Rank::Fourth)
                    && !on_file(ours, File::B)
                    && !on_file(ours, File::E)
            }
            Structure::Carlsbad => {
                has(ours, File::D, Rank::Fourth)
                    && !on_file(ours, File::C)
                    && has_theirs(File::C, Rank::Sixth)
                    && has_theirs(File::D, Rank::Fifth)
                    && !on_file(theirs, File::E)
            }
            Structure::MaroczyBind => {
                has(ours, File::C, Rank::Fourth)
                    && has(ours, File::E, Rank::Fourth)
                    && !on_file(ours, File::D)
                    && !on_file(theirs, File::C)
            }
            Structure::Stonewall => [
                (File::C, Rank::Third),
                (File::D, Rank::Fourth),
                (File::E, Rank::Third),
                (File::F, Rank::Fourth),
            ]
            .into_iter()
            .all(|(file, rank)| has(ours, file, rank)),
        }
    }

    /// Whether `board` has it, for either colour.
    pub fn on(self, board: &Board) -> bool {
        self.of(board, Color::White) || self.of(board, Color::Black)
    }
}

/// Pieces on squares: one side's against the other's ("Nd5 e4 vs e6"), with either colour on
/// either side. The squares are named from the first side's view, so "Nd5 vs e6" is also a black
/// knight on d4 against a white pawn on e3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    sides: [Vec<(Role, Square)>; 2],
}

/// The most pieces a placement names.
const MOST_PLACED: usize = 32;

impl Placement {
    /// "Nd5 e4 vs e6", "nd5 v e6", "Bg5": each side's pieces as SAN names them (a pawn by its
    /// square alone, or with P), the sides parted by "vs" or "v". None for anything else, for no
    /// pieces, for two on one square, or for a pawn on the first or last rank.
    pub fn parse(text: &str) -> Option<Placement> {
        let mut sides: [Vec<(Role, Square)>; 2] = [Vec::new(), Vec::new()];
        let mut side = 0;
        for token in text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|token| !token.is_empty())
        {
            if token.eq_ignore_ascii_case("vs") || token.eq_ignore_ascii_case("v") {
                if side == 1 {
                    return None;
                }
                side = 1;
                continue;
            }
            let (role, square) = match token.len() {
                2 => (Role::Pawn, token),
                3 => {
                    let role = match token.chars().next()?.to_ascii_uppercase() {
                        'K' => Role::King,
                        'Q' => Role::Queen,
                        'R' => Role::Rook,
                        'B' => Role::Bishop,
                        'N' => Role::Knight,
                        'P' => Role::Pawn,
                        _ => return None,
                    };
                    (role, token.get(1..)?)
                }
                _ => return None,
            };
            let square: Square = square.to_ascii_lowercase().parse().ok()?;
            if role == Role::Pawn && matches!(square.rank(), Rank::First | Rank::Eighth) {
                return None;
            }
            sides[side].push((role, square));
        }
        let all: Vec<Square> = sides.iter().flatten().map(|(_, square)| *square).collect();
        let mut distinct = all.clone();
        distinct.sort();
        distinct.dedup();
        if all.is_empty() || all.len() > MOST_PLACED || distinct.len() != all.len() {
            return None;
        }
        Some(Placement { sides })
    }

    /// Whether `board` has it with `color` as the first side.
    fn of(&self, board: &Board, color: Color) -> bool {
        self.sides
            .iter()
            .zip([color, !color])
            .all(|(pieces, side)| {
                pieces.iter().all(|&(role, square)| {
                    let square = if color.is_white() {
                        square
                    } else {
                        square.flip_vertical()
                    };
                    board.piece_at(square) == Some(role.of(side))
                })
            })
    }

    /// Whether `board` has it, either colour on either side.
    pub fn on(&self, board: &Board) -> bool {
        self.of(board, Color::White) || self.of(board, Color::Black)
    }
}

/// Whether the game from `start` with the encoded moves `code` reaches a position `test` keeps
/// true for `HOLD` plies, or until the game ends. `changes` says which moves can change the
/// answer, so `test` is asked again only after them. None when the code is damaged.
pub fn reaches(
    start: &VariantPosition,
    code: &[u8],
    changes: impl Fn(&shakmaty::Move) -> bool,
    test: impl Fn(&Board) -> bool,
) -> Option<bool> {
    let mut pos = start.clone();
    let mut held = usize::from(test(pos.board()));
    if held >= HOLD {
        return Some(true);
    }
    for &byte in code {
        let m = super::coded_move(&pos, byte)?;
        let changed = changes(&m);
        pos.play_unchecked(m);
        let now = if changed { test(pos.board()) } else { held > 0 };
        held = if now { held + 1 } else { 0 };
        if held >= HOLD {
            return Some(true);
        }
    }
    Some(held > 0)
}

/// The structures each side reaches and keeps in the game from `start` through `moves`, as
/// `reaches` counts them, with the colour that has each: a side can have several, and both
/// sides the same one.
pub fn structures_held<P: Position + Clone>(
    start: &P,
    moves: &[shakmaty::Move],
) -> Vec<(Structure, Color)> {
    let kinds: Vec<(Structure, Color)> = Structure::ALL
        .into_iter()
        .flat_map(|structure| [(structure, Color::White), (structure, Color::Black)])
        .collect();
    let mut held = vec![0usize; kinds.len()];
    let mut kept = vec![false; kinds.len()];
    let mut pos = start.clone();
    let look = |board: &Board, held: &mut [usize], kept: &mut [bool], changed: bool| {
        for (index, (structure, color)) in kinds.iter().enumerate() {
            let now = if changed {
                structure.of(board, *color)
            } else {
                held[index] > 0
            };
            held[index] = if now { held[index] + 1 } else { 0 };
            kept[index] |= held[index] >= HOLD;
        }
    };
    look(pos.board(), &mut held, &mut kept, true);
    for m in moves {
        pos.play_unchecked(*m);
        look(pos.board(), &mut held, &mut kept, changes_pawns(m));
    }
    kinds
        .into_iter()
        .zip(held.into_iter().zip(kept))
        .filter(|(_, (held, kept))| *kept || *held > 0)
        .map(|(kind, _)| kind)
        .collect()
}

/// The moves that can change a material balance: captures and promotions.
pub fn changes_material(m: &shakmaty::Move) -> bool {
    m.is_capture() || m.is_promotion()
}

/// The moves that can change a pawn structure: pawn moves and captures.
pub fn changes_pawns(m: &shakmaty::Move) -> bool {
    m.role() == Role::Pawn || m.is_capture()
}

/// The time control's speed as Lichess counts it: the base time and forty increments, in
/// seconds (`-` is correspondence). None for one it cannot read.
pub fn speed(time_control: &str) -> Option<Speed> {
    let time_control = time_control.trim();
    if time_control == "-" {
        return Some(Speed::Correspondence);
    }
    // FIDE's classical controls are written as periods ("40/7200:1800+30"): the first period's
    // time is enough to call it classical.
    let first = time_control.split(':').next()?;
    let first = first.rsplit('/').next()?;
    let (base, increment) = match first.split_once('+') {
        Some((base, increment)) => (base, increment.parse::<u64>().ok()?),
        None => (first, 0),
    };
    let duration = base.parse::<u64>().ok()? + 40 * increment;
    Some(match duration {
        0..30 => Speed::UltraBullet,
        30..180 => Speed::Bullet,
        180..480 => Speed::Blitz,
        480..1500 => Speed::Rapid,
        _ => Speed::Classical,
    })
}

/// How fast a game was played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::fen::Fen;
    use shakmaty::{CastlingMode, Chess};

    fn board(fen: &str) -> Board {
        let pos: Chess = fen
            .parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Standard)
            .unwrap();
        pos.board().clone()
    }

    #[test]
    fn a_game_holds_the_structures_each_side_keeps() {
        use shakmaty::san::San;
        let play = |fen: &str, sans: &[&str]| {
            let start: Chess = fen
                .parse::<Fen>()
                .unwrap()
                .into_position(CastlingMode::Standard)
                .unwrap();
            let mut pos = start.clone();
            let moves: Vec<shakmaty::Move> = sans
                .iter()
                .map(|san| {
                    let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
                    pos.play_unchecked(m);
                    m
                })
                .collect();
            structures_held(&start, &moves)
        };
        // White's isolated d-pawn, kept for four plies.
        let isolated = "r1bq1rk1/pp2bppp/2n1pn2/8/3P4/2NB1N2/PP3PPP/R1BQ1RK1 w - - 0 1";
        assert_eq!(
            play(isolated, &["Re1", "Nb4", "Bb1", "b6"]),
            vec![(Structure::IsolatedQueenPawn, Color::White)]
        );
        // Given back at once: dxe5 leaves no d-pawn, so it never lasted.
        let traded = "r1bq1rk1/pp2bppp/2n2n2/4p3/3P4/2NB1N2/PP3PPP/R1BQ1RK1 w - - 0 1";
        assert_eq!(
            play(traded, &["dxe5", "Nxe5", "Nxe5", "Qd4", "Re1"]),
            Vec::new()
        );
        // Reached as the game ends: it counts, as a search finds it.
        let late = "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2NB1N2/PP3PPP/R1BQ1RK1 b - - 0 1";
        assert_eq!(
            play(late, &["dxc4", "Bxc4"]),
            vec![(Structure::IsolatedQueenPawn, Color::White)]
        );
        assert_eq!(play(late, &[]), Vec::new());
    }

    #[test]
    fn pieces_are_written_as_san_names_them() {
        let outpost = Placement::parse("Nd5 vs e6").unwrap();
        assert_eq!(Placement::parse(" nd5  V  E6 "), Some(outpost.clone()));
        assert_eq!(Placement::parse("Nd5 vs Pe6"), Some(outpost.clone()));
        assert!(
            Placement::parse("Bb5 b4").is_some(),
            "a bishop and a pawn on the b-file"
        );
        assert!(
            Placement::parse("vs e6").is_some(),
            "the second side's alone"
        );
        assert_eq!(Placement::parse("Nd5, e4"), Placement::parse("Nd5 e4"));
        for refused in [
            "",
            "vs",
            "Nd9",
            "Xd5",
            "Nd5 vs vs e6",
            "e8",
            "Nd5 vs Bd5",
            "d5d6",
        ] {
            assert_eq!(Placement::parse(refused), None, "{refused:?}");
        }

        // A white knight on d5 against a black pawn on e6, or the same with the colours swapped:
        // a black knight on d4 against a white pawn on e3.
        let white = board("4k3/8/4p3/3N4/8/8/8/4K3 w - - 0 1");
        let black = board("4k3/8/8/8/3n4/4P3/8/4K3 w - - 0 1");
        assert!(outpost.on(&white));
        assert!(outpost.on(&black));
        // Each side's own pieces: a black knight on d5 is not the outpost.
        assert!(!outpost.on(&board("4k3/8/4p3/3n4/8/8/8/4K3 w - - 0 1")));
        assert!(!outpost.on(&board("4k3/8/8/3N4/8/8/8/4K3 w - - 0 1")));
    }

    #[test]
    fn a_balance_is_written_as_each_side_s_pieces() {
        let rook_bishop = Material::parse("R vs B").unwrap();
        assert_eq!(Material::parse("krvkb"), Some(rook_bishop));
        assert_eq!(Material::parse("R-B"), Some(rook_bishop));
        assert_eq!(Material::parse(" r v b "), Some(rook_bishop));
        for refused in ["", "R", "R vs X", "v", "RvBvN", "QQQQQQQQQQQ v K"] {
            assert_eq!(Material::parse(refused), None, "{refused:?}");
        }
        assert!(Material::parse("Q v K").is_some());

        // Rook against bishop, pawns aside, with either colour.
        let white_rook = board("4k3/pp3b2/8/8/8/8/PPP5/3RK3 w - - 0 1");
        let black_rook = board("3rk3/8/8/8/8/8/5B2/4K3 w - - 0 1");
        assert!(rook_bishop.on(&white_rook));
        assert!(rook_bishop.on(&black_rook));
        assert!(!rook_bishop.on(&board("4k3/5b2/8/8/8/8/8/2RRK3 w - - 0 1")));
        // Named pawns count.
        let philidor = Material::parse("KRP vs KR").unwrap();
        assert!(philidor.on(&board("4k3/8/8/8/4P3/8/r7/4K2R w - - 0 1")));
        assert!(!philidor.on(&board("4k3/8/8/8/4P3/3P4/r7/4K2R w - - 0 1")));
    }

    #[test]
    fn each_structure_is_found_for_either_colour() {
        let iqp = "r1bq1rk1/pp2bppp/2n1pn2/8/3P4/2NB1N2/PP3PPP/R1BQ1RK1 w - - 0 1";
        assert!(Structure::IsolatedQueenPawn.on(&board(iqp)));
        let black_iqp = "r1bq1rk1/pp3ppp/2n2n2/3p4/8/2N1PN2/PP3PPP/R1BQ1RK1 w - - 0 1";
        assert!(Structure::IsolatedQueenPawn.on(&board(black_iqp)));
        let hanging = "r1bq1rk1/pp3ppp/4pn2/8/2PP4/5N2/P4PPP/R1BQ1RK1 w - - 0 1";
        assert!(Structure::HangingPawns.on(&board(hanging)));
        assert!(!Structure::HangingPawns.on(&board(iqp)));
        let carlsbad = "r1bq1rk1/pp2bppp/2p2n2/3p4/3P4/2N1P3/PP3PPP/R1BQKBNR w - - 0 1";
        assert!(Structure::Carlsbad.on(&board(carlsbad)));
        let maroczy = "r1bqk2r/pp2ppbp/2np1np1/8/2P1P3/2N1B3/PP2BPPP/R2QK1NR w - - 0 1";
        assert!(Structure::MaroczyBind.on(&board(maroczy)));
        let dutch = "rnbq1rk1/pp2b1pp/2p1pn2/3p1p2/2PP4/5NP1/PP2PPBP/RNBQ1RK1 w - - 0 1";
        assert!(Structure::Stonewall.on(&board(dutch)));
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        for structure in [
            Structure::IsolatedQueenPawn,
            Structure::HangingPawns,
            Structure::Carlsbad,
            Structure::MaroczyBind,
            Structure::Stonewall,
        ] {
            assert!(!structure.on(&board(start)), "{structure:?}");
        }
    }

    #[test]
    fn a_balance_counts_once_it_lasts() {
        use crate::gamedb::{encode_line, start_of};
        use shakmaty::uci::UciMove;
        let start = start_of(None).unwrap();
        let line = |ucis: &[&str]| {
            let mut pos = start.clone();
            let moves: Vec<shakmaty::Move> = ucis
                .iter()
                .map(|uci| {
                    let m = uci.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
                    pos.play_unchecked(m);
                    m
                })
                .collect();
            encode_line(&start, &moves).unwrap()
        };
        // 1. e4 d5 2. exd5 Qxd5: a pawn up for White for one ply only.
        let traded = line(&["e2e4", "d7d5", "e4d5", "d8d5", "g1f3", "g8f6"]);
        let pawn_up = |board: &Board| {
            board.by_piece(Role::Pawn.of(Color::White)).count()
                > board.by_piece(Role::Pawn.of(Color::Black)).count()
        };
        assert_eq!(
            reaches(&start, &traded, changes_material, pawn_up),
            Some(false)
        );
        // 1. e4 d5 2. exd5 Nf6 3. Nc3 c6 4. Nf3: the pawn kept for four plies.
        let kept = line(&["e2e4", "d7d5", "e4d5", "g8f6", "b1c3", "c7c6", "g1f3"]);
        assert_eq!(
            reaches(&start, &kept, changes_material, pawn_up),
            Some(true)
        );
        // Reached on the last move: the game ends with it.
        let last = line(&["e2e4", "d7d5", "e4d5"]);
        assert_eq!(
            reaches(&start, &last, changes_material, pawn_up),
            Some(true)
        );
        assert_eq!(reaches(&start, &[250], changes_material, pawn_up), None);
    }

    #[test]
    fn a_time_control_has_its_speed() {
        assert_eq!(speed("180+2"), Some(Speed::Blitz));
        assert_eq!(speed("60+0"), Some(Speed::Bullet));
        assert_eq!(speed("15"), Some(Speed::UltraBullet));
        assert_eq!(speed("600+5"), Some(Speed::Rapid));
        assert_eq!(speed("5400+30"), Some(Speed::Classical));
        assert_eq!(speed("40/7200:1800+30"), Some(Speed::Classical));
        assert_eq!(speed("-"), Some(Speed::Correspondence));
        assert_eq!(speed("?"), None);
    }
}
