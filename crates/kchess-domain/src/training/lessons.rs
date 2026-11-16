//! Interactive lessons: the checkmate and endgame courses
//! (`data/lessons.json`), and how a lesson step stands after a line of moves.
//!
//! A step is read, followed (`line`: your moves alternate with scripted replies), solved
//! (`mate`: force mate within a few moves; replies come from the solver below, so these work
//! without an engine) or played out against the engine (`play`, with a goal).

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Color, Move, Position, Role, Square, variant::VariantPosition};
use std::sync::OnceLock;

use crate::rules;

const LESSONS_JSON: &str = include_str!("../../data/lessons.json");

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub id: String,
    pub group: String,
    pub title: String,
    pub subtitle: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub fen: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    pub task: Task,
}

/// An arrow (`dest`) or a circled square on the board.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    pub orig: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest: Option<String>,
    pub brush: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Task {
    /// Nothing to play.
    Read,
    /// UCI moves from the step's position: yours at even indices, the scripted replies between.
    /// Your last move may be any checkmate.
    Line { moves: Vec<String> },
    /// Force checkmate within `within` of your moves.
    Mate { within: u32 },
    /// Play the goal out against the engine within `within` of your moves.
    Play {
        goal: Goal,
        within: u32,
        /// `reach`: where your king must stand; `confine`: the box the enemy king must be kept in.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        squares: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Goal {
    /// Checkmate.
    Mate,
    /// Checkmate, a promotion the opponent cannot capture, or a bare enemy king against your
    /// queen or rook.
    Win,
    /// Reach a drawn end, or last `within` moves without being lost.
    Draw,
    /// Your king on one of `squares`, without losing material.
    Reach,
    /// The enemy king and every square it may go to inside `squares`, without losing material.
    Confine,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Data {
    pub courses: Vec<Course>,
}

/// The courses.
pub fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| crate::embedded::parse("lessons", LESSONS_JSON))
}

/// Why a step ended, or what the side to move waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// Read steps end at once.
    Read,
    /// Every move of the line was played.
    LineDone,
    Checkmate,
    /// You were checkmated.
    Mated,
    Stalemate,
    Insufficient,
    Repetition,
    FiftyMoves,
    /// A promotion the opponent cannot take back.
    Promoted,
    /// The enemy king has nothing but a bare king against your queen or rook.
    BareKing,
    Reached,
    Confined,
    /// A draw goal lasted its moves.
    Held,
    /// Out of moves for the goal.
    OutOfMoves,
    /// You lost material the goal does not allow.
    MaterialLost,
    /// The opponent won material, a promotion or a won ending.
    Lost,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub over: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    /// The side that solves the step: the side to move in its position.
    pub player: &'static str,
    pub turn: &'static str,
    pub fen: String,
    pub check: bool,
    /// Your moves played so far, and how many the task allows (`within`), if it limits them.
    pub played: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// The opponent's next move when the lesson knows it (a scripted or solved reply).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// The opponent is to move and the engine must answer.
    pub engine: bool,
}

const INVALID: &str = "Invalid lesson position.";

fn chess(fen: &str) -> Result<Chess, String> {
    match rules::standard_from_fen(fen) {
        Some(VariantPosition::Chess(pos)) => Ok(pos),
        _ => Err(INVALID.into()),
    }
}

fn legal(pos: &Chess, uci: &str) -> Option<Move> {
    let parsed = rules::parse_uci(uci)?;
    rules::legal_move(&VariantPosition::Chess(pos.clone()), parsed)
}

