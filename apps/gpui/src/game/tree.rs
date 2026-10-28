//! The analysis board's move tree: every position keeps
//! the moves tried from it, the first child being the main line. A node is addressed by its path,
//! the UCI moves from the root joined with spaces. The rules decide what a path means, whether a
//! move is new, where a node goes and how it is written; this walks the tree and applies them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::rules;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// Empty at the root.
    pub uci: String,
    pub san: String,
    pub fen: String,
    /// Half-moves since the game's start: 0 is White's first move still to play.
    pub ply: f64,
    pub children: Vec<TreeNode>,
    /// PGN headers, on the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_comments: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nags: Option<Vec<i64>>,
}

fn text(method: &str, args: &[Value]) -> String {
    rules(method, args)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub fn path_of(moves: &[String]) -> String {
    text("pathOf", &[json!(moves)])
}

pub fn moves_of(path: &str) -> Vec<String> {
    rules("movesOf", &[json!(path)])
        .ok()
        .and_then(|moves| serde_json::from_value(moves).ok())
        .unwrap_or_default()
}

/// A path one move longer: its parent's path and the move.
pub fn child_path(parent: &str, uci: &str) -> String {
    if parent.is_empty() {
        uci.to_string()
    } else {
        format!("{parent} {uci}")
    }
}

/// `analysisContext`: the position with the moves that led to it, for repetition.
pub fn analysis_context(root_fen: &str, moves: &[String]) -> String {
    text("analysisContext", &[json!(root_fen), json!(moves)])
}

/// "+0.34", "−1.20", "#3", as Lichess prints them; a dash for nothing.
pub fn format_eval(line: Option<&Value>) -> String {
    text("formatEval", &[line.cloned().unwrap_or(Value::Null)])
}

/// White's winning chances in −1…1, for the evaluation bar.
pub fn winning_chances(line: Option<&Value>) -> f32 {
    rules("winningChances", &[line.cloned().unwrap_or(Value::Null)])
        .ok()
        .and_then(|chances| chances.as_f64())
        .unwrap_or_default() as f32
}

/// One move of a principal variation: its SAN and its label ("12… Nf6").
#[derive(Clone, Debug, Deserialize)]
pub struct PvMove {
    pub san: String,
    pub label: String,
}

/// A principal variation in SAN with move numbers, stopping at the first illegal move.
pub fn pv_san(fen: &str, pv: &[String], max: usize) -> Vec<PvMove> {
    let pv = &pv[..pv.len().min(max)];
    rules("pvSan", &[json!(fen), json!(pv), json!(pv.len())])
        .ok()
        .and_then(|moves| serde_json::from_value(moves).ok())
        .unwrap_or_default()
}

/// "1.", "1…", or nothing: the number to print before a move at `ply`.
pub fn move_number(ply: f64, always: bool) -> String {
    text("moveNumber", &[json!(ply), json!(always)])
}

/// The annotation a move is marked with: its glyph ("!?") and name.
pub fn move_glyph(node: &TreeNode) -> Option<(String, String)> {
    let nags = node.nags.as_ref()?;
    let glyph = rules("moveGlyph", &[json!(nags)]).ok()?;
    Some((
        glyph["glyph"].as_str()?.to_string(),
        glyph["label"].as_str().unwrap_or_default().to_string(),
    ))
}

/// A comment as a reader sees it (a game's clock times are not notes).
pub fn comment_text(comment: &str) -> String {
    text("commentText", &[json!(comment)])
}

impl TreeNode {
    /// A tree holding only its start; an unusable FEN starts from the usual position.
    pub fn new(fen: Option<&str>) -> TreeNode {
        let start =
            rules("startNode", &[json!(fen.unwrap_or(super::STANDARD_FEN))]).unwrap_or(Value::Null);
        TreeNode {
            fen: start["fen"]
                .as_str()
                .unwrap_or(super::STANDARD_FEN)
                .to_string(),
            ply: start["ply"].as_f64().unwrap_or_default(),
            ..TreeNode::default()
        }
    }

    /// One bounded PGN document, or `None` when it cannot be imported without loss.
    pub fn from_pgn(text: &str) -> Option<TreeNode> {
        let tree = rules("treeFromPgn", &[json!(text)]).ok()?;
        serde_json::from_value(tree).ok()
    }

    /// The tree as PGN, with its own headers.
    pub fn to_pgn(&self) -> String {
        fn strip(node: &TreeNode) -> Value {
            let mut out = Map::new();
            out.insert("san".into(), json!(node.san));
            out.insert(
                "children".into(),
                Value::Array(node.children.iter().map(strip).collect()),
            );
            if let Some(comments) = &node.comments {
                out.insert("comments".into(), json!(comments));
            }
            if let Some(comments) = &node.starting_comments {
                out.insert("startingComments".into(), json!(comments));
            }
            if let Some(nags) = &node.nags {
                out.insert("nags".into(), json!(nags));
            }
            Value::Object(out)
        }
        let mut document = Map::new();
        document.insert("fen".into(), json!(self.fen));
        document.insert(
            "children".into(),
            Value::Array(self.children.iter().map(strip).collect()),
        );
        if let Some(comments) = &self.comments {
            document.insert("comments".into(), json!(comments));
        }
        let headers = Value::Object(self.headers.clone().unwrap_or_default());
        text("treeToPgn", &[Value::Object(document), headers])
    }

    /// The nodes from the root to `path`, as far as the path exists.
    pub fn nodes_along(&self, path: &str) -> Vec<&TreeNode> {
        let mut nodes = vec![self];
        for uci in moves_of(path) {
            let Some(parent) = nodes.last() else { break };
            match parent.children.iter().find(|child| child.uci == uci) {
                Some(next) => nodes.push(next),
                None => break,
            }
        }
        nodes
    }

    pub fn node_at(&self, path: &str) -> &TreeNode {
        self.nodes_along(path).last().copied().unwrap_or(self)
    }

    pub fn node_at_mut(&mut self, path: &str) -> &mut TreeNode {
        let mut node = self;
        for uci in moves_of(path) {
            let Some(index) = node.children.iter().position(|child| child.uci == uci) else {
                break;
            };
            node = &mut node.children[index];
        }
        node
    }

    /// The path as far as it exists in the tree.
    pub fn existing_path(&self, path: &str) -> String {
        let moves: Vec<String> = self
            .nodes_along(path)
            .iter()
            .skip(1)
            .map(|node| node.uci.clone())
            .collect();
        path_of(&moves)
    }

    /// Play `uci` after `path`, re-using the node when that move was already tried; the new
    /// path, or `None` when the move is not legal there.
    pub fn add_move(&mut self, path: &str, uci: &str) -> Option<String> {
        let parent = self.node_at(path);
        let plan = rules(
            "addMovePlan",
            &[json!({
                "path": path,
                "uci": uci,
                "fen": parent.fen,
                "ply": parent.ply,
                "childUcis": parent.children.iter().map(|child| child.uci.clone()).collect::<Vec<_>>(),
            })],
        )
        .ok()?;
        if plan.is_null() {
            return None;
        }
        if let Some(child) = plan.get("child").filter(|child| !child.is_null()) {
            let child = TreeNode {
                uci: child["uci"].as_str()?.to_string(),
                san: child["san"].as_str()?.to_string(),
                fen: child["fen"].as_str()?.to_string(),
                ply: child["ply"].as_f64()?,
                ..TreeNode::default()
            };
            self.node_at_mut(path).children.push(child);
        }
        plan["childPath"].as_str().map(str::to_string)
    }

    /// Follow the first child from `path` to the end of that line.
    pub fn line_end(&self, path: &str) -> String {
        let mut chain = Vec::new();
        let mut node = self.node_at(path);
        while let Some(child) = node.children.first() {
            chain.push(child.uci.clone());
            node = child;
        }
        text("lineEnd", &[json!(path), json!(chain)])
    }

    /// The index of each node along `path` among its parent's children.
    fn indices_along(&self, path: &str) -> Vec<usize> {
        let nodes = self.nodes_along(path);
        nodes
            .windows(2)
            .map(|pair| {
                pair[0]
                    .children
                    .iter()
                    .position(|child| child.uci == pair[1].uci)
                    .unwrap_or_default()
            })
            .collect()
    }

    /// True when every move on `path` is a first child.
    pub fn on_mainline(&self, path: &str) -> bool {
        rules("onMainline", &[json!(self.indices_along(path))])
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(true)
    }

    /// Remove the move at `path` and everything after it; the parent's path.
    pub fn delete_at(&mut self, path: &str) -> String {
        let Ok(plan) = rules("deleteAtPlan", &[json!(path)]) else {
            return path.to_string();
        };
        let parent = plan["parentPath"].as_str().unwrap_or_default().to_string();
        if let Some(uci) = plan["uci"].as_str() {
            self.node_at_mut(&parent)
                .children
                .retain(|child| child.uci != uci);
        }
        parent
    }

    /// Make the line through `path` the main line at every branch on the way.
    pub fn promote(&mut self, path: &str) {
        let moves = moves_of(path);
        let Some(plan) = rules("promotePlan", &[json!(self.indices_along(path))])
            .ok()
            .and_then(|plan| serde_json::from_value::<Vec<(usize, usize)>>(plan).ok())
        else {
            return;
        };
        for (depth, from) in plan {
            let holder = self.node_at_mut(&path_of(&moves[..depth.min(moves.len())]));
            if from < holder.children.len() {
                let child = holder.children.remove(from);
                holder.children.insert(0, child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pgn_keeps_its_annotations() {
        let tree = TreeNode::from_pgn("1. e4!? e5 2. Nf3 $1 *").unwrap();
        let e4 = &tree.children[0];
        assert_eq!(e4.nags, Some(vec![5]));
        assert_eq!(e4.children[0].children[0].nags, Some(vec![1]));
    }

    #[test]
    fn moves_branch_and_promote_like_the_electron_tree() {
        let mut tree = TreeNode::new(None);
        let e4 = tree.add_move("", "e2e4").unwrap();
        let e5 = tree.add_move(&e4, "e7e5").unwrap();
        assert_eq!(e5, "e2e4 e7e5");
        // The same move again re-uses its node; another one is a variation.
        assert_eq!(tree.add_move(&e4, "e7e5").unwrap(), e5);
        let c5 = tree.add_move(&e4, "c7c5").unwrap();
        assert_eq!(tree.node_at(&e4).children.len(), 2);
        assert!(!tree.on_mainline(&c5));
        tree.promote(&c5);
        assert!(tree.on_mainline(&c5));
        assert_eq!(tree.line_end(""), "e2e4 c7c5");
        assert_eq!(tree.delete_at(&c5), e4);
        assert_eq!(tree.node_at(&e4).children.len(), 1);
        assert!(tree.add_move("", "e2e5").is_none());
    }

    #[test]
    fn pgn_round_trips_variations_and_comments() {
        let pgn = "[Event \"Test\"]\n\n1. e4 { best by test } e5 (1... c5 2. Nf3) 2. Nf3 *";
        let tree = TreeNode::from_pgn(pgn).unwrap();
        let again = TreeNode::from_pgn(&tree.to_pgn()).unwrap();
        assert_eq!(again, tree);
        assert_eq!(
            tree.node_at("e2e4").comments.as_deref(),
            Some(&["best by test".to_string()][..])
        );
        assert_eq!(tree.node_at("e2e4").children[1].san, "c5");
    }
}
