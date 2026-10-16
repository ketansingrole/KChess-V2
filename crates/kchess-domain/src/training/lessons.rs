//! Interactive lessons (`crates/kchess-wasm/js/lessons.ts`): the checkmate and endgame courses
//! (`data/lessons.json`), and how a lesson step stands after a line of moves.
//!
//! A step is read, followed (`line`: your moves alternate with scripted replies), solved
//! (`mate`: force mate within a few moves; replies come from the solver below, so these work
//! without an engine) or played out against the engine (`play`, with a goal).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::{Chess, Color, Move, Position, Role, Square, variant::VariantPosition};
use std::collections::HashMap;
use std::sync::OnceLock;

use super::{Out, to_value, value_arg};
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

/// The courses; an error only if the embedded file is malformed.
pub fn data() -> Result<&'static Data, String> {
    static DATA: OnceLock<Result<Data, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(LESSONS_JSON).map_err(|e| format!("lesson data is not valid: {e}"))
    })
    .as_ref()
    .map_err(Clone::clone)
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

fn color_name(color: Color) -> &'static str {
    if color == Color::White {
        "white"
    } else {
        "black"
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

fn repetition_key(pos: &Chess) -> String {
    rules::make_fen(&VariantPosition::Chess(pos.clone()))
        .split(' ')
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
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
    let mut seen: HashMap<String, u32> = HashMap::from([(repetition_key(&pos), 1)]);
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
        let key = repetition_key(&pos);
        let count = seen.get(&key).copied().unwrap_or(0) + 1;
        seen.insert(key, count);
        out.repeated |= count >= 3;
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

/// `lessonStatus`: where `task`, from `fen`, stands after `moves` (UCI).
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
        player: color_name(r.player),
        turn: color_name(turn),
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

/// `lessonMove`: whether `uci`, your move after `moves`, is one the step accepts. A line wants its
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

/// `lessonHint`: the move a line or mate task wants next; None for read and play tasks (the
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
            let mut pos = start.clone();
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

/* ── Calls ───────────────────────────────────────────────────────────────────────────────── */

fn step_args(args: &[Value]) -> Result<(String, Task, Vec<String>), String> {
    Ok((
        value_arg(args, 0, "fen")?,
        value_arg(args, 1, "task")?,
        value_arg(args, 2, "moves")?,
    ))
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "lessonCourses" => data().and_then(|d| to_value(&d.courses)),
        "lessonProblems" => data().and_then(|d| to_value(problems(d))),
        "lessonStatus" => step_args(args)
            .and_then(|(fen, task, moves)| status(&fen, &task, &moves))
            .and_then(to_value),
        "lessonMove" => step_args(args)
            .and_then(|(fen, task, moves)| {
                let uci: String = value_arg(args, 3, "uci")?;
                try_move(&fen, &task, &moves, &uci)
            })
            .and_then(to_value),
        "lessonHint" => step_args(args)
            .and_then(|(fen, task, moves)| hint(&fen, &task, &moves))
            .and_then(to_value),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn every_course_step_can_be_completed() {
        let data = data().expect("lesson data parses");
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

    #[test]
    fn invalid_input_is_an_error() {
        assert!(status("nonsense", &Task::Read, &[] as &[String]).is_err());
        assert!(call("lessonStatus", &[]).unwrap().is_err());
        assert!(call("lessonCourses", &[]).unwrap().is_ok());
    }
}
