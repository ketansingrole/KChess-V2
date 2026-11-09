//! The analysis board's move tree.

use serde::Serialize;
use shakmaty::Position;
use shakmaty::variant::VariantPosition;

use crate::js;
use crate::pgn::{Game, parse_pgn};
use crate::rules;

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub uci: String,
    pub san: String,
    pub fen: String,
    pub ply: u32,
    pub children: Vec<TreeNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_comments: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nags: Option<Vec<u32>>,
}

const MAX_TEXT: usize = 2_000_000;
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 1024;

fn ply_of(pos: &VariantPosition) -> u32 {
    (pos.fullmoves().get() - 1) * 2 + u32::from(!pos.turn().is_white())
}

/// `startingPosition(headers)`.
fn starting_position(game: &Game) -> Option<VariantPosition> {
    let variant = rules::pgn_variant(game.header("Variant"))?;
    match game.header("FEN").filter(|fen| !fen.is_empty()) {
        Some(fen) => rules::setup_position(variant, rules::parse_fen(fen)?),
        None => Some(rules::default_position(variant)),
    }
}

/// The tree's root position: `newTree` re-reads the start under standard rules, falling back to
/// the usual start. A variant's start is its own (a Crazyhouse pocket, Three-check's checks are in
/// its FEN).
fn root_position(start: &VariantPosition) -> VariantPosition {
    if rules::variant_of(start) == shakmaty::variant::Variant::Chess {
        rules::standard_from_fen(&rules::make_fen(start))
            .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess))
    } else {
        start.clone()
    }
}

/// `treeFromPgn`: one bounded PGN document, or None when it cannot be imported without loss.
pub fn tree_from_pgn(text: &str) -> Option<TreeNode> {
    if js::utf16_len(text) > MAX_TEXT {
        return None;
    }
    let mut games = parse_pgn(text);
    if games.len() != 1 {
        return None;
    }
    let game = games.pop()?;
    let start = starting_position(&game)?;
    let root_pos = root_position(&start);
    let mut root = TreeNode {
        uci: String::new(),
        san: String::new(),
        fen: rules::make_fen(&root_pos),
        ply: ply_of(&root_pos),
        children: Vec::new(),
        headers: Some(
            game.headers
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect(),
        ),
        comments: game.comments.clone(),
        starting_comments: None,
        nags: None,
    };
    let mut count = 0;
    walk(&game, 0, &mut root, &start, 0, &mut count).then_some(root)
}

/// Returns false as soon as the document is found invalid (the result is then discarded).
fn walk(
    game: &Game,
    from: usize,
    to: &mut TreeNode,
    pos: &VariantPosition,
    depth: usize,
    count: &mut usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    for child in game.children(from) {
        *count += 1;
        if *count > MAX_NODES {
            return false;
        }
        let data = &game.nodes[child];
        let Some(m) = rules::parse_san(pos, data.san.as_str()) else {
            return false;
        };
        let mut position = pos.clone();
        let uci = rules::make_uci(&m);
        let san = rules::san_and_play(&mut position, m);
        let mut node = TreeNode {
            uci,
            san,
            fen: rules::make_fen(&position),
            ply: to.ply + 1,
            children: Vec::new(),
            headers: None,
            comments: data.comments.clone(),
            starting_comments: data.starting_comments.clone(),
            nags: data.nags.clone(),
        };
        if !walk(game, child, &mut node, &position, depth + 1, count) {
            return false;
        }
        to.children.push(node);
    }
    true
}

/// Whether `treeFromPgn` accepts the document, without building the tree: the same limits
/// and move checks, but no SAN, UCI or FEN text.
pub fn valid_pgn(text: &str) -> bool {
    if js::utf16_len(text) > MAX_TEXT {
        return false;
    }
    let games = parse_pgn(text);
    let [game] = games.as_slice() else {
        return false;
    };
    let Some(start) = starting_position(game) else {
        return false;
    };
    let mut count = 0;
    check(game, 0, &start, 0, &mut count)
}

/// What a study's card shows of a document `treeFromPgn` accepts, read without building the tree:
/// the main line's final position and length, the moves and root with comments, the variations,
/// and the headers. Building the tree writes SAN, UCI and a FEN for every move, and was most of
/// opening the Studies page.
pub struct PgnSummary {
    pub fen: String,
    pub plies: u32,
    pub comments: usize,
    pub variations: usize,
    pub headers: Vec<(String, String)>,
}

