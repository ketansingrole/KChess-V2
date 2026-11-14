//! The analysis board's move tree: every position keeps the moves tried from it, the first child
//! being the main line. A node is addressed by its path, the UCI moves from the root joined with
//! spaces. A tree is at most `MAX_DEPTH` plies deep; nothing here recurses once a ply, so any
//! thread's stack holds the deepest one.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use shakmaty::Position;
use shakmaty::variant::VariantPosition;

use crate::js;
use crate::pgn::{Game, PgnNode, make_pgn, parse_pgn};
use crate::rules;
use crate::trainer::analysis_tree::{self, moves_of, path_of};
use crate::variant::Variant;

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// Empty at the root.
    pub uci: String,
    pub san: String,
    pub fen: String,
    /// Half-moves since the game's start: 0 is White's first move still to play.
    pub ply: u32,
    pub children: Vec<TreeNode>,
    /// PGN headers, on the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_comments: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nags: Option<Vec<u32>>,
}

/// A tree is dropped a level at a time, not by recursing once a ply.
impl Drop for TreeNode {
    fn drop(&mut self) {
        let mut below = std::mem::take(&mut self.children);
        while let Some(mut node) = below.pop() {
            below.append(&mut node.children);
        }
    }
}

const MAX_TEXT: usize = 2_000_000;
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 1024;

fn starting_position(game: &Game) -> Option<VariantPosition> {
    let variant = rules::pgn_variant(game.header("Variant"))?;
    match game.header("FEN").filter(|fen| !fen.is_empty()) {
        Some(fen) => rules::setup_position(variant, rules::parse_fen(fen)?),
        None => Some(rules::default_position(variant)),
    }
}

/// The tree's root position: the start re-read under standard rules, falling back to
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

/// One bounded PGN document, or None when it cannot be imported without loss.
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
    let root = TreeNode {
        uci: String::new(),
        san: String::new(),
        fen: rules::make_fen(&root_pos),
        ply: rules::ply_of(&root_pos),
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
    walk(&game, root, start)
}

/// A node being built: its moves still to read, and the position it reached.
struct Building {
    node: TreeNode,
    position: VariantPosition,
    moves: Vec<usize>,
    next: usize,
}

/// The game's moves under `root`, depth first; `None` as soon as the document is found invalid.
fn walk(game: &Game, root: TreeNode, start: VariantPosition) -> Option<TreeNode> {
    let mut count = 0;
    let mut stack = vec![Building {
        node: root,
        position: start,
        moves: game.children(0).collect(),
        next: 0,
    }];
    loop {
        let depth = stack.len();
        let top = stack.last_mut()?;
        let Some(&child) = top.moves.get(top.next) else {
            let done = stack.pop()?;
            match stack.last_mut() {
                Some(parent) => parent.node.children.push(done.node),
                None => return Some(done.node),
            }
            continue;
        };
        top.next += 1;
        count += 1;
        if count > MAX_NODES || depth > MAX_DEPTH {
            return None;
        }
        let data = &game.nodes[child];
        let m = rules::parse_san(&top.position, data.san.as_str())?;
        let mut position = top.position.clone();
        let uci = rules::make_uci(&m);
        let san = rules::san_and_play(&mut position, m);
        let node = TreeNode {
            uci,
            san,
            fen: rules::make_fen(&position),
            ply: top.node.ply + 1,
            children: Vec::new(),
            headers: None,
            comments: data.comments.clone(),
            starting_comments: data.starting_comments.clone(),
            nags: data.nags.clone(),
        };
        stack.push(Building {
            node,
            position,
            moves: game.children(child).collect(),
            next: 0,
        });
    }
}

/// Whether `tree_from_pgn` accepts the document, without building the tree: the same limits
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
    check(game, &start)
}

/// What a study's card shows of a document `tree_from_pgn` accepts, read without building the tree:
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
    if !check(&game, &start) {
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
    for next in game.main_line() {
        let m = rules::parse_san(&position, game.nodes[next].san.as_str())?;
        position.play_unchecked(m);
        plies += 1;
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

fn check(game: &Game, start: &VariantPosition) -> bool {
    let mut count = 0;
    // Each node to check, the position before it and its depth, depth first.
    let mut stack: Vec<(usize, VariantPosition, usize)> = game
        .children(0)
        .map(|child| (child, start.clone(), 1))
        .collect();
    stack.reverse();
    while let Some((node, mut position, depth)) = stack.pop() {
        count += 1;
        if count > MAX_NODES || depth > MAX_DEPTH {
            return false;
        }
        let Some(m) = rules::parse_san(&position, game.nodes[node].san.as_str()) else {
            return false;
        };
        position.play_unchecked(m);
        let children: Vec<usize> = game.children(node).collect();
        stack.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, position.clone(), depth + 1)),
        );
    }
    true
}

