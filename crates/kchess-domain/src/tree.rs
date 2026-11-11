//! The analysis board's move tree.

use serde::Serialize;
use serde_json::{Map, Value};
use shakmaty::Position;
use shakmaty::variant::VariantPosition;

use crate::js;
use crate::pgn::{Game, PgnNode, make_pgn, parse_pgn};
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
/// illegal one. `variant` is a Lichess key, empty for standard chess.
pub fn pv_san<S: AsRef<str>>(fen: &str, pv: &[S], max: usize, variant: &str) -> Vec<PvMove> {
    let position = match variant {
        "" => rules::standard_from_fen(fen),
        variant => rules::setup_start(variant, fen),
    };
    let Some(mut pos) = position else {
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

fn strings(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|c| {
                c.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "comments must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("comments must be a list".to_string()),
    }
}

fn nags(value: Option<&Value>) -> Result<Option<Vec<u32>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|n| {
                n.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| "nags must be whole numbers".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("nags must be a list".to_string()),
    }
}

/// An analysis tree (`fen`, `children` of `san`, `comments`, `startingComments`, `nags`) as one
/// PGN document with `headers`.
pub fn tree_to_pgn(root: &Value, headers: Option<&Map<String, Value>>) -> Result<String, String> {
    let mut game = Game::empty();
    for (key, value) in headers.into_iter().flatten() {
        let value = value
            .as_str()
            .ok_or_else(|| "header values must be strings".to_string())?;
        game.put_header(key, value.to_string());
    }
    game.comments = strings(root.get("comments"))?;
    let fen = root
        .get("fen")
        .and_then(Value::as_str)
        .ok_or_else(|| "root.fen".to_string())?;
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
    let mut stack: Vec<(&Value, usize)> = vec![(root, 0)];
    while let Some((node, index)) = stack.pop() {
        let children = node
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| "children".to_string())?;
        let mut added = Vec::with_capacity(children.len());
        for child in children {
            let san = child
                .get("san")
                .and_then(Value::as_str)
                .ok_or_else(|| "san".to_string())?;
            let move_node = PgnNode::with(
                san,
                strings(child.get("comments"))?,
                strings(child.get("startingComments"))?,
                nags(child.get("nags"))?,
            );
            added.push((child, game.add_move(index, move_node)));
        }
        // Children are written in order; their subtrees are independent of each other.
        stack.extend(added.into_iter().rev());
    }
    Ok(make_pgn(&game))
}

/// Bytes of stack for work on a deep tree.
const DEEP_STACK: usize = 32 * 1024 * 1024;

/// `work` on a thread with a stack deep enough for the largest tree, whatever thread asked:
/// building, walking, converting and dropping a tree recurse once a ply.
pub fn on_large_stack<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("kchess-deep-tree".into())
            .stack_size(DEEP_STACK)
            .spawn_scoped(scope, work)
            .map_err(|error| format!("could not start a thread: {error}"))?
            .join()
            .map_err(|_| "internal error".to_string())
    })
}

#[cfg(test)]
mod tests {
    /// A variant game's tree is written with its Variant header, and its FEN only when it starts
    /// elsewhere than its variant does, so it reads back in that variant.
    #[test]
    fn a_variant_tree_is_written_in_its_variant() {
        let written = |pgn: &str| {
            let tree = serde_json::to_value(tree_from_pgn(pgn).unwrap()).unwrap();
            tree_to_pgn(&tree, tree["headers"].as_object()).unwrap()
        };
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

    /// A 1024-ply tree, the deepest the rules build, is built, written out and dropped on the
    /// large stack whatever stack asked for it (it overflowed tokio's 2 MiB default in a debug
    /// build).
    #[test]
    fn the_deepest_tree_round_trips_on_a_small_stack() {
        let moves = ["Nf3", "Nf6", "Ng1", "Ng8"].repeat(256).join(" ");
        let pgn = format!("{moves} *");
        let small = std::thread::Builder::new().stack_size(256 * 1024);
        let written = small
            .spawn(move || {
                on_large_stack(|| {
                    let tree = serde_json::to_value(tree_from_pgn(&pgn).unwrap()).unwrap();
                    tree_to_pgn(&tree, None)
                })
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            written.trim_end().ends_with("512. Ng1 Ng8 *"),
            "{}",
            &written[written.len() - 80..]
        );
    }

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
