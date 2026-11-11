//! The analysis board's move tree: every position keeps
//! the moves tried from it, the first child being the main line. A node is addressed by its path,
//! the UCI moves from the root joined with spaces. The rules decide what a path means, whether a
//! move is new, where a node goes and how it is written; this walks the tree and applies them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use kchess_domain::trainer::analysis_tree::{self, Score};

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

pub use analysis_tree::{comment_text, moves_of, path_of};

/// A path one move longer: its parent's path and the move.
pub fn child_path(parent: &str, uci: &str) -> String {
    if parent.is_empty() {
        uci.to_string()
    } else {
        format!("{parent} {uci}")
    }
}

/// The position with the moves that led to it, for repetition.
pub use kchess_domain::records::context::analysis_context;

/// "+0.34", "−1.20", "#3", as Lichess prints them; a dash for nothing.
pub fn format_eval(line: Option<&Value>) -> String {
    analysis_tree::format_eval(line.and_then(Score::of_line))
}

/// White's winning chances in −1…1, for the evaluation bar.
pub fn winning_chances(line: Option<&Value>) -> f32 {
    analysis_tree::winning_chances(line.and_then(Score::of_line)) as f32
}

pub use kchess_domain::tree::PvMove;

/// A principal variation in SAN with move numbers, stopping at the first illegal move; `variant`
/// is the Lichess key of the rules it is played by.
pub fn pv_san(fen: &str, pv: &[String], max: usize, variant: &str) -> Vec<PvMove> {
    let variant = if super::standard_rules(variant) {
        ""
    } else {
        variant
    };
    kchess_domain::tree::pv_san(fen, pv, max, variant)
}

/// "1.", "1…", or nothing: the number to print before a move at `ply`.
pub fn move_number(ply: f64, always: bool) -> String {
    analysis_tree::move_number(ply, always)
}

/// The annotation a move is marked with: its glyph ("!?") and name.
pub fn move_glyph(node: &TreeNode) -> Option<(String, String)> {
    let nags: Vec<f64> = node.nags.as_ref()?.iter().map(|&nag| nag as f64).collect();
    let glyph = analysis_tree::move_glyph(&nags)?;
    Some((glyph.glyph.to_string(), glyph.label.to_string()))
}

impl TreeNode {
    /// A tree holding only its start; an unusable FEN starts from the usual position.
    pub fn new(fen: Option<&str>) -> TreeNode {
        let (fen, ply) = kchess_domain::tree::tree_root(fen.unwrap_or(super::STANDARD_FEN));
        TreeNode {
            fen,
            ply: f64::from(ply),
            ..TreeNode::default()
        }
    }

