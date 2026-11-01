//! What a move did, in facts a person can check: the material an engine line wins and the tactics
//! that win it. The line is the proof and this only names it, so a pattern is reported only when
//! the line goes on to collect from it. A pin the line ignores is not a fact.
//!
//! Standard chess only: the facts feed explanations of reviewed games and puzzles.

use serde::Serialize;
use shakmaty::attacks;
use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Color, Move, Piece, Position, Role, Square};

use crate::rules;

/// The plies of a line read: long enough for a combination, short of the engine's far horizon,
/// where its moves stop being what the position is about.
const LINE_PLIES: usize = 10;

/// The tactics a line can be credited with.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Tactic {
    /// `square`'s piece was taken for nothing.
    #[serde(rename_all = "camelCase")]
    Hanging {
        role: &'static str,
        square: String,
        san: String,
    },
    /// `san` attacked `targets` at once, and the line took `won`.
    #[serde(rename_all = "camelCase")]
    Fork {
        role: &'static str,
        square: String,
        san: String,
        targets: Vec<String>,
        won: String,
    },
    /// The piece on `pinned` could not leave the line to `behind`, and was taken.
    #[serde(rename_all = "camelCase")]
    Pin {
        role: &'static str,
        square: String,
        pinned: String,
        behind: String,
    },
}

/// What a line achieves for the side it favours.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LineFacts {
    /// The line in SAN, from its first move.
    pub san: Vec<String>,
    /// Material won, in pawns, by the line's end (never counted mid-exchange).
    pub material: i32,
    pub tactics: Vec<Tactic>,
}

/// What a played move did, and what the best move would have done.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MoveFacts {
    pub san: String,
    /// The opponent's punishment of the played move.
    pub refutation: Option<LineFacts>,
    /// The best move's line, unless the best move was the one played.
    pub better: Option<LineFacts>,
    /// Forced mate for the opponent after the move, in moves.
    pub mated_in: Option<u32>,
    /// The forced mate the mover had before it, in moves, when the move lost it.
    pub missed_mate: Option<u32>,
}

/// The scores a move is judged on, from White's side, as a review stores them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mates {
    pub before: Option<i32>,
    pub after: Option<i32>,
}

fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
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

/// `color`'s material, in pawns.
fn material(pos: &VariantPosition, color: Color) -> i32 {
    let board = pos.board();
    Role::ALL
        .iter()
        .map(|&role| {
            let piece = Piece { color, role };
            value(role) * board.by_piece(piece).count() as i32
        })
        .sum()
}

/// `color`'s lead in material.
fn lead(pos: &VariantPosition, color: Color) -> i32 {
    material(pos, color) - material(pos, color.other())
}

/// A piece of `color` on `square`, as `Qe4`.
fn named(pos: &VariantPosition, square: Square) -> Option<(Role, String)> {
    let piece = pos.board().piece_at(square)?;
    let letter = match piece.role {
        Role::Pawn => "",
        role => role_letter(role),
    };
    Some((piece.role, format!("{letter}{square}")))
}

fn role_letter(role: Role) -> &'static str {
    match role {
        Role::Pawn => "",
        Role::Knight => "N",
        Role::Bishop => "B",
        Role::Rook => "R",
        Role::Queen => "Q",
        Role::King => "K",
    }
}

/// One move of a line, with the position it left.
struct Step {
    mover: Color,
    played: Move,
    san: String,
    after: VariantPosition,
}

/// `line` from `start` while it stays legal, up to `LINE_PLIES`.
fn walk<S: AsRef<str>>(start: &VariantPosition, line: &[S]) -> Vec<Step> {
    let mut pos = start.clone();
    let mut steps = Vec::new();
    for text in line.iter().take(LINE_PLIES) {
        let Some(played) =
            rules::parse_uci(text.as_ref()).and_then(|uci| rules::legal_move(&pos, uci))
        else {
            break;
        };
        let mover = pos.turn();
        let san = rules::san_and_play(&mut pos, played);
        steps.push(Step {
            mover,
            played,
            san,
            after: pos.clone(),
        });
    }
    steps
}

/// `side`'s gain over `steps`. A line ending on `side`'s capture has not shown the recapture, so
/// a capture the other side could still answer is not counted.
fn settled_gain(start: &VariantPosition, steps: &[Step], side: Color) -> i32 {
    let open = steps.last().is_some_and(|last| {
        let board = last.after.board();
        last.mover == side
            && last.played.is_capture()
            && !board
                .attacks_to(last.played.to(), side.other(), board.occupied())
                .is_empty()
    });
    let settled = steps.len() - usize::from(open);
    let end = settled
        .checked_sub(1)
        .map_or(start, |last| &steps[last].after);
    lead(end, side) - lead(start, side)
}

