//! The analysis board's move tree: every position keeps
//! the moves tried from it, the first child being the main line. A node is addressed by its path,
//! the UCI moves from the root joined with spaces. The rules decide what a path means, whether a
//! move is new, where a node goes and how it is written; this walks the tree and applies them.

use serde_json::Value;

use kchess_domain::trainer::analysis_tree::{self, Score};

pub use kchess_domain::tree::TreeNode;

/// A tree's PGN, as the page shows and saves it.
pub trait TreePgn {
    fn pgn_text(&self) -> String;
}

impl TreePgn for TreeNode {
    /// Its PGN; a header that is not text, which no tree the app builds has, is logged and gives
    /// nothing.
    fn pgn_text(&self) -> String {
        self.to_pgn().unwrap_or_else(|error| {
            crate::log::write(
                kchess_contracts::Level::Warn,
                "analysis",
                &format!("the tree could not be written as PGN: {error}"),
            );
            String::new()
        })
    }
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
    kchess_domain::variant::Variant::from_key(variant)
        .map(|variant| kchess_domain::tree::pv_san(fen, pv, max, variant))
        .unwrap_or_default()
}

/// "1.", "1…", or nothing: the number to print before a move at `ply`.
pub fn move_number(ply: u32, always: bool) -> String {
    analysis_tree::move_number(f64::from(ply), always)
}

/// The annotation a move is marked with: its glyph ("!?") and name.
pub fn move_glyph(node: &TreeNode) -> Option<(String, String)> {
    let nags: Vec<f64> = node.nags.as_ref()?.iter().map(|&nag| nag as f64).collect();
    let glyph = analysis_tree::move_glyph(&nags)?;
    Some((glyph.glyph.to_string(), glyph.label.to_string()))
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
                // Dropped here, on the small stack.
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
        let line = tree.line_to("e2e4 e7e5 g1f3").pgn_text();
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
        let line = tree.line_to(&path).pgn_text();
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
        let again = TreeNode::from_pgn(&tree.pgn_text()).unwrap();
        assert_eq!(again, tree);
        assert_eq!(
            tree.node_at("e2e4").comments.as_deref(),
            Some(&["best by test".to_string()][..])
        );
        assert_eq!(tree.node_at("e2e4").children[1].san, "c5");
    }
}