    /// The Lichess key of the variant the tree is played in (its `Variant` header), standard
    /// without one.
    pub fn variant(&self) -> &'static str {
        let name = self
            .headers
            .as_ref()
            .and_then(|headers| headers.get("Variant"))
            .and_then(Value::as_str);
        kchess_domain::variant::Variant::from_pgn_name(name).map_or("standard", |v| v.key())
    }

    /// One bounded PGN document, or `None` when it cannot be imported without loss.
    pub fn from_pgn(text: &str) -> Option<TreeNode> {
        // A tree nests once a ply: it is built and converted on a stack deep enough for it.
        kchess_domain::tree::on_large_stack(|| {
            kchess_domain::tree::tree_from_pgn(text).map(|tree| TreeNode::from_rules(&tree))
        })
        .ok()
        .flatten()
    }

    fn from_rules(node: &kchess_domain::tree::TreeNode) -> TreeNode {
        TreeNode {
            uci: node.uci.clone(),
            san: node.san.clone(),
            fen: node.fen.clone(),
            ply: f64::from(node.ply),
            children: node.children.iter().map(TreeNode::from_rules).collect(),
            headers: node.headers.clone(),
            comments: node.comments.clone(),
            starting_comments: node.starting_comments.clone(),
            nags: node
                .nags
                .as_ref()
                .map(|nags| nags.iter().map(|&nag| i64::from(nag)).collect()),
        }
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
        kchess_domain::tree::tree_to_pgn(&Value::Object(document), self.headers.as_ref())
            .unwrap_or_else(|error| {
                crate::log::write(
                    kchess_contracts::Level::Warn,
                    "analysis",
                    &format!("the tree could not be written as PGN: {error}"),
                );
                String::new()
            })
    }

    /// The moves from the start to `path` alone, as a tree of their own: the comments and
    /// annotations on the way and the variant, without side lines or the game's other headers.
    pub fn line_to(&self, path: &str) -> TreeNode {
        let alone = |node: &TreeNode, children: Vec<TreeNode>| TreeNode {
            uci: node.uci.clone(),
            san: node.san.clone(),
            fen: node.fen.clone(),
            ply: node.ply,
            children,
            headers: None,
            comments: node.comments.clone(),
            starting_comments: node.starting_comments.clone(),
            nags: node.nags.clone(),
        };
        let nodes = self.nodes_along(path);
        let mut line = Vec::new();
        for node in nodes.iter().skip(1).rev() {
            line = vec![alone(node, line)];
        }
        let mut root = alone(self, line);
        root.headers = self
            .headers
            .as_ref()
            .and_then(|headers| headers.get("Variant"))
            .map(|variant| Map::from_iter([("Variant".to_string(), variant.clone())]));
        root
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
        let variant = self.variant();
        let parent = self.node_at(path);
        let child_ucis: Vec<String> = parent
            .children
            .iter()
            .map(|child| child.uci.clone())
            .collect();
        let plan = analysis_tree::add_move_plan(&analysis_tree::MoveInput {
            path,
            uci,
            fen: &parent.fen,
            ply: parent.ply,
            child_ucis: &child_ucis,
            // Standard rules read Chess960 castling too.
            variant: if super::standard_rules(variant) {
                ""
            } else {
                variant
            },
        })?;
        if let Some(child) = plan.child {
            let child = TreeNode {
                uci: child.uci,
                san: child.san,
                fen: child.fen,
                ply: child.ply,
                ..TreeNode::default()
            };
            self.node_at_mut(path).children.push(child);
        }
        Some(plan.child_path)
    }

    /// Follow the first child from `path` to the end of that line.
    pub fn line_end(&self, path: &str) -> String {
        let mut chain = Vec::new();
        let mut node = self.node_at(path);
        while let Some(child) = node.children.first() {
            chain.push(child.uci.clone());
            node = child;
        }
        analysis_tree::line_end(path, &chain)
    }

    /// The index of each node along `path` among its parent's children.
    fn indices_along(&self, path: &str) -> Vec<u64> {
        let nodes = self.nodes_along(path);
        nodes
            .windows(2)
            .map(|pair| {
                pair[0]
                    .children
                    .iter()
                    .position(|child| child.uci == pair[1].uci)
                    .unwrap_or_default() as u64
            })
            .collect()
    }

    /// Where the variation shown at `path` branched off: the position before its last move that
    /// is not a first child. `None` on the main line.
    pub fn fork_before(&self, path: &str) -> Option<String> {
        let moves = moves_of(&self.existing_path(path));
        let last = self
            .indices_along(path)
            .iter()
            .rposition(|&index| index > 0)?;
        Some(path_of(&moves[..last]))
    }

    /// True when every move on `path` is a first child.
    pub fn on_mainline(&self, path: &str) -> bool {
        analysis_tree::on_mainline(&self.indices_along(path))
    }

    /// Remove the move at `path` and everything after it; the parent's path.
    pub fn delete_at(&mut self, path: &str) -> String {
        let plan = analysis_tree::delete_at_plan(path);
        if let Some(uci) = plan.uci {
            self.node_at_mut(&plan.parent_path)
                .children
                .retain(|child| child.uci != uci);
        }
        plan.parent_path
    }

    /// Make the line through `path` the main line at every branch on the way.
    pub fn promote(&mut self, path: &str) {
        let moves = moves_of(path);
        for (depth, from) in analysis_tree::promote_plan(&self.indices_along(path)) {
            let from = from as usize;
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

    /// A move line of `plies` half-moves that shuffles knights back and forth.
    fn shuffle(plies: usize) -> String {
        let shuffle = ["Nf3", "Nf6", "Ng1", "Ng8"];
        let moves: Vec<String> = (0..plies)
            .map(|ply| {
                let number = format!("{}. ", ply / 2 + 1);
                let prefix = if ply % 2 == 0 { number.as_str() } else { "" };
                format!("{prefix}{}", shuffle[ply % 4])
            })
            .collect();
        format!("{} *", moves.join(" "))
    }

    /// The deepest legal tree (1024 plies) read on a 1 MiB thread, as on Windows' main thread:
    /// reading the answer back there overflowed the stack and aborted the app on a dropped PGN.
    /// A 100-move game could not be opened from Game history: its tree was refused at depth 128.
    #[test]
    fn the_deepest_tree_reads_on_a_small_stack() {
        let plies = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let tree = TreeNode::from_pgn(&shuffle(1024)).expect("a tree");
                let (mut node, mut plies) = (&tree, 0);
                while let Some(next) = node.children.first() {
                    node = next;
                    plies += 1;
                }
                // Dropping the tree recurses once a ply too, here on the small stack.
                drop(tree);
                plies
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(plies, 1024);
        let (tree, mut plies) = (TreeNode::from_pgn(&shuffle(200)).expect("a tree"), 0);
        let mut node = &tree;
        while let Some(next) = node.children.first() {
            node = next;
            plies += 1;
        }
        assert_eq!(plies, 200);
    }

    #[test]
    fn a_pgn_keeps_its_annotations() {
        let tree = TreeNode::from_pgn("1. e4!? e5 2. Nf3 $1 *").unwrap();
        let e4 = &tree.children[0];
        assert_eq!(e4.nags, Some(vec![5]));
        assert_eq!(e4.children[0].children[0].nags, Some(vec![1]));
    }

    /// A line is the moves to one move, with what was said on the way, and no side lines.
    #[test]
    fn a_line_is_copied_without_its_side_lines() {
        let tree = TreeNode::from_pgn(
            "[Event \"Club\"]\n[White \"A\"]\n\n1. e4 {Best by test} e5 (1... c5 2. Nf3) 2. Nf3 $1 Nc6 3. Bb5 1-0",
        )
        .unwrap();
        let line = tree.line_to("e2e4 e7e5 g1f3").to_pgn();
        assert!(
            line.contains("1. e4 { Best by test } e5 2. Nf3 $1 *"),
            "{line}"
        );
        assert!(
            !line.contains("c5") && !line.contains("Nc6") && !line.contains("Club"),
            "{line}"
        );
        // From a set-up position, the line says where it starts.
        let fen = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
        let mut tree = TreeNode::new(Some(fen));
        let path = tree.add_move("", "e2e4").unwrap();
        let line = tree.line_to(&path).to_pgn();
        assert!(line.contains(fen) && line.contains("1. e4"), "{line}");
    }

    #[test]
    fn a_variation_is_left_where_it_branched_off() {
        let tree = TreeNode::from_pgn("1. e4 e5 (1... c5 2. Nf3 (2. c3 d5) d6) 2. Nf3 *").unwrap();
        assert_eq!(tree.fork_before("e2e4 e7e5 g1f3"), None);
        assert_eq!(
            tree.fork_before("e2e4 c7c5 g1f3 d7d6").as_deref(),
            Some("e2e4")
        );
        // From a variation inside a variation, out to the inner one's fork first.
        assert_eq!(
            tree.fork_before("e2e4 c7c5 c2c3 d7d5").as_deref(),
            Some("e2e4 c7c5")
        );
        assert_eq!(tree.fork_before("e2e4 c7c5").as_deref(), Some("e2e4"));
    }

    #[test]
    fn moves_branch_and_promote() {
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