/// The root of a new tree from `fen`, read under standard rules: its normalized FEN and ply, the
/// usual start when the FEN is unusable.
pub fn tree_root(fen: &str) -> (String, u32) {
    let pos = rules::standard_from_fen(fen)
        .unwrap_or_else(|| rules::default_position(shakmaty::variant::Variant::Chess));
    (rules::make_fen(&pos), rules::ply_of(&pos))
}

/// One move of a principal variation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PvMove {
    pub san: String,
    pub uci: String,
    /// The SAN with its move number: "12. Nf6", "12… Nf6", or the SAN alone.
    pub label: String,
    /// The position after it.
    pub fen: String,
}

/// "1.", "1…", or nothing: the number printed before the move at `ply`.
fn move_number(ply: u32, always: bool) -> String {
    let number = ply / 2 + 1;
    if ply.is_multiple_of(2) {
        format!("{number}.")
    } else if always {
        format!("{number}…")
    } else {
        String::new()
    }
}

/// A principal variation in SAN with move numbers, at most `max` moves, stopping at the first
/// illegal one.
pub fn pv_san<S: AsRef<str>>(fen: &str, pv: &[S], max: usize, variant: Variant) -> Vec<PvMove> {
    let Some(mut pos) = rules::setup_start(variant, fen) else {
        return Vec::new();
    };
    let mut moves = Vec::new();
    for (ply, uci) in (rules::ply_of(&pos)..).zip(pv.iter().take(max)) {
        let uci = uci.as_ref();
        let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
            break;
        };
        let number = move_number(ply, moves.is_empty());
        let san = rules::san_and_play(&mut pos, m);
        let label = if number.is_empty() {
            san.clone()
        } else {
            format!("{number} {san}")
        };
        moves.push(PvMove {
            san,
            uci: uci.to_string(),
            label,
            fen: rules::make_fen(&pos),
        });
    }
    moves
}

/// The tree as one PGN document with `headers`, which must be strings.
pub fn tree_to_pgn(
    root: &TreeNode,
    headers: Option<&Map<String, Value>>,
) -> Result<String, String> {
    let mut game = Game::empty();
    for (key, value) in headers.into_iter().flatten() {
        let value = value
            .as_str()
            .ok_or_else(|| "header values must be strings".to_string())?;
        game.put_header(key, value.to_string());
    }
    game.comments = root.comments.clone();
    let fen = root.fen.as_str();
    // A variant game (Crazyhouse, Atomic…) keeps its Variant header, and its FEN unless it starts
    // where that variant does.
    let variant = rules::pgn_variant(game.header("Variant"))
        .filter(|variant| *variant != shakmaty::variant::Variant::Chess);
    if let Some(variant) = variant {
        if let Some(pos) =
            rules::parse_fen(fen).and_then(|setup| rules::setup_position(variant, setup))
        {
            let start = rules::make_fen(&pos);
            if start == rules::make_fen(&rules::default_position(variant)) {
                game.remove_header("FEN");
            } else {
                game.put_header("FEN", start);
            }
        }
    } else if fen != rules::INITIAL_FEN
        && let Some(pos) = rules::standard_from_fen(fen)
    {
        // Standard rules, so no Variant header; a FEN unless the usual start.
        game.remove_header("Variant");
        let start = rules::make_fen(&pos);
        if start == rules::INITIAL_FEN {
            game.remove_header("FEN");
        } else {
            game.put_header("FEN", start);
        }
    }
    let mut stack: Vec<(&TreeNode, usize)> = vec![(root, 0)];
    while let Some((node, index)) = stack.pop() {
        let mut added = Vec::with_capacity(node.children.len());
        for child in &node.children {
            let move_node = PgnNode::with(
                &child.san,
                child.comments.clone(),
                child.starting_comments.clone(),
                child.nags.clone(),
            );
            added.push((child, game.add_move(index, move_node)));
        }
        // Children are written in order; their subtrees are independent of each other.
        stack.extend(added.into_iter().rev());
    }
    Ok(make_pgn(&game))
}

