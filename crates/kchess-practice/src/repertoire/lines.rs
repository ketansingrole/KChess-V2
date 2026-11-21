//! The repertoire as lines: one tree from a study's chapters, its lines and their ids, which
//! line is weakest, and when each is due.

use super::*;

/// Whether `a` and `b` are the same move from `fen`: castling written either way is one move,
/// and a rook's move from the king's square is never castling.
pub(super) fn same_move_at(fen: &str, a: &str, b: &str) -> bool {
    kchess_domain::rules::standard_from_fen(fen)
        .is_some_and(|position| kchess_domain::position::same_move(&position, a, b))
}

/// Adds another chapter's moves (and comments) from the same position to a tree.
pub(super) fn merge(into: &mut TreeNode, mut from: TreeNode) {
    for comment in from.comments.take().unwrap_or_default() {
        let comments = into.comments.get_or_insert_with(Vec::new);
        if !comments.contains(&comment) {
            comments.push(comment);
        }
    }
    for child in std::mem::take(&mut from.children) {
        match into
            .children
            .iter_mut()
            .find(|entry| entry.uci == child.uci)
        {
            Some(same) => merge(same, child),
            None => into.children.push(child),
        }
    }
}

/// How often the moves at and below a node were missed: the trainer steers towards weak lines.
pub(super) fn weakness(start: &TreeNode, table: Option<&Keyed<serde_json::Number>>) -> f64 {
    let mut total = 0.;
    let mut stack = vec![start];
    while let Some(next) = stack.pop() {
        total += table
            .and_then(|table| table.get(&next.fen))
            .and_then(serde_json::Number::as_f64)
            .unwrap_or(0.);
        stack.extend(next.children.iter());
    }
    total
}

/// One of the nodes, more often those with weak lines below them; `draw` is in [0, 1).
pub(super) fn pick_weak(
    nodes: &[&TreeNode],
    table: Option<&Keyed<serde_json::Number>>,
    draw: f64,
) -> Option<usize> {
    let weights: Vec<f64> = nodes
        .iter()
        .map(|node| 1. + 2. * weakness(node, table))
        .collect();
    let mut pick = draw * weights.iter().sum::<f64>();
    for (index, weight) in weights.iter().enumerate() {
        pick -= weight;
        if pick <= 0. {
            return Some(index);
        }
    }
    (!nodes.is_empty()).then_some(0)
}

/// The trees drilled from a study: one per starting position among the chosen chapters.
pub(super) fn study_roots(chapters: &[Chapter]) -> Vec<TreeNode> {
    let mut roots: Vec<TreeNode> = Vec::new();
    for chapter in chapters {
        let Some(tree) = TreeNode::from_pgn(&chapter.pgn) else {
            continue;
        };
        match roots.iter_mut().find(|root| root.fen == tree.fen) {
            Some(same) => merge(same, tree),
            None => roots.push(tree),
        }
    }
    roots
}

/// The paths of a tree's leaves: each one is a line, from the root to its end.
pub(super) fn leaves(root: &TreeNode) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![(root, String::new())];
    while let Some((node, path)) = stack.pop() {
        if node.children.is_empty() {
            if !path.is_empty() {
                found.push(path);
            }
            continue;
        }
        for child in node.children.iter().rev() {
            stack.push((child, child_path(&path, &child.uci)));
        }
    }
    found
}

/// A line's name in the schedule.
pub(super) fn line_id(root: &TreeNode, path: &str) -> String {
    repetition::line_id(&root.fen, root.fen == STANDARD_FEN, path)
}

/// Whether `path` is `line` or on the way to it.
pub(super) fn leads_to(path: &str, line: &str) -> bool {
    line == path
        || line
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with(' '))
}

/// When a line is next due, from now: "in 5 min", "in 3 h", "in 2 days".
pub(super) fn due_in(due: f64, now: f64) -> String {
    let minutes = ((due - now) / 60_000.).ceil().max(1.);
    if minutes < 60. {
        format!("in {minutes} min")
    } else if minutes < 24. * 60. {
        format!("in {} h", (minutes / 60.).round())
    } else {
        let days = (minutes / (24. * 60.)).round();
        format!("in {days} day{}", if days == 1. { "" } else { "s" })
    }
}

/// The panel's line on the schedule: how many lines are due, or when the next one is.
pub(super) fn schedule_text(summary: &repetition::Summary, now: f64) -> Option<String> {
    match (summary.due, summary.next_due) {
        (0, None) => None,
        (0, Some(due)) => Some(format!("Next line due {}", due_in(due, now))),
        (1, _) => Some("1 line due".into()),
        (due, _) => Some(format!("{due} lines due")),
    }
}

/// Whether `color` has a move to play at `node`: a line's last move leaves nothing to play,
/// whoever is on move, and the next line follows.
pub(super) fn to_play(node: &TreeNode, color: Color) -> bool {
    !node.children.is_empty() && kchess_domain::rules::fen_turn(&node.fen) == color
}
