//! `core/src/domain/studies.ts`: what a study's card shows (`summarizeStudy`). The TypeScript
//! side remembers results per PGN; the summary itself is worked out here.

use serde_json::{Value, json};

use super::{Out, arg};
use crate::tree::{TreeNode, tree_from_pgn};

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "summarizeStudy" => arg(args, 0)
            .and_then(Value::as_str)
            .map(|pgn| Ok(summarize_study(pgn)))
            .unwrap_or_else(|| Err("pgn must be a string".into())),
        _ => return None,
    })
}

/// `summarizeStudy`: the card of a study's PGN, or null when the PGN cannot be imported.
pub fn summarize_study(pgn: &str) -> Value {
    match tree_from_pgn(pgn) {
        Some(root) => summarize(&root),
        None => Value::Null,
    }
}

/// A header's value as the card shows it: '' when empty, '?' or missing.
fn known<'a>(root: &'a TreeNode, name: &str) -> &'a str {
    root.headers
        .as_ref()
        .and_then(|headers| headers.get(name))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value != "?")
        .unwrap_or("")
}

fn has_comments(node: &TreeNode) -> bool {
    node.comments.as_ref().is_some_and(|c| !c.is_empty())
}

/// Half-moves, comments and variations of the whole tree; the final position of the main line.
fn summarize(root: &TreeNode) -> Value {
    let mut comments = usize::from(has_comments(root));
    let mut variations = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        variations += node.children.len().saturating_sub(1);
        for child in &node.children {
            if has_comments(child) {
                comments += 1;
            }
            stack.push(child);
        }
    }
    let mut end = root;
    while let Some(next) = end.children.first() {
        end = next;
    }
    let white = known(root, "White");
    let black = known(root, "Black");
    let players = if white.is_empty() && black.is_empty() {
        String::new()
    } else {
        format!(
            "{} \u{2013} {}",
            if white.is_empty() { "?" } else { white },
            if black.is_empty() { "?" } else { black },
        )
    };
    json!({
        "fen": end.fen,
        "plies": end.ply.saturating_sub(root.ply),
        "comments": comments,
        "variations": variations,
        "players": players,
        "event": known(root, "Event"),
    })
}