impl TreeNode {
    /// A tree holding only its start; an unusable FEN starts from the usual position.
    pub fn new(fen: Option<&str>) -> TreeNode {
        let (fen, ply) = tree_root(fen.unwrap_or(rules::INITIAL_FEN));
        TreeNode::leaf(String::new(), String::new(), fen, ply)
    }

    /// One bounded PGN document (`tree_from_pgn`).
    pub fn from_pgn(text: &str) -> Option<TreeNode> {
        tree_from_pgn(text)
    }

    /// A node with no moves after it, comments or annotations.
    pub fn leaf(uci: String, san: String, fen: String, ply: u32) -> TreeNode {
        TreeNode {
            uci,
            san,
            fen,
            ply,
            children: Vec::new(),
            headers: None,
            comments: None,
            starting_comments: None,
            nags: None,
        }
    }

    /// The variant the tree is played in (its `Variant` header), standard without one.
    pub fn variant(&self) -> crate::variant::Variant {
        let name = self
            .headers
            .as_ref()
            .and_then(|headers| headers.get("Variant"))
            .and_then(Value::as_str);
        crate::variant::Variant::from_pgn_name(name).unwrap_or(crate::variant::Variant::Standard)
    }

    /// The tree as PGN, with its own headers.
    pub fn to_pgn(&self) -> Result<String, String> {
        tree_to_pgn(self, self.headers.as_ref())
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
            ply: f64::from(parent.ply),
            child_ucis: &child_ucis,
            variant,
        })?;
        if let Some(child) = plan.child {
            let child = TreeNode::leaf(child.uci, child.san, child.fen, child.ply as u32);
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
    /// A variant game's tree is written with its Variant header, and its FEN only when it starts
    /// elsewhere than its variant does, so it reads back in that variant.
    #[test]
    fn a_variant_tree_is_written_in_its_variant() {
        let written = |pgn: &str| tree_from_pgn(pgn).unwrap().to_pgn().unwrap();
        let pgn = written("[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. P@e3 *");
        assert!(pgn.contains("[Variant \"Crazyhouse\"]"), "{pgn}");
        assert!(!pgn.contains("[FEN"), "{pgn}");
        assert!(pgn.contains("3. P@e3"), "{pgn}");
        let held = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[Pp] w KQkq - 0 1";
        let pgn = written(&format!(
            "[Variant \"Crazyhouse\"]\n[FEN \"{held}\"]\n\n1. P@e4 *"
        ));
        assert!(pgn.contains(&format!("[FEN \"{held}\"]")), "{pgn}");
        assert!(pgn.contains("[Variant \"Crazyhouse\"]"), "{pgn}");
    }

    /// A 1024-ply tree, the deepest the rules build, is built, written out, copied and dropped on
    /// a small stack (it overflowed tokio's 2 MiB default in a debug build, and Windows' 1 MiB
    /// main thread on a dropped PGN).
    #[test]
    fn the_deepest_tree_round_trips_on_a_small_stack() {
        let moves = ["Nf3", "Nf6", "Ng1", "Ng8"].repeat(256).join(" ");
        let pgn = format!("{moves} *");
        let small = std::thread::Builder::new().stack_size(256 * 1024);
        let written = small
            .spawn(move || {
                let tree = tree_from_pgn(&pgn).unwrap();
                let line = tree.line_to(&tree.line_end(""));
                let written = tree.to_pgn();
                drop((tree, line));
                written
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert!(
            written.trim_end().ends_with("512. Ng1 Ng8 *"),
            "{}",
            &written[written.len() - 80..]
        );
    }

    use super::*;

    /// A tree far deeper than the rules build is dropped a level at a time: dropped node by node
    /// it would overflow a small stack.
    #[test]
    fn a_very_deep_tree_drops_on_a_small_stack() {
        let small = std::thread::Builder::new().stack_size(128 * 1024);
        small
            .spawn(|| {
                let mut tree = TreeNode::default();
                for _ in 0..200_000 {
                    let mut parent = TreeNode::default();
                    parent.children.push(tree);
                    tree = parent;
                }
                drop(tree);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The analysis board's edits through paths: adding, following, finding, promoting and
    /// deleting moves.
    #[test]
    fn a_tree_is_edited_by_path() {
        let mut tree = TreeNode::from_pgn("1. e4 e5 (1... c5 2. Nf3) 2. Nf3 *").unwrap();
        assert_eq!(tree.children[0].children.len(), 2);
        assert_eq!(tree.existing_path("e2e4 e7e5 b1c3 b8c6"), "e2e4 e7e5");
        assert_eq!(tree.existing_path("d2d4"), "");
        assert_eq!(tree.line_end(""), "e2e4 e7e5 g1f3");
        assert_eq!(tree.line_end("e2e4 c7c5"), "e2e4 c7c5 g1f3");

        assert!(tree.on_mainline("e2e4 e7e5 g1f3"));
        assert!(!tree.on_mainline("e2e4 c7c5"));
        assert_eq!(tree.fork_before("e2e4 e7e5 g1f3"), None);
        assert_eq!(tree.fork_before("e2e4 c7c5 g1f3"), Some("e2e4".into()));
        assert_eq!(tree.indices_along("e2e4 c7c5 g1f3"), vec![0, 1, 0]);

        // A move already tried is re-used; a new one is added; an illegal one is refused.
        assert_eq!(tree.add_move("e2e4", "e7e5").as_deref(), Some("e2e4 e7e5"));
        assert_eq!(tree.children[0].children.len(), 2);
        assert_eq!(tree.add_move("e2e4", "d7d5").as_deref(), Some("e2e4 d7d5"));
        assert_eq!(tree.children[0].children[2].san, "d5");
        assert_eq!(tree.add_move("e2e4", "e2e4"), None);

        tree.promote("e2e4 c7c5 g1f3");
        let sans: Vec<&str> = tree.children[0]
            .children
            .iter()
            .map(|child| child.san.as_str())
            .collect();
        assert_eq!(sans, ["c5", "e5", "d5"]);
        assert!(tree.on_mainline("e2e4 c7c5 g1f3"));

        assert_eq!(tree.delete_at("e2e4 e7e5"), "e2e4");
        assert_eq!(tree.existing_path("e2e4 e7e5"), "e2e4");
        assert_eq!(tree.children[0].children.len(), 2);
    }

    /// A tree is played in its Variant header's variant, standard without one.
    #[test]
    fn a_tree_reads_its_variant_from_its_headers() {
        let tree = TreeNode::from_pgn("[Variant \"Atomic\"]\n\n1. e4 *").unwrap();
        assert_eq!(tree.variant(), Variant::Atomic);
        assert_eq!(TreeNode::new(None).variant(), Variant::Standard);
        assert!(TreeNode::from_pgn("1. e5 *").is_none());
    }

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
        let deepest = tree_from_pgn(&shuffle(MAX_DEPTH)).unwrap();
        let mut node = &deepest;
        while let Some(next) = node.children.first() {
            node = next;
        }
        assert_eq!(node.ply, MAX_DEPTH as u32);
        assert!(accepted(&shuffle(MAX_DEPTH)));
        assert!(!accepted(&shuffle(MAX_DEPTH + 1)));
    }

    /// Hostile PGN nests variations inside variations: the depth limit counts every level, so a
    /// document nested deeper than a tree may be is refused, and one at the limit reads.
    #[test]
    fn deeply_nested_variations_are_held_to_the_depth_limit() {
        // Each variation plays the move it replaces again, then one more: a level is a ply.
        let nested = |levels: usize| {
            let moves = ["Nf3", "Nf6", "Ng1", "Ng8"];
            let mut pgn = String::from(moves[0]);
            for level in 0..levels {
                pgn.push_str(&format!(
                    " ( {} {}",
                    moves[level % 4],
                    moves[(level + 1) % 4]
                ));
            }
            pgn.push_str(&" )".repeat(levels));
            pgn.push_str(" *");
            pgn
        };
        let deepest = |pgn: &str| {
            let tree = tree_from_pgn(pgn)?;
            let mut deepest = 0;
            let mut stack = vec![&tree];
            while let Some(node) = stack.pop() {
                deepest = deepest.max(node.ply);
                stack.extend(&node.children);
            }
            Some(deepest)
        };
        assert_eq!(deepest(&nested(MAX_DEPTH - 1)), Some(MAX_DEPTH as u32));
        assert_eq!(deepest(&nested(MAX_DEPTH)), None);
        let hostile = nested(4 * MAX_DEPTH);
        let refused = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || (tree_from_pgn(&hostile).is_none(), valid_pgn(&hostile)))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(refused, (true, false));
    }
}
