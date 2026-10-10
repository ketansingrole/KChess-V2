//! The analysis board's move tree (`crates/kchess-wasm/js/analysisTree.ts`).

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

/// chessops `startingPosition(headers)`.
fn starting_position(game: &Game) -> Option<VariantPosition> {
    let variant = rules::pgn_variant(game.header("Variant"))?;
    match game.header("FEN").filter(|fen| !fen.is_empty()) {
        Some(fen) => rules::setup_position(variant, rules::parse_fen(fen)?),
        None => Some(rules::default_position(variant)),
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
    // `newTree` re-reads the start under standard rules, falling back to the usual start.
    let start_fen = rules::make_fen(&start);
    let root_pos = rules::standard_from_fen(&start_fen)
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess));
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
}