/// Whether `steps` from `from` on take a piece of the other side on `square` for `side`.
fn taken_on(steps: &[Step], from: usize, square: Square, side: Color) -> bool {
    steps
        .iter()
        .skip(from)
        .any(|step| step.mover == side && step.played.is_capture() && step.played.to() == square)
}

/// A capture of `victim` by `side` that the line leaves standing: not answered on that square.
fn hanging(start: &VariantPosition, steps: &[Step], side: Color, gain: i32) -> Option<Tactic> {
    let index = steps
        .iter()
        .position(|step| step.mover == side && step.played.is_capture())?;
    // Only the line's first blow: later ones are the combination's, not a loose piece's.
    if index > 1 {
        return None;
    }
    let step = &steps[index];
    let square = step.played.to();
    let before = if index == 0 {
        start
    } else {
        &steps[index - 1].after
    };
    let (role, _) = named(before, square)?;
    let answered = steps
        .get(index + 1)
        .is_some_and(|reply| reply.played.is_capture() && reply.played.to() == square);
    (!answered && gain >= value(role)).then(|| Tactic::Hanging {
        role: role_name(role),
        square: square.to_string(),
        san: step.san.clone(),
    })
}

/// The pieces of `victim` that `square`'s piece attacks and could profitably take: the king, a
/// bigger piece, or one left undefended.
fn real_targets(pos: &VariantPosition, square: Square, victim: Color) -> Vec<Square> {
    let board = pos.board();
    let Some(attacker) = board.piece_at(square) else {
        return Vec::new();
    };
    (board.attacks_from(square) & board.by_color(victim))
        .into_iter()
        .filter(|&target| {
            let Some(piece) = board.piece_at(target) else {
                return false;
            };
            piece.role != Role::Pawn
                && (piece.role == Role::King
                    || value(piece.role) > value(attacker.role)
                    || board
                        .attacks_to(target, victim, board.occupied())
                        .is_empty())
        })
        .collect()
}

/// A move of `side` attacking two targets at once, one of which the line then takes.
fn fork(steps: &[Step], side: Color, gain: i32) -> Option<Tactic> {
    if gain <= 0 {
        return None;
    }
    steps.iter().enumerate().find_map(|(index, step)| {
        if step.mover != side {
            return None;
        }
        let square = step.played.to();
        let targets = real_targets(&step.after, square, side.other());
        if targets.len() < 2 {
            return None;
        }
        let won = targets
            .iter()
            .copied()
            .find(|&target| taken_on(steps, index + 1, target, side))?;
        let (role, _) = named(&step.after, square)?;
        Some(Tactic::Fork {
            role: role_name(role),
            square: square.to_string(),
            san: step.san.clone(),
            targets: targets
                .iter()
                .filter_map(|&target| named(&step.after, target).map(|(_, name)| name))
                .collect(),
            won: named(&step.after, won)?.1,
        })
    })
}

/// In `pos`, a slider of `side` looking through one enemy piece at the enemy's king or queen.
fn pins(pos: &VariantPosition, side: Color) -> Vec<(Square, Square, Square)> {
    let board = pos.board();
    let occupied = board.occupied();
    let enemy = board.by_color(side.other());
    let mut found = Vec::new();
    for slider in board.by_color(side) {
        let Some(piece) = board.piece_at(slider) else {
            continue;
        };
        for behind in enemy {
            let Some(back) = board.piece_at(behind) else {
                continue;
            };
            if !matches!(back.role, Role::King | Role::Queen) {
                continue;
            }
            if !attacks::attacks(slider, piece, Bitboard::EMPTY).contains(behind) {
                continue;
            }
            let Some(pinned) = (attacks::between(slider, behind) & occupied).single_square() else {
                continue;
            };
            let Some(front) = board.piece_at(pinned) else {
                continue;
            };
            if front.color != side
                && (back.role == Role::King || value(front.role) < value(back.role))
            {
                found.push((slider, pinned, behind));
            }
        }
    }
    found
}

/// A pin of `side`'s, standing in the line, whose pinned piece the line then takes.
fn pin(start: &VariantPosition, steps: &[Step], side: Color, gain: i32) -> Option<Tactic> {
    if gain <= 0 {
        return None;
    }
    let positions = std::iter::once((0, start)).chain(
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| (index + 1, &step.after)),
    );
    for (next, pos) in positions.take(4) {
        for (slider, pinned, behind) in pins(pos, side) {
            if taken_on(steps, next, pinned, side)
                && let (Some((role, _)), Some((_, pinned)), Some((_, behind))) =
                    (named(pos, slider), named(pos, pinned), named(pos, behind))
            {
                return Some(Tactic::Pin {
                    role: role_name(role),
                    square: slider.to_string(),
                    pinned,
                    behind,
                });
            }
        }
    }
    None
}