pub fn summarize_pgn(text: &str) -> Option<PgnSummary> {
    if js::utf16_len(text) > MAX_TEXT {
        return None;
    }
    let mut games = parse_pgn(text);
    if games.len() != 1 {
        return None;
    }
    let game = games.pop()?;
    let start = starting_position(&game)?;
    let mut count = 0;
    if !check(&game, 0, &start, 0, &mut count) {
        return None;
    }
    let commented =
        |comments: &Option<Vec<String>>| comments.as_ref().is_some_and(|c| !c.is_empty());
    let mut comments = usize::from(commented(&game.comments));
    let mut variations = 0usize;
    let mut stack = vec![0];
    while let Some(node) = stack.pop() {
        let children: Vec<usize> = game.children(node).collect();
        variations += children.len().saturating_sub(1);
        for child in children {
            comments += usize::from(commented(&game.nodes[child].comments));
            stack.push(child);
        }
    }
    // The main line, played from the start as the tree's moves are.
    let mut position = start.clone();
    let mut plies = 0u32;
    let mut node = 0;
    while let Some(next) = game.children(node).next() {
        let m = rules::parse_san(&position, game.nodes[next].san.as_str())?;
        position.play_unchecked(m);
        plies += 1;
        node = next;
    }
    let fen = if plies == 0 {
        rules::make_fen(&root_position(&start))
    } else {
        rules::make_fen(&position)
    };
    Some(PgnSummary {
        fen,
        plies,
        comments,
        variations,
        headers: game.headers,
    })
}

fn check(game: &Game, from: usize, pos: &VariantPosition, depth: usize, count: &mut usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    for child in game.children(from) {
        *count += 1;
        if *count > MAX_NODES {
            return false;
        }
        let Some(m) = rules::parse_san(pos, game.nodes[child].san.as_str()) else {
            return false;
        };
        let mut position = pos.clone();
        position.play_unchecked(m);
        if !check(game, child, &position, depth + 1, count) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crazyhouse_tree_starts_with_pockets_and_keeps_its_drops() {
        let tree =
            tree_from_pgn("[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. Nc3 Qa5 4. P@d4 *")
                .unwrap();
        assert!(tree.fen.contains("[]"), "{}", tree.fen);
        let mut node = &tree;
        while let Some(child) = node.children.first() {
            node = child;
        }
        assert_eq!(node.uci, "P@d4");
        assert_eq!(node.san, "P@d4");
        assert!(node.fen.contains("[p]"), "{}", node.fen);
    }

    #[test]
    fn builds_a_tree_with_variations() {
        let tree =
            tree_from_pgn("1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 3. Bb5 a6 4. O-O *").unwrap();
        assert_eq!(tree.children[0].uci, "e2e4");
        let e4 = &tree.children[0];
        assert_eq!(e4.children.len(), 2);
        assert_eq!(e4.children[1].san, "c5");
        let mut node = &e4.children[0];
        while let Some(next) = node.children.first() {
            node = next;
        }
        assert_eq!(node.san, "O-O");
        assert_eq!(node.uci, "e1h1");
        assert_eq!(node.ply, 7);
    }

    #[test]
    fn rejects_illegal_and_multi_game_documents() {
        assert!(tree_from_pgn("1. e5 *").is_none());
        assert!(tree_from_pgn("1. e4 e5\n\n1. d4 d5").is_none());
    }

    /// A main line of `plies` knight moves, back and forth.
    fn shuffle(plies: usize) -> String {
        let knights = ["Nf3", "Nf6", "Ng1", "Ng8"];
        (0..plies)
            .map(|ply| match ply % 2 {
                0 => format!("{}. {}", ply / 2 + 1, knights[ply % 4]),
                _ => knights[ply % 4].to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Both readers accept a document up to each limit and refuse it one past.
    fn accepted(pgn: &str) -> bool {
        let built = tree_from_pgn(pgn).is_some();
        assert_eq!(valid_pgn(pgn), built, "the two readers agree");
        built
    }

    fn on_a_large_stack(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(test)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a_document_is_read_up_to_its_length_limit() {
        let game = "1. e4 *";
        let padded = |len: usize| format!("{game}{}", " ".repeat(len - game.len()));
        assert!(accepted(&padded(MAX_TEXT)));
        assert!(!accepted(&padded(MAX_TEXT + 1)));
    }

    #[test]
    fn a_tree_is_read_up_to_its_node_limit() {
        // The main line's first move and its alternatives, each one node.
        let with = |nodes: usize| format!("1. e4 {} *", "(1. d4) ".repeat(nodes - 1));
        assert!(accepted(&with(MAX_NODES)));
        assert!(!accepted(&with(MAX_NODES + 1)));
    }

    #[test]
    fn a_tree_is_read_up_to_its_depth_limit() {
        on_a_large_stack(|| {
            let deepest = tree_from_pgn(&shuffle(MAX_DEPTH)).unwrap();
            let mut node = &deepest;
            while let Some(next) = node.children.first() {
                node = next;
            }
            assert_eq!(node.ply, MAX_DEPTH as u32);
            assert!(accepted(&shuffle(MAX_DEPTH)));
            assert!(!accepted(&shuffle(MAX_DEPTH + 1)));
        });
    }
}