fn uci_of(m: &Move) -> String {
    rules::make_uci(m)
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

fn material(pos: &Chess, color: Color) -> i32 {
    let board = pos.board();
    Role::ALL
        .iter()
        .map(|&role| value(role) * (board.by_color(color) & board.by_role(role)).count() as i32)
        .sum()
}

/// Only a king left for `color`, against a queen or rook of the other side.
fn bare_king_against_major(pos: &Chess, color: Color) -> bool {
    let board = pos.board();
    let theirs = board.by_color(color);
    let ours = board.by_color(!color);
    theirs.count() == 1 && !(ours & (board.queens() | board.rooks())).is_empty()
}

fn fen_of(pos: &Chess) -> String {
    rules::make_fen(&VariantPosition::Chess(pos.clone()))
}

/* ── Forced mate ─────────────────────────────────────────────────────────────────────────── */

/// The side to move forces checkmate within `n` of its moves.
pub fn mates_within(pos: &Chess, n: u32) -> bool {
    if n == 0 {
        return false;
    }
    let mut quiet = Vec::new();
    for m in pos.legal_moves() {
        let mut next = pos.clone();
        next.play_unchecked(m);
        if next.is_check() {
            if next.is_checkmate() || (n > 1 && defender_lost(&next, n - 1)) {
                return true;
            }
        } else if n > 1 {
            quiet.push(next);
        }
    }
    // Only a check mates at once; quiet moves need a reply first.
    quiet
        .iter()
        .any(|next| !next.legal_moves().is_empty() && defender_lost(next, n - 1))
}

/// Every reply of the side to move (not checkmated) still allows mate within `n`.
fn defender_lost(pos: &Chess, n: u32) -> bool {
    let replies = pos.legal_moves();
    !replies.is_empty()
        && replies.iter().all(|m| {
            let mut next = pos.clone();
            next.play_unchecked(*m);
            mates_within(&next, n)
        })
}

/// The fewest moves (up to `limit`) in which the side to move forces mate.
fn mate_distance(pos: &Chess, limit: u32) -> Option<u32> {
    (1..=limit).find(|&n| mates_within(pos, n))
}

/// Each legal move with the position after it, in UCI order.
fn successors(pos: &Chess) -> Vec<(String, Chess)> {
    let mut out: Vec<(String, Chess)> = pos
        .legal_moves()
        .into_iter()
        .map(|m| {
            let mut next = pos.clone();
            next.play_unchecked(m);
            (uci_of(&m), next)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The defence that delays mate longest (looking `limit` moves ahead), the first in UCI order on
/// a tie; a reply that escapes mate altogether beats all of them.
fn best_defence(pos: &Chess, limit: u32) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for (uci, next) in successors(pos) {
        let resist = mate_distance(&next, limit).unwrap_or(limit + 1);
        if best.as_ref().is_none_or(|(b, _)| resist > *b) {
            best = Some((resist, uci));
        }
    }
    best.map(|(_, uci)| uci)
}

/// The quickest mating move for the side to move, the first in UCI order on a tie.
fn quickest_mate(pos: &Chess, limit: u32) -> Option<String> {
    let moves = successors(pos);
    for n in 1..=limit {
        for (uci, next) in &moves {
            if next.is_checkmate() || (n > 1 && defender_lost(next, n - 1)) {
                return Some(uci.clone());
            }
        }
    }
    None
}

/* ── Steps ───────────────────────────────────────────────────────────────────────────────── */

struct Replay {
    pos: Chess,
    player: Color,
    /// Your moves played.
    played: u32,
    repeated: bool,
    /// Material you lost to captures, and the opponent's net gain against you.
    lost_material: bool,
    swing: i32,
    /// Your last move promoted on this square.
    promoted_on: Option<Square>,
}

fn replay<S: AsRef<str>>(fen: &str, moves: &[S]) -> Result<Replay, String> {
    let start = chess(fen)?;
    let player = start.turn();
    let balance = |pos: &Chess| material(pos, player) - material(pos, !player);
    let initial_balance = balance(&start);
    let initial_ours = material(&start, player);
    let mut pos = start.clone();
    let mut seen = rules::Repetitions::starting_at(&fen_of(&pos));
    let mut out = Replay {
        pos: start,
        player,
        played: 0,
        repeated: false,
        lost_material: false,
        swing: 0,
        promoted_on: None,
    };
    for uci in moves {
        let Some(m) = legal(&pos, uci.as_ref()) else {
            break;
        };
        let mover = pos.turn();
        out.promoted_on = (mover == player && m.promotion().is_some()).then(|| m.to());
        pos.play_unchecked(m);
        if mover == player {
            out.played += 1;
        }
        out.repeated |= seen.reached(&fen_of(&pos));
        // Judged after your moves, so a capture you take straight back is no loss. Promotions
        // add material, so only a drop below the start counts.
        if mover == player {
            out.lost_material |= material(&pos, player) < initial_ours;
            out.swing = out.swing.max(initial_balance - balance(&pos));
        }
    }
    out.pos = pos;
    Ok(out)
}

fn finish(success: bool, reason: Reason) -> (bool, Option<bool>, Option<Reason>) {
    (true, Some(success), Some(reason))
}

/// Where `task`, from `fen`, stands after `moves` (UCI).
pub fn status<S: AsRef<str>>(fen: &str, task: &Task, moves: &[S]) -> Result<Status, String> {
    let r = replay(fen, moves)?;
    let pos = &r.pos;
    let turn = pos.turn();
    let ours = turn == r.player;
    let limit = match task {
        Task::Mate { within } | Task::Play { within, .. } => Some(*within),
        _ => None,
    };
    let no_moves = pos.legal_moves().is_empty();
    let drawn = if !no_moves || pos.is_check() {
        if pos.is_insufficient_material() {
            Some(Reason::Insufficient)
        } else if r.repeated {
            Some(Reason::Repetition)
        } else if pos.halfmoves() >= 100 {
            Some(Reason::FiftyMoves)
        } else {
            None
        }
    } else {
        Some(Reason::Stalemate)
    };
    let mated = pos.is_checkmate();

    let (over, success, reason) = match task {
        Task::Read => finish(true, Reason::Read),
        Task::Line { moves: line } => {
            if mated && !ours {
                finish(true, Reason::Checkmate)
            } else if moves.len() >= line.len() {
                finish(true, Reason::LineDone)
            } else {
                (false, None, None)
            }
        }
        Task::Mate { within } => {
            if mated {
                finish(
                    !ours,
                    if ours {
                        Reason::Mated
                    } else {
                        Reason::Checkmate
                    },
                )
            } else if let Some(drawn) = drawn {
                finish(false, drawn)
            } else if ours && r.played >= *within {
                finish(false, Reason::OutOfMoves)
            } else {
                (false, None, None)
            }
        }
        Task::Play {
            goal,
            within,
            squares,
        } => play_status(&r, *goal, *within, squares, mated, drawn),
    };

    let reply = if over || ours {
        None
    } else {
        match task {
            Task::Line { moves: line } => line.get(moves.len()).cloned(),
            Task::Mate { within } => best_defence(pos, within.saturating_sub(r.played)),
            _ => None,
        }
    };
    let engine = !over && !ours && matches!(task, Task::Play { .. });
    Ok(Status {
        over,
        success,
        reason,
        player: crate::color::name(r.player),
        turn: crate::color::name(turn),
        fen: rules::make_fen(&VariantPosition::Chess(pos.clone())),
        check: pos.is_check(),
        played: r.played,
        limit,
        reply,
        engine,
    })
}

fn squares_of(names: &[String]) -> Vec<Square> {
    names
        .iter()
        .filter_map(|s| rules::parse_square(s))
        .collect()
}

fn play_status(
    r: &Replay,
    goal: Goal,
    within: u32,
    squares: &[String],
    mated: bool,
    drawn: Option<Reason>,
) -> (bool, Option<bool>, Option<Reason>) {
    let pos = &r.pos;
    let ours = pos.turn() == r.player;
    if mated {
        return if ours {
            finish(false, Reason::Mated)
        } else {
            finish(goal != Goal::Draw, Reason::Checkmate)
        };
    }
    if let Some(drawn) = drawn {
        return finish(goal == Goal::Draw, drawn);
    }
    // Also judged after your move: with the move you might still take back what was lost.
    let lost = !ours && (r.swing >= 3 || bare_king_against_major(pos, r.player));
    match goal {
        Goal::Draw => {
            if lost {
                return finish(false, Reason::Lost);
            }
            if ours && r.played >= within {
                return finish(true, Reason::Held);
            }
        }
        Goal::Win => {
            if let Some(square) = r.promoted_on.filter(|_| !ours) {
                let safe = !pos.legal_moves().iter().any(|m| m.to() == square);
                if safe {
                    return finish(true, Reason::Promoted);
                }
            }
            let majors =
                pos.board().by_color(r.player) & (pos.board().queens() | pos.board().rooks());
            let safe = !pos.legal_moves().iter().any(|m| majors.contains(m.to()));
            if !ours && safe && bare_king_against_major(pos, !r.player) {
                return finish(true, Reason::BareKing);
            }
            if lost {
                return finish(false, Reason::Lost);
            }
        }
        Goal::Mate | Goal::Reach | Goal::Confine => {
            if r.lost_material {
                return finish(false, Reason::MaterialLost);
            }
            if !ours {
                let region = squares_of(squares);
                if goal == Goal::Reach {
                    let king = pos.board().king_of(r.player);
                    if king.is_some_and(|k| region.contains(&k)) {
                        return finish(true, Reason::Reached);
                    }
                } else if goal == Goal::Confine && confined(pos, &region) {
                    return finish(true, Reason::Confined);
                }
            }
        }
    }
    if ours && r.played >= within {
        return finish(false, Reason::OutOfMoves);
    }
    (false, None, None)
}

/// The king of the side to move stands in `region`, and every move it has stays there.
fn confined(pos: &Chess, region: &[Square]) -> bool {
    let Some(king) = pos.board().king_of(pos.turn()) else {
        return false;
    };
    region.contains(&king)
        && pos
            .legal_moves()
            .iter()
            .filter(|m| m.role() == Role::King)
            .all(|m| region.contains(&m.to()))
}

/// The answer `lessonMove` gives a move you try.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub accepted: bool,
    /// The move as the board plays it (castling as king to rook's square normalized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uci: Option<String>,
}

/// Whether `uci`, your move after `moves`, is one the step accepts. A line wants its
/// next move (or any mate as its last); a mate task wants a move that keeps a forced mate within
/// the moves left; play tasks take any legal move.
pub fn try_move<S: AsRef<str>>(
    fen: &str,
    task: &Task,
    moves: &[S],
    uci: &str,
) -> Result<Verdict, String> {
    let r = replay(fen, moves)?;
    let rejected = Verdict {
        accepted: false,
        uci: None,
    };
    if r.pos.turn() != r.player || matches!(task, Task::Read) {
        return Ok(rejected);
    }
    let Some(m) = legal(&r.pos, uci) else {
        return Ok(rejected);
    };
    let played = uci_of(&m);
    let mut next = r.pos.clone();
    next.play_unchecked(m);
    let accepted = match task {
        Task::Read => false,
        Task::Line { moves: line } => {
            let expected = line
                .get(moves.len())
                .and_then(|e| legal(&r.pos, e))
                .map(|e| uci_of(&e));
            let last = moves.len() + 1 >= line.len();
            expected.as_deref() == Some(played.as_str()) || (last && next.is_checkmate())
        }
        Task::Mate { within } => {
            let left = within.saturating_sub(r.played);
            next.is_checkmate() || (left > 1 && defender_lost(&next, left - 1))
        }
        Task::Play { .. } => true,
    };
    Ok(Verdict {
        accepted,
        uci: accepted.then_some(played),
    })
}

/// The move a line or mate task wants next; None for read and play tasks (the
/// engine hints those) or when it is not your move.
pub fn hint<S: AsRef<str>>(fen: &str, task: &Task, moves: &[S]) -> Result<Option<String>, String> {
    let r = replay(fen, moves)?;
    if r.pos.turn() != r.player {
        return Ok(None);
    }
    Ok(match task {
        Task::Line { moves: line } => line.get(moves.len()).cloned(),
        Task::Mate { within } => quickest_mate(&r.pos, within.saturating_sub(r.played)),
        _ => None,
    })
}

/// The course groups, in the order Practice lists them.
pub const GROUPS: [&str; 3] = ["Checkmates", "Pawn endgames", "Rook endgames"];

/// Problems with the course data: unknown squares, illegal or unsolvable steps, duplicate ids.
/// Empty when every step can be completed as written.
pub fn problems(data: &Data) -> Vec<String> {
    let mut out = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let id_ok = |id: &str| {
        !id.is_empty()
            && id.len() <= 40
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    for course in &data.courses {
        if !id_ok(&course.id) || !ids.insert(course.id.clone()) {
            out.push(format!("course id {:?}", course.id));
        }
        if !GROUPS.contains(&course.group.as_str()) {
            out.push(format!("{}: group {:?}", course.id, course.group));
        }
        if course.chapters.is_empty() {
            out.push(format!("{}: no chapters", course.id));
        }
        for chapter in &course.chapters {
            if !id_ok(&chapter.id) || !ids.insert(chapter.id.clone()) {
                out.push(format!("chapter id {:?}", chapter.id));
            }
            if chapter.steps.is_empty() {
                out.push(format!("{}: no steps", chapter.id));
            }
            if chapter.steps.iter().all(|s| matches!(s.task, Task::Read)) {
                out.push(format!("{}: nothing to play", chapter.id));
            }
            for (i, step) in chapter.steps.iter().enumerate() {
                let at = format!("{} step {}", chapter.id, i + 1);
                out.extend(
                    step_problems(step)
                        .into_iter()
                        .map(|p| format!("{at}: {p}")),
                );
            }
        }
    }
    out
}

fn step_problems(step: &Step) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(start) = chess(&step.fen) else {
        return vec![format!("illegal position {}", step.fen)];
    };
    if step.text.trim().is_empty() {
        out.push("no text".into());
    }
    for shape in &step.shapes {
        let squares = std::iter::once(&shape.orig).chain(shape.dest.as_ref());
        if squares
            .into_iter()
            .any(|s| rules::parse_square(s).is_none())
            || !["green", "red", "blue", "yellow"].contains(&shape.brush.as_str())
        {
            out.push(format!("shape {shape:?}"));
        }
    }
    let begun = status(&step.fen, &step.task, &[] as &[String]);
    match &step.task {
        Task::Read => {}
        Task::Line { moves } => {
            if moves.is_empty() || moves.len() % 2 == 0 {
                out.push("a line must end with your move".into());
            }
            // `start` is only borrowed by the `Task::Mate` arm below, so this arm can take it.
            let mut pos = start;
            for uci in moves {
                match legal(&pos, uci) {
                    Some(m) if uci_of(&m) == *uci => pos.play_unchecked(m),
                    _ => {
                        out.push(format!("{uci} is not a legal move in this line"));
                        break;
                    }
                }
            }
        }
        Task::Mate { within } => {
            if !(1..=3).contains(within) {
                out.push("mate tasks are solved within 1 to 3 moves".into());
            } else if !mates_within(&start, *within) {
                out.push(format!("no forced mate in {within}"));
            } else if *within > 1 && mates_within(&start, within - 1) {
                out.push(format!("mates sooner than in {within}"));
            }
        }
        Task::Play {
            goal,
            within,
            squares,
        } => {
            if *within == 0 || *within > 60 {
                out.push("within must be 1 to 60 moves".into());
            }
            let needs_squares = matches!(goal, Goal::Reach | Goal::Confine);
            if needs_squares == squares.is_empty() || squares_of(squares).len() != squares.len() {
                out.push("squares".into());
            }
            if begun.as_ref().is_ok_and(|s| s.over) {
                out.push("already over".into());
            }
        }
    }
    if let Err(e) = begun {
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn every_course_step_can_be_completed() {
        let data = data();
        let problems = problems(data);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn mate_in_two_accepts_only_moves_that_keep_the_mate() {
        // Back rank: 1.Re8+ Rxe8 2.Rxe8#.
        let fen = "3r2k1/5ppp/8/8/8/8/4RPPP/4R1K1 w - - 0 1";
        let task = Task::Mate { within: 2 };
        assert!(mates_within(&chess(fen).unwrap(), 2));
        assert!(!try_move(fen, &task, &m(&[]), "h2h3").unwrap().accepted);
        let first = try_move(fen, &task, &m(&[]), "e2e8").unwrap();
        assert!(first.accepted);
        let after = status(fen, &task, &m(&["e2e8"])).unwrap();
        assert_eq!(after.reply.as_deref(), Some("d8e8"));
        let done = status(fen, &task, &m(&["e2e8", "d8e8", "e1e8"])).unwrap();
        assert_eq!(
            (done.over, done.success, done.reason),
            (true, Some(true), Some(Reason::Checkmate))
        );
        assert_eq!(hint(fen, &task, &m(&[])).unwrap().as_deref(), Some("e2e8"));
    }

    /// A repetition reached stays reached, and material lost stays lost, whatever is played
    /// after.
    #[test]
    fn a_repetition_or_a_loss_of_material_is_kept_past_later_moves() {
        let fen = "7k/8/8/8/8/8/8/R5K1 w - - 0 1";
        // One move more than White plays here: a status that missed the repetition searches the
        // defence to a mate in what is left, and a deeper one takes minutes.
        let task = Task::Mate { within: 6 };
        let shuffle = ["a1a2", "h8g8", "a2a1", "g8h8"].repeat(2);
        let mut moves = m(&shuffle);
        moves.push("a1a2".into());
        let after = status(fen, &task, &moves).unwrap();
        assert_eq!(
            (after.over, after.success, after.reason),
            (true, Some(false), Some(Reason::Repetition))
        );

        let fen = "r6k/8/8/8/8/8/8/R5K1 w - - 0 1";
        let task = Task::Play {
            goal: Goal::Mate,
            within: 10,
            squares: Vec::new(),
        };
        let after = status(fen, &task, &m(&["a1a7", "a8a7", "g1f2"])).unwrap();
        assert_eq!(
            (after.over, after.success, after.reason),
            (true, Some(false), Some(Reason::MaterialLost))
        );
    }

    #[test]
    fn lines_follow_the_script_and_accept_any_final_mate() {
        let fen = "6k1/8/6K1/8/8/8/8/R7 w - - 0 1";
        let task = Task::Line {
            moves: m(&["a1a8"]),
        };
        assert!(try_move(fen, &task, &m(&[]), "a1a8").unwrap().accepted);
        assert!(!try_move(fen, &task, &m(&[]), "a1a7").unwrap().accepted);
        let done = status(fen, &task, &m(&["a1a8"])).unwrap();
        assert_eq!(done.reason, Some(Reason::Checkmate));
        let scripted = Task::Line {
            moves: m(&["g6f6", "g8h7", "a1h1"]),
        };
        let mid = status(fen, &scripted, &m(&["g6f6"])).unwrap();
        assert_eq!(mid.reply.as_deref(), Some("g8h7"));
    }

    #[test]
    fn play_goals() {
        let reach = Task::Play {
            goal: Goal::Reach,
            within: 3,
            squares: m(&["c5", "d5", "e5"]),
        };
        let fen = "4k3/8/8/8/8/3K4/3P4/8 w - - 0 1";
        let s = status(fen, &reach, &m(&["d3d4", "e8e7"])).unwrap();
        assert!(!s.over);
        let s = status(fen, &reach, &m(&["d3d4", "e8e7", "d4d5"])).unwrap();
        assert_eq!(s.reason, Some(Reason::Reached));
        assert!(!status(fen, &reach, &m(&[])).unwrap().engine);
        assert!(status(fen, &reach, &m(&["d3c3"])).unwrap().engine);

        let win = Task::Play {
            goal: Goal::Win,
            within: 10,
            squares: vec![],
        };
        let promote = "8/4P3/8/8/8/8/k7/4K3 w - - 0 1";
        let s = status(promote, &win, &m(&["e7e8q"])).unwrap();
        assert_eq!(s.reason, Some(Reason::Promoted));
        let hanging = "8/3kP3/8/8/8/8/8/K7 w - - 0 1";
        assert!(!status(hanging, &win, &m(&["e7e8q"])).unwrap().over);

        let draw = Task::Play {
            goal: Goal::Draw,
            within: 1,
            squares: vec![],
        };
        let s = status("8/8/8/8/8/2k5/2p5/2K5 w - - 0 1", &draw, &m(&["c1d2"])).unwrap();
        assert!(s.over);

        let confine = Task::Play {
            goal: Goal::Confine,
            within: 2,
            squares: m(&["g8", "h8", "g7", "h7"]),
        };
        let s = status("7k/8/5K2/8/8/8/8/R7 w - - 0 1", &confine, &m(&["a1a6"])).unwrap();
        assert_eq!(s.reason, Some(Reason::Confined));
    }

    #[test]
    fn a_promotion_taken_straight_back_is_not_a_loss() {
        // Stockfish's line from the centre-pawn lesson: the pawn queens and the queen takes it.
        let fen = "8/7Q/8/8/8/8/4pk2/K7 w - - 0 1";
        let win = Task::Play {
            goal: Goal::Win,
            within: 25,
            squares: vec![],
        };
        let line = m(&[
            "h7h4", "f2f1", "h4f4", "f1g1", "f4e3", "g1h2", "a1b1", "e2e1q",
        ]);
        assert!(!status(fen, &win, &line).unwrap().over);
        let mut taken = line.clone();
        taken.push("e3e1".into());
        let s = status(fen, &win, &taken).unwrap();
        assert_eq!((s.success, s.reason), (Some(true), Some(Reason::BareKing)));
    }
    fn mate_in_one_position() -> &'static str {
        "7k/6pp/p7/8/8/8/8/1R2K3 w - - 0 1"
    }

    #[test]
    fn pieces_are_worth_their_usual_points() {
        let points: Vec<i32> = Role::ALL.iter().map(|&r| value(r)).collect();
        assert_eq!(points.iter().sum::<i32>(), 1 + 3 + 3 + 5 + 9);
        assert_eq!(value(Role::Pawn), 1);
        assert_eq!(value(Role::Queen), 9);
        assert_eq!(value(Role::King), 0);
        let start = chess("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert_eq!(material(&start, Color::White), 39);
        assert_eq!(material(&start, Color::Black), 39);
    }

    #[test]
    fn the_mate_distance_is_the_fewest_moves() {
        let back_rank = chess("3r2k1/5ppp/8/8/8/8/4RPPP/4R1K1 w - - 0 1").unwrap();
        assert_eq!(mate_distance(&back_rank, 3), Some(2));
        assert_eq!(mate_distance(&back_rank, 1), None);
        let one = chess(mate_in_one_position()).unwrap();
        assert_eq!(mate_distance(&one, 3), Some(1));
        let start = chess("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        assert_eq!(mate_distance(&start, 2), None);
    }

    /// A reply that escapes mate beats one that is mated at the last move looked at, and on a
    /// tie the first in UCI order wins.
    #[test]
    fn the_best_defence_escapes_the_mate() {
        // Only the pawn move a6a5 leaves 1.Rb8#; the king and the other pawns all get away.
        let pos = chess("7k/6pp/p7/8/8/8/8/1R2K3 b - - 0 1").unwrap();
        assert_eq!(best_defence(&pos, 1).as_deref(), Some("g7g5"));
        // Against a mate in two the king walk that lasts longest is chosen.
        let back_rank = chess("3r2k1/5ppp/8/8/8/8/4RPPP/4R1K1 w - - 0 1").unwrap();
        let (_, after) = successors(&back_rank)
            .into_iter()
            .find(|(uci, _)| uci == "e2e8")
            .unwrap();
        assert_eq!(best_defence(&after, 1).as_deref(), Some("d8e8"));
        assert_eq!(quickest_mate(&back_rank, 2).as_deref(), Some("e2e8"));
    }

    #[test]
    fn a_hanging_piece_is_a_swing_but_a_capture_is_not() {
        // White leaves the knight to the bishop and moves on: 3 points down.
        let draw = Task::Play {
            goal: Goal::Draw,
            within: 10,
            squares: vec![],
        };
        let fen = "4k3/8/8/8/1b6/2N5/P7/4K3 w - - 0 1";
        let s = status(fen, &draw, &m(&["e1d1", "b4c3", "d1c1"])).unwrap();
        assert_eq!(
            (s.over, s.success, s.reason),
            (true, Some(false), Some(Reason::Lost))
        );
        // Taking a knight is a gain, not a loss.
        let s = status("6k1/3n4/8/8/8/8/8/3QK3 w - - 0 1", &draw, &m(&["d1d7"])).unwrap();
        assert!(!s.over);
    }

    #[test]
    fn a_bare_king_against_a_rook_is_lost_even_for_a_pawn() {
        // The pawn falls (1 point) and then the king stands alone against the rook.
        let draw = Task::Play {
            goal: Goal::Draw,
            within: 10,
            squares: vec![],
        };
        let fen = "r6k/8/8/8/8/8/P7/6K1 w - - 0 1";
        let s = status(fen, &draw, &m(&["g1h1", "a8a2", "h1g1"])).unwrap();
        assert_eq!(
            (s.over, s.success, s.reason),
            (true, Some(false), Some(Reason::Lost))
        );
    }

    #[test]
    fn material_is_judged_after_your_moves_not_theirs() {
        let task = Task::Play {
            goal: Goal::Mate,
            within: 10,
            squares: vec![],
        };
        let fen = "r6k/8/8/8/8/8/8/R5K1 w - - 0 1";
        let s = status(fen, &task, &m(&["a1a7", "a8a7"])).unwrap();
        assert!(!s.over);
    }

    #[test]
    fn mating_and_drawing_are_judged_against_the_goal() {
        let fen = "6k1/8/6K1/8/8/8/8/R7 w - - 0 1";
        let play = |goal| Task::Play {
            goal,
            within: 5,
            squares: vec![],
        };
        let s = status(fen, &play(Goal::Mate), &m(&["a1a8"])).unwrap();
        assert_eq!((s.success, s.reason), (Some(true), Some(Reason::Checkmate)));
        let s = status(fen, &play(Goal::Draw), &m(&["a1a8"])).unwrap();
        assert_eq!(
            (s.success, s.reason),
            (Some(false), Some(Reason::Checkmate))
        );

        let stalemate = "8/8/8/8/8/2k5/2p5/2K5 w - - 0 1";
        let s = status(stalemate, &play(Goal::Draw), &m(&[])).unwrap();
        assert_eq!((s.success, s.reason), (Some(true), Some(Reason::Stalemate)));
        let s = status(stalemate, &play(Goal::Win), &m(&[])).unwrap();
        assert_eq!(
            (s.success, s.reason),
            (Some(false), Some(Reason::Stalemate))
        );
    }

    #[test]
    fn a_bare_king_wins_only_while_the_rook_is_safe() {
        let win = Task::Play {
            goal: Goal::Win,
            within: 10,
            squares: vec![],
        };
        // The pawn on e7 hangs to the king, but the rook does not: still a bare king against it.
        let fen = "4k3/4P3/8/8/8/8/8/R3K3 w - - 0 1";
        let s = status(fen, &win, &m(&["a1a2"])).unwrap();
        assert_eq!((s.success, s.reason), (Some(true), Some(Reason::BareKing)));
        // A rook the king can take is not safe.
        let s = status("8/8/8/8/8/1k6/8/R6K w - - 0 1", &win, &m(&["a1a2"])).unwrap();
        assert!(!s.over);
    }

    #[test]
    fn a_king_is_confined_only_inside_the_region() {
        let region: Vec<Square> = squares_of(&m(&["g8", "h8", "h7", "g7"]));
        // No king move at all, and the king stands outside the region.
        let stalemate = chess("7k/5Q2/7K/8/8/8/8/8 b - - 0 1").unwrap();
        assert!(!confined(&stalemate, &squares_of(&m(&["a1"]))));
        assert!(confined(&stalemate, &region));
        // Only king moves count: the pawn's moves leave the region.
        let free = chess("7k/p7/5K2/8/8/8/8/R7 b - - 0 1").unwrap();
        assert!(confined(&free, &region));
        assert!(!confined(&free, &squares_of(&m(&["g8", "h8"]))));
    }

    #[test]
    fn only_your_own_turn_can_be_answered() {
        let play = Task::Play {
            goal: Goal::Win,
            within: 10,
            squares: vec![],
        };
        let fen = "4k3/8/8/8/8/3K4/3P4/8 w - - 0 1";
        let theirs = try_move(fen, &play, &m(&["d3d4"]), "e8e7").unwrap();
        assert!(!theirs.accepted && theirs.uci.is_none());
        assert!(try_move(fen, &play, &m(&[]), "d3d4").unwrap().accepted);
    }

    #[test]
    fn any_mate_ends_a_line() {
        // Both rooks mate: only a8 is scripted.
        let fen = "6k1/8/6K1/8/8/8/R7/1R6 w - - 0 1";
        let line = Task::Line {
            moves: m(&["a2a8"]),
        };
        assert!(try_move(fen, &line, &m(&[]), "b1b8").unwrap().accepted);
        assert!(!try_move(fen, &line, &m(&[]), "b1b7").unwrap().accepted);
    }

    #[test]
    fn a_mate_task_rejects_a_move_that_only_mates_later() {
        let fen = "4k3/8/4K3/8/8/8/8/R7 w - - 0 1";
        let task = Task::Mate { within: 2 };
        let start = chess(fen).unwrap();
        assert!(mates_within(&start, 2));
        let slow: Vec<String> = successors(&start)
            .into_iter()
            .filter(|(_, next)| !next.is_checkmate() && !defender_lost(next, 1))
            .map(|(uci, _)| uci)
            .collect();
        assert!(!slow.is_empty());
        for uci in slow {
            let verdict = try_move(fen, &task, &m(&[]), &uci).unwrap();
            assert!(!verdict.accepted, "{uci}");
        }
    }

    fn step(fen: &str, task: Task) -> Step {
        Step {
            fen: fen.to_string(),
            text: "Text".into(),
            shapes: Vec::new(),
            task,
        }
    }

    fn course(id: &str, chapter: &str) -> Course {
        Course {
            id: id.to_string(),
            group: GROUPS[0].to_string(),
            title: "Title".into(),
            subtitle: "Subtitle".into(),
            chapters: vec![Chapter {
                id: chapter.to_string(),
                title: "Chapter".into(),
                steps: vec![step(
                    "6k1/8/6K1/8/8/8/8/R7 w - - 0 1",
                    Task::Mate { within: 1 },
                )],
            }],
        }
    }

    fn id_problems(courses: Vec<Course>) -> Vec<String> {
        problems(&Data { courses })
    }

    #[test]
    fn course_and_chapter_ids_are_short_lowercase_and_unique() {
        assert!(id_problems(vec![course("rook-2", "mate-1")]).is_empty());
        let forty = "a".repeat(40);
        assert!(id_problems(vec![course(&forty, "mate-1")]).is_empty());
        for bad in ["", "Upper", "with space", "under_score", &"a".repeat(41)] {
            let found = id_problems(vec![course(bad, "mate-1")]);
            assert!(
                found.iter().any(|p| p.starts_with("course id")),
                "{bad:?}: {found:?}"
            );
            let found = id_problems(vec![course("rook", bad)]);
            assert!(
                found.iter().any(|p| p.starts_with("chapter id")),
                "{bad:?}: {found:?}"
            );
        }
        let twice = id_problems(vec![course("rook", "one"), course("rook", "two")]);
        assert_eq!(twice, ["course id \"rook\""]);
        let twice = id_problems(vec![course("rook", "one"), course("pawn", "one")]);
        assert_eq!(twice, ["chapter id \"one\""]);
    }

    fn found(step: &Step) -> Vec<String> {
        step_problems(step)
    }

    #[test]
    fn shapes_need_squares_and_a_known_brush() {
        let mut s = step("6k1/8/6K1/8/8/8/8/R7 w - - 0 1", Task::Read);
        let shape = |orig: &str, dest: Option<&str>, brush: &str| Shape {
            orig: orig.into(),
            dest: dest.map(String::from),
            brush: brush.into(),
        };
        s.shapes = vec![shape("a1", Some("a8"), "green")];
        assert!(found(&s).is_empty());
        s.shapes = vec![shape("a1", None, "purple")];
        assert_eq!(found(&s).len(), 1);
        s.shapes = vec![shape("z9", None, "red")];
        assert_eq!(found(&s).len(), 1);
        s.shapes = vec![shape("a1", Some("a9"), "red")];
        assert_eq!(found(&s).len(), 1);
    }

    #[test]
    fn a_line_has_an_odd_number_of_legal_moves() {
        let fen = "6k1/8/6K1/8/8/8/8/R7 w - - 0 1";
        let line = |moves: &[&str]| step(fen, Task::Line { moves: m(moves) });
        assert!(found(&line(&["a1a8"])).is_empty());
        let ends = |s: &Step| {
            found(s)
                .iter()
                .any(|p| p.contains("must end with your move"))
        };
        assert!(!ends(&line(&["a1a8"])));
        assert!(ends(&line(&["a1a7", "g8f8"])));
        assert!(ends(&line(&[])));
        let illegal = found(&line(&["a1a9"]));
        assert_eq!(illegal, ["a1a9 is not a legal move in this line"]);
        // Castling is written as the king's move, not as the king taking its rook.
        let castle = step(
            "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
            Task::Line {
                moves: m(&["e1g1"]),
            },
        );
        assert_eq!(found(&castle).len(), 1);
    }

    #[test]
    fn a_mate_task_must_be_solvable_in_exactly_its_moves() {
        let fen = "6k1/8/6K1/8/8/8/8/R7 w - - 0 1";
        let mate = |within| step(fen, Task::Mate { within });
        assert!(found(&mate(1)).is_empty());
        assert_eq!(found(&mate(2)), ["mates sooner than in 2"]);
        assert_eq!(found(&mate(3)), ["mates sooner than in 3"]);
        assert_eq!(
            found(&mate(4)),
            ["mate tasks are solved within 1 to 3 moves"]
        );
        assert_eq!(
            found(&mate(0)),
            ["mate tasks are solved within 1 to 3 moves"]
        );
        let none = step("4k3/8/8/8/8/8/8/K6R w - - 0 1", Task::Mate { within: 1 });
        assert_eq!(found(&none), ["no forced mate in 1"]);
    }

    #[test]
    fn a_play_task_needs_a_fair_number_of_moves_and_its_squares() {
        let fen = "4k3/8/8/8/8/3K4/3P4/8 w - - 0 1";
        let play = |goal, within, squares: &[&str]| {
            step(
                fen,
                Task::Play {
                    goal,
                    within,
                    squares: m(squares),
                },
            )
        };
        let range = |s: &Step| found(s).iter().any(|p| p.contains("within must be"));
        assert!(found(&play(Goal::Win, 1, &[])).is_empty());
        assert!(found(&play(Goal::Win, 60, &[])).is_empty());
        assert!(range(&play(Goal::Win, 0, &[])));
        assert!(range(&play(Goal::Win, 61, &[])));
        assert!(found(&play(Goal::Reach, 3, &["c5"])).is_empty());
        assert_eq!(found(&play(Goal::Reach, 3, &[])), ["squares"]);
        assert_eq!(found(&play(Goal::Reach, 3, &["zz"])), ["squares"]);
        assert_eq!(found(&play(Goal::Win, 3, &["c5"])), ["squares"]);
        let over = step(
            "6k1/8/6K1/8/8/8/8/R7 b - - 0 1",
            Task::Play {
                goal: Goal::Win,
                within: 3,
                squares: vec![],
            },
        );
        assert!(found(&over).is_empty());
        let over = step(
            "7k/5Q2/7K/8/8/8/8/8 b - - 0 1",
            Task::Play {
                goal: Goal::Win,
                within: 3,
                squares: vec![],
            },
        );
        assert_eq!(found(&over), ["already over"]);
    }
}