/// What `line`, played from `start`, wins for `side`.
pub fn line_facts<S: AsRef<str>>(
    start: &VariantPosition,
    line: &[S],
    side: Color,
) -> Option<LineFacts> {
    let steps = walk(start, line);
    if steps.is_empty() {
        return None;
    }
    let gain = settled_gain(start, &steps, side);
    // A fork or pin says more than the piece it won, so it goes first and speaks for that piece.
    let mut tactics: Vec<Tactic> = [fork(&steps, side, gain), pin(start, &steps, side, gain)]
        .into_iter()
        .flatten()
        .collect();
    if tactics.is_empty() {
        tactics.extend(hanging(start, &steps, side, gain));
    }
    Some(LineFacts {
        san: steps.into_iter().map(|step| step.san).collect(),
        material: gain,
        tactics,
    })
}

/// The moves until mate `score` (positive: White mates) means for `side`, when `side` mates.
fn mate_for(score: Option<i32>, side: Color) -> Option<u32> {
    let score = score.filter(|&score| score != 0)?;
    let white_mates = score > 0;
    (white_mates == (side == Color::White)).then(|| score.unsigned_abs())
}

/// What `played` (UCI) did in `fen`. `best` starts with the engine's move there, `refutation` is
/// the engine's line after `played`; either may be empty, as when Lichess did the analysis.
pub fn move_facts<S: AsRef<str>, T: AsRef<str>>(
    fen: &str,
    played: &str,
    best: &[S],
    refutation: &[T],
    mates: Mates,
) -> Option<MoveFacts> {
    let start = rules::standard_from_fen(fen)?;
    let mover = start.turn();
    let chosen = rules::parse_uci(played).and_then(|uci| rules::legal_move(&start, uci))?;
    let mut after = start.clone();
    let san = rules::san_and_play(&mut after, chosen);
    let same = best.first().is_some_and(|first| first.as_ref() == played);
    let mated_in = mate_for(mates.after, mover.other());
    let had_mate = mate_for(mates.before, mover);
    Some(MoveFacts {
        san,
        refutation: line_facts(&after, refutation, mover.other()),
        better: if same {
            None
        } else {
            line_facts(&start, best, mover)
        },
        mated_in,
        missed_mate: had_mate.filter(|_| mate_for(mates.after, mover).is_none()),
    })
}

