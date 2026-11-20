//! The analysis tree under random edits, as Zed tests its buffers (`random_*` tests): moves added
//! anywhere in the tree, lines deleted and promoted, comments and annotations written, over many
//! seeds, with what must hold checked after every edit. A failure names its seed:
//! `SEED=<n> cargo test -p kchess-domain --test random_tree` replays it.

use kchess_domain::rules;
use kchess_domain::trainer::analysis_tree::{moves_of, path_of};
use kchess_domain::tree::TreeNode;
use kchess_testkit::random::{self, Rng};
use shakmaty::{CastlingMode, Position as _};

/// Seeds a run tries; each makes `EDITS` edits.
const RUNS: u64 = 48;
const EDITS: usize = 40;

/// Every node's path, the root's (empty) first.
fn paths(tree: &TreeNode) -> Vec<String> {
    fn walk(node: &TreeNode, moves: &mut Vec<String>, out: &mut Vec<String>) {
        out.push(path_of(moves));
        for child in &node.children {
            moves.push(child.uci.clone());
            walk(child, moves, out);
            moves.pop();
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut Vec::new(), &mut out);
    out
}

/// The legal moves at `fen`, in UCI as the tree writes them (castling as the king's two squares).
fn legal_ucis(fen: &str) -> Vec<String> {
    let Some(position) = rules::standard_from_fen(fen) else {
        return Vec::new();
    };
    position
        .legal_moves()
        .iter()
        .map(|legal| legal.to_uci(CastlingMode::Standard).to_string())
        .collect()
}

/// What PGN keeps of a tree: every move, position, comment and annotation, in order.
#[derive(Debug, PartialEq)]
struct Kept {
    uci: String,
    fen: String,
    ply: u32,
    comments: Vec<String>,
    nags: Vec<u32>,
    children: Vec<Kept>,
}

fn kept(node: &TreeNode) -> Kept {
    Kept {
        uci: node.uci.clone(),
        fen: node.fen.clone(),
        ply: node.ply,
        comments: node.comments.clone().unwrap_or_default(),
        nags: node.nags.clone().unwrap_or_default(),
        children: node.children.iter().map(kept).collect(),
    }
}

/// What holds of any tree: each move is legal where it is played, follows its parent by one ply
/// and is the only child with that move; and the tree reads back from its PGN unchanged.
fn check_whole(tree: &TreeNode) {
    fn sound(node: &TreeNode) {
        let legal = legal_ucis(&node.fen);
        let mut seen = Vec::new();
        for child in &node.children {
            assert!(legal.contains(&child.uci), "{} is not legal at {}", child.uci, node.fen);
            assert_eq!(child.ply, node.ply + 1, "{} follows its parent by one ply", child.uci);
            assert!(!seen.contains(&&child.uci), "{} is tried twice", child.uci);
            seen.push(&child.uci);
            sound(child);
        }
    }
    sound(tree);
    let pgn = tree.to_pgn().expect("the tree writes as PGN");
    let read = TreeNode::from_pgn(&pgn).expect("its PGN reads back");
    assert_eq!(kept(&read), kept(tree), "the tree reads back from its PGN:\n{pgn}");
}

/// A comment of a few words, as a person types one.
fn comment(rng: &mut Rng) -> String {
    const WORDS: [&str; 8] = ["good", "idea", "but", "the", "knight", "hangs", "later", "!"];
    let count = 1 + rng.below(4);
    let words: Vec<&str> = (0..count)
        .filter_map(|_| rng.pick(&WORDS).copied())
        .collect();
    words.join(" ")
}

/// One random edit at a random node, with what it promises checked at once.
fn edit(tree: &mut TreeNode, rng: &mut Rng) {
    let all = paths(tree);
    let path = rng.pick(&all).cloned().unwrap_or_default();
    match rng.below(10) {
        // Most edits play a move, so the tree grows.
        0..=5 => {
            let Some(uci) = rng.pick(&legal_ucis(&tree.node_at(&path).fen)).cloned() else {
                return;
            };
            let children = tree.node_at(&path).children.len();
            let tried = tree.node_at(&path).children.iter().any(|c| c.uci == uci);
            let added = tree.add_move(&path, &uci).expect("a legal move is added");
            assert_eq!(moves_of(&added).last(), Some(&uci));
            assert_eq!(tree.existing_path(&added), added, "the new move is in the tree");
            let now = tree.node_at(&path).children.len();
            assert_eq!(now, children + usize::from(!tried), "a move tried before is re-used");
            assert_eq!(tree.add_move(&path, &uci), Some(added), "playing it again re-uses it");
        }
        6 if !path.is_empty() => {
            let parent = tree.delete_at(&path);
            let mut moves = moves_of(&path);
            moves.pop();
            assert_eq!(parent, path_of(&moves), "deleting answers the parent");
            assert_ne!(tree.existing_path(&path), path, "the deleted move is gone");
        }
        7 => {
            tree.promote(&path);
            assert!(tree.on_mainline(&path), "a promoted line is the main line");
            assert_eq!(tree.existing_path(&path), path, "promoting keeps every move");
        }
        8 if !path.is_empty() => {
            let text = comment(rng);
            tree.node_at_mut(&path)
                .comments
                .get_or_insert_with(Vec::new)
                .push(text);
        }
        9 if !path.is_empty() => {
            // Move and position annotations: !, ?, !!, ??, !?, ?! and =, ∞.
            let nag = *rng.pick(&[1, 2, 3, 4, 5, 6, 10, 13]).unwrap_or(&1);
            let nags = tree.node_at_mut(&path).nags.get_or_insert_with(Vec::new);
            if !nags.contains(&nag) {
                nags.push(nag);
            }
        }
        _ => {
            let end = tree.line_end(&path);
            assert!(end.starts_with(&path), "{end} continues {path}");
            assert!(tree.node_at(&end).children.is_empty(), "a line's end has no moves after");
        }
    }
}

#[test]
fn random_edits_keep_the_tree_sound_and_its_pgn_whole() {
    random::check(RUNS, |rng| {
        let mut tree = TreeNode::new(None);
        for _ in 0..EDITS {
            edit(&mut tree, rng);
            check_whole(&tree);
        }
    });
}