/// What a puzzle's solution wins: `moves` from `fen` as the puzzle plays them, the opponent's
/// setting-up move first.
pub fn puzzle_facts<S: AsRef<str>>(fen: &str, moves: &[S]) -> Option<LineFacts> {
    let mut start = rules::standard_from_fen(fen)?;
    let (first, solution) = moves.split_first()?;
    let setup = rules::parse_uci(first.as_ref()).and_then(|uci| rules::legal_move(&start, uci))?;
    rules::san_and_play(&mut start, setup);
    let side = start.turn();
    line_facts(&start, solution, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(facts: &LineFacts) -> Vec<&'static str> {
        facts
            .tactics
            .iter()
            .map(|tactic| match tactic {
                Tactic::Hanging { .. } => "hanging",
                Tactic::Fork { .. } => "fork",
                Tactic::Pin { .. } => "pin",
            })
            .collect()
    }

    fn facts(fen: &str, line: &[&str]) -> LineFacts {
        let start = rules::standard_from_fen(fen).unwrap();
        let side = start.turn();
        line_facts(&start, line, side).unwrap()
    }

    #[test]
    fn a_knight_fork_is_credited_when_the_line_collects() {
        // Nc7+ forks the king on e8 and the rook on a8.
        let line = facts(
            "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1",
            &["b5c7", "e8d7", "c7a8"],
        );
        assert_eq!(line.material, 5);
        assert_eq!(kinds(&line), ["fork"]);
        let Tactic::Fork { targets, won, .. } = &line.tactics[0] else {
            panic!("{line:?}");
        };
        assert_eq!(targets, &["Ra8", "Ke8"]);
        assert_eq!(won, "Ra8");
    }

    #[test]
    fn a_double_attack_the_line_ignores_is_not_a_fork() {
        let line = facts(
            "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/4K3 w - - 0 1",
            &["b5c7", "e8d7", "c7b5"],
        );
        assert_eq!(line.material, 0);
        assert!(line.tactics.is_empty(), "{line:?}");
    }

    #[test]
    fn pawns_and_defended_equals_are_no_fork_targets() {
        let pawns = rules::standard_from_fen("4k3/8/2p1p3/1p3p2/3N4/8/8/4K3 w - - 0 1").unwrap();
        assert!(real_targets(&pawns, Square::D4, Color::Black).is_empty());
        let loose = rules::standard_from_fen("4k3/p7/5n2/8/4N3/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(real_targets(&loose, Square::E4, Color::Black), [Square::F6]);
        let defended = rules::standard_from_fen("4k3/6p1/5n2/8/4N3/8/8/4K3 w - - 0 1").unwrap();
        assert!(real_targets(&defended, Square::E4, Color::Black).is_empty());
    }

    #[test]
    fn a_hanging_piece_is_a_capture_the_line_leaves_standing() {
        let line = facts("4k3/8/8/3q4/8/8/8/3RK3 w - - 0 1", &["d1d5"]);
        assert_eq!(line.material, 9);
        assert_eq!(kinds(&line), ["hanging"]);
    }

    #[test]
    fn a_line_ending_on_an_answerable_capture_counts_nothing_for_it() {
        let line = facts("4k3/4n3/8/3q4/8/8/8/3RK3 w - - 0 1", &["d1d5"]);
        assert_eq!(line.material, 0);
        assert!(line.tactics.is_empty(), "{line:?}");
    }

    #[test]
    fn a_recaptured_piece_is_not_hanging() {
        let line = facts(
            "4k3/8/4p3/3n4/4P3/8/8/4K3 w - - 0 1",
            &["e4d5", "e6d5", "e1d2"],
        );
        assert_eq!(line.material, 2);
        assert!(line.tactics.is_empty(), "{line:?}");
    }

    #[test]
    fn a_pin_is_credited_when_the_pinned_piece_falls() {
        // The bishop pins the knight to the king; the pawn takes it.
        let line = facts(
            "4k3/8/2n5/1B1P4/8/8/8/4K3 w - - 0 1",
            &["d5c6", "e8e7", "e1d2"],
        );
        assert_eq!(line.material, 3);
        assert_eq!(kinds(&line), ["pin"]);
    }

    #[test]
    fn moves_are_judged_from_both_sides_of_the_board() {
        // White walks the queen next to a pawn.
        let facts = move_facts(
            "4k3/8/8/4p3/8/8/8/3QK3 w - - 0 1",
            "d1d4",
            &["e1e2"],
            &["e5d4", "e1e2"],
            Mates::default(),
        )
        .unwrap();
        assert_eq!(facts.san, "Qd4");
        let refutation = facts.refutation.unwrap();
        assert_eq!(refutation.material, 9);
        assert_eq!(kinds(&refutation), ["hanging"]);
        assert_eq!(facts.better.unwrap().san, ["Ke2"]);
    }

    #[test]
    fn the_best_move_has_nothing_better() {
        let facts = move_facts(
            "4k3/8/8/4p3/8/8/8/3QK3 w - - 0 1",
            "e1e2",
            &["e1e2"],
            &["e8e7"],
            Mates::default(),
        )
        .unwrap();
        assert_eq!(facts.better, None);
    }

    #[test]
    fn mates_read_from_the_side_that_mates() {
        let mated = move_facts(
            "4k3/8/8/8/8/8/8/4K2R b - - 0 1",
            "e8d8",
            &["e8e7"],
            &[] as &[&str],
            Mates {
                before: None,
                after: Some(3),
            },
        )
        .unwrap();
        assert_eq!(mated.mated_in, Some(3));
        let missed = move_facts(
            "4k3/8/8/8/8/8/8/4K2R w - - 0 1",
            "e1e2",
            &["h1h8"],
            &[] as &[&str],
            Mates {
                before: Some(4),
                after: Some(0),
            },
        )
        .unwrap();
        assert_eq!(missed.missed_mate, Some(4));
        // Still mating after the move: nothing was missed.
        let kept = move_facts(
            "4k3/8/8/8/8/8/8/4K2R w - - 0 1",
            "e1e2",
            &["h1h8"],
            &[] as &[&str],
            Mates {
                before: Some(4),
                after: Some(5),
            },
        )
        .unwrap();
        assert_eq!(kept.missed_mate, None);
    }

    #[test]
    fn an_illegal_line_stops_where_it_stops_being_legal() {
        let line = facts("4k3/8/8/8/8/8/8/4K2R w - - 0 1", &["h1h8", "a1a2", "e8e7"]);
        assert_eq!(line.san, ["Rh8+"]);
        assert!(
            move_facts(
                "not a fen",
                "e2e4",
                &[] as &[&str],
                &[] as &[&str],
                Mates::default()
            )
            .is_none()
        );
        assert!(
            move_facts(
                "4k3/8/8/8/8/8/8/4K2R w - - 0 1",
                "a1a2",
                &[] as &[&str],
                &[] as &[&str],
                Mates::default()
            )
            .is_none()
        );
    }

    #[test]
    fn a_puzzle_is_read_after_its_setting_up_move() {
        let line = puzzle_facts(
            "r3k3/pp3ppp/8/1N6/8/8/PPP2PPP/3K4 w - - 0 1",
            &["d1e1", "a8d8", "b5c7"],
        );
        // White moved first, so the solver is Black.
        assert_eq!(line.unwrap().san, ["Rd8", "Nc7+"]);
    }
}
