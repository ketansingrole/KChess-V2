//! Games merged into one move tree, for one study chapter (the Database page's "Merge into a
//! study"): every game's moves from their common start, the most played move the main line at
//! each branch (the first game's on a tie) and the others its variations, the most played
//! first. Each game's own variations come along, played by none of them.
//!
//! It is a move tree, not a position graph: a node is the moves that lead to it, so two games
//! that reach one position by different move orders (a transposition) stay on branches of their
//! own, as they were played.
//!
//! The games' comments and NAGs on a move are kept, a comment two games share once; the clock
//! times and evaluations a comment carries (`[%clk]`, `[%emt]`, `[%eval]`) belong to one game's
//! moment and are left out. Each game's line ends with a comment naming it: its players, result,
//! event and year.

use super::{MAX_NODES, TreeNode, continues_line, tree_from_game};
use crate::pgn::Game;
use crate::variant::Variant;

/// The most games one chapter merges.
pub const MAX_GAMES: usize = 200;

/// The commands of a comment that describe one game's moment, not its move.
const MOMENT: [&str; 3] = ["clk", "emt", "eval"];

/// What a merge made.
#[derive(Debug)]
pub struct Merged {
    pub tree: TreeNode,
    /// The games whose moves are in the tree.
    pub merged: usize,
    /// Games left out for starting from another position, or in another variant, than the first
    /// game merged; or whose moves could not be read.
    pub elsewhere: usize,
    /// Games left out for want of room: past `MAX_GAMES`, or moves past what a chapter holds.
    pub no_room: usize,
}

/// What became of a game offered to a `Merger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    Merged,
    Elsewhere,
    NoRoom,
}

/// A move of the tree being built: the node (its children apart), how many games played it on
/// their main line, and its moves, by their place in `Merger::slots`. A move is always placed
/// after the move before it, and the earlier of two siblings was met first.
struct Slot {
    node: TreeNode,
    played: usize,
    children: Vec<usize>,
}

/// Merges games one at a time, so a caller reading them from a database stops once it is full.
pub struct Merger {
    /// The root first.
    slots: Vec<Slot>,
    /// The first game's variant and start, which every other game must share.
    start: Option<(Variant, String)>,
    merged: usize,
    elsewhere: usize,
    no_room: usize,
}

impl Default for Merger {
    fn default() -> Self {
        Merger::new()
    }
}

impl Merger {
    pub fn new() -> Merger {
        Merger {
            slots: Vec::new(),
            start: None,
            merged: 0,
            elsewhere: 0,
            no_room: 0,
        }
    }

    /// Whether `MAX_GAMES` games are in: any other is left out.
    pub fn full(&self) -> bool {
        self.merged >= MAX_GAMES
    }

    /// Counts `count` more games left out for want of room, unread.
    pub fn no_room_for(&mut self, count: usize) {
        self.no_room += count;
    }

    /// Counts a game left out that could not be read at all.
    pub fn unreadable(&mut self) {
        self.elsewhere += 1;
    }

    /// Merges `game`, or says why it is left out.
    pub fn add(&mut self, game: &Game) -> Added {
        let Some(tree) = tree_from_game(game) else {
            self.elsewhere += 1;
            return Added::Elsewhere;
        };
        let start = (tree.variant(), tree.fen.clone());
        match &self.start {
            Some(first) if *first != start => {
                self.elsewhere += 1;
                return Added::Elsewhere;
            }
            Some(_) => {}
            None => {
                self.start = Some(start);
                self.slots.push(Slot {
                    node: root_of(&tree),
                    played: 0,
                    children: Vec::new(),
                });
            }
        }
        if self.full() || self.moves() + self.fresh(&tree) > MAX_NODES {
            self.no_room += 1;
            return Added::NoRoom;
        }
        self.join(&tree, label(game));
        self.merged += 1;
        Added::Merged
    }

    /// The tree, each position's moves the most played first; None when no game was merged.
    pub fn finish(mut self) -> Option<Merged> {
        if self.merged == 0 {
            return None;
        }
        // A move's slot comes after its parent's, so the moves after it are built before it.
        let mut built: Vec<Option<TreeNode>> = (0..self.slots.len()).map(|_| None).collect();
        for at in (0..self.slots.len()).rev() {
            let mut order = std::mem::take(&mut self.slots[at].children);
            // Stable, and siblings are in the order they were met: a tie keeps the first game's.
            order.sort_by_key(|&child| std::cmp::Reverse(self.slots[child].played));
            let mut node = std::mem::take(&mut self.slots[at].node);
            node.children = order
                .into_iter()
                .filter_map(|child| built.get_mut(child).and_then(Option::take))
                .collect();
            // The main line's next move: a note written before it would read as the move
            // before's, so it leads the move's own comments.
            continues_line(&mut node.children);
            if let Some(place) = built.get_mut(at) {
                *place = Some(node);
            }
        }
        let tree = built.into_iter().next().flatten()?;
        Some(Merged {
            tree,
            merged: self.merged,
            elsewhere: self.elsewhere,
            no_room: self.no_room,
        })
    }

    /// The moves in the tree.
    fn moves(&self) -> usize {
        self.slots.len().saturating_sub(1)
    }

    /// The slot of `uci` after the move at `at`, when the tree has it.
    fn child(&self, at: usize, uci: &str) -> Option<usize> {
        self.slots[at]
            .children
            .iter()
            .copied()
            .find(|&child| self.slots[child].node.uci == uci)
    }

    /// How many of `tree`'s moves the tree lacks.
    fn fresh(&self, tree: &TreeNode) -> usize {
        let mut count = 0;
        let mut stack: Vec<(Option<usize>, &TreeNode)> = vec![(Some(0), tree)];
        while let Some((at, node)) = stack.pop() {
            for child in &node.children {
                let found = at.and_then(|at| self.child(at, &child.uci));
                count += usize::from(found.is_none());
                stack.push((found, child));
            }
        }
        count
    }

    /// Adds `tree`'s moves, its main line played once more, and `label` where that line ends.
    fn join(&mut self, tree: &TreeNode, label: String) {
        note(&mut self.slots[0].node.comments, tree.comments.as_deref());
        let mut end = 0;
        // Each move with its slot, and whether it is on the game's main line.
        let mut stack: Vec<(usize, &TreeNode, bool)> = vec![(0, tree, true)];
        while let Some((at, node, main)) = stack.pop() {
            if main {
                if at > 0 {
                    self.slots[at].played += 1;
                }
                if node.children.is_empty() {
                    end = at;
                }
            }
            // Placed in the game's order, so its own variations keep it.
            let mut next = Vec::with_capacity(node.children.len());
            for (index, child) in node.children.iter().enumerate() {
                let slot = match self.child(at, &child.uci) {
                    Some(slot) => slot,
                    None => {
                        let slot = self.slots.len();
                        self.slots.push(Slot {
                            node: TreeNode::leaf(
                                child.uci.clone(),
                                child.san.clone(),
                                child.fen.clone(),
                                child.ply,
                            ),
                            played: 0,
                            children: Vec::new(),
                        });
                        self.slots[at].children.push(slot);
                        slot
                    }
                };
                let into = &mut self.slots[slot].node;
                note(&mut into.comments, child.comments.as_deref());
                note(
                    &mut into.starting_comments,
                    child.starting_comments.as_deref(),
                );
                for &nag in child.nags.iter().flatten() {
                    let nags = into.nags.get_or_insert_with(Vec::new);
                    if !nags.contains(&nag) {
                        nags.push(nag);
                    }
                }
                next.push((slot, child, main && index == 0));
            }
            stack.extend(next.into_iter().rev());
        }
        if !label.is_empty() {
            self.slots[end]
                .node
                .comments
                .get_or_insert_with(Vec::new)
                .push(label);
        }
    }
}

/// The games merged into one tree (`Merger`); None when none could be.
pub fn merge_games<'a>(games: impl IntoIterator<Item = &'a Game>) -> Option<Merged> {
    let mut merger = Merger::new();
    for game in games {
        merger.add(game);
    }
    merger.finish()
}

/// The merged tree's root: the first game's start, in its variant, with nothing of its own.
fn root_of(tree: &TreeNode) -> TreeNode {
    let mut root = TreeNode::leaf(String::new(), String::new(), tree.fen.clone(), tree.ply);
    root.headers = Some(
        tree.headers
            .iter()
            .flatten()
            .filter(|(name, _)| *name == "Variant")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    );
    root
}

/// Adds the comments `from` that `into` lacks, without the commands of one game's moment.
fn note(into: &mut Option<Vec<String>>, from: Option<&[String]>) {
    for comment in from.into_iter().flatten() {
        let Some(comment) = without_moment(comment) else {
            continue;
        };
        let list = into.get_or_insert_with(Vec::new);
        if !list.contains(&comment) {
            list.push(comment);
        }
    }
}

/// `comment` without its clock times and evaluations (`MOMENT`); None when nothing else is left.
/// A comment without them is kept exactly as written.
fn without_moment(comment: &str) -> Option<String> {
    let mut kept = String::new();
    let mut dropped = false;
    let mut rest = comment;
    while let Some(at) = rest.find("[%") {
        let Some(end) = rest[at..].find(']') else {
            break;
        };
        let command = &rest[at + 2..at + end];
        let name = command.split_whitespace().next().unwrap_or_default();
        kept.push_str(&rest[..at]);
        if MOMENT.contains(&name) {
            dropped = true;
            kept.push(' ');
        } else {
            kept.push_str(&rest[at..at + end + 1]);
        }
        rest = &rest[at + end + 1..];
    }
    kept.push_str(rest);
    let kept = if dropped {
        kept.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        kept
    };
    (!kept.trim().is_empty()).then_some(kept)
}

/// What names a game at the end of its line: "Carlsen, Magnus – Gukesh D, 1-0, Tata Steel 2024",
/// without what its headers leave unknown.
pub fn label(game: &Game) -> String {
    let known = |name: &str| {
        game.header(name)
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.starts_with('?'))
    };
    let mut parts = vec![format!(
        "{} – {}",
        known("White").unwrap_or("?"),
        known("Black").unwrap_or("?")
    )];
    if let Some(result) = known("Result").filter(|result| *result != "*") {
        parts.push(result.to_string());
    }
    let year = known("Date")
        .and_then(|date| date.get(..4))
        .filter(|year| year.chars().all(|c| c.is_ascii_digit()));
    match (known("Event"), year) {
        (Some(event), Some(year)) => parts.push(format!("{event} {year}")),
        (Some(event), None) => parts.push(event.to_string()),
        (None, Some(year)) => parts.push(year.to_string()),
        (None, None) => {}
    }
    // A comment ends at `}`.
    parts.join(", ").replace(['{', '}'], "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgn::parse_pgn;
    use crate::tree::tree_from_pgn;

    fn game(white: &str, black: &str, result: &str, moves: &str) -> String {
        format!(
            "[Event \"Open\"]\n[Date \"2024.05.01\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n\
             [Result \"{result}\"]\n\n{moves} {result}\n\n"
        )
    }

    fn merged(pgn: &str) -> Merged {
        merge_games(&parse_pgn(pgn)).expect("games merged")
    }

    /// Each position's moves in the tree's order, as SAN, from `path` (UCI moves).
    fn moves_at(tree: &TreeNode, path: &str) -> Vec<String> {
        tree.node_at(path)
            .children
            .iter()
            .map(|child| child.san.clone())
            .collect()
    }

    fn main_line(tree: &TreeNode) -> Vec<String> {
        let mut line = Vec::new();
        let mut node = tree;
        while let Some(next) = node.children.first() {
            line.push(next.san.clone());
            node = next;
        }
        line
    }

    #[test]
    fn games_sharing_their_first_moves_branch_where_they_part() {
        let pgn = [
            game("Ana", "Ben", "1-0", "1. e4 e5 2. Nf3 Nc6"),
            game("Carl", "Dora", "0-1", "1. e4 e5 2. Bc4"),
        ]
        .concat();
        let merged = merged(&pgn);
        assert_eq!((merged.merged, merged.elsewhere, merged.no_room), (2, 0, 0));
        assert_eq!(moves_at(&merged.tree, ""), ["e4"]);
        assert_eq!(moves_at(&merged.tree, "e2e4"), ["e5"]);
        assert_eq!(moves_at(&merged.tree, "e2e4 e7e5"), ["Nf3", "Bc4"]);
        assert_eq!(main_line(&merged.tree), ["e4", "e5", "Nf3", "Nc6"]);
    }

    #[test]
    fn the_most_played_move_is_the_main_line_and_the_others_follow_by_count() {
        let pgn = [
            game("A", "B", "1-0", "1. d4 d5"),
            game("C", "D", "1-0", "1. e4 c5"),
            game("E", "F", "1-0", "1. e4 e5"),
            game("G", "H", "1-0", "1. c4"),
            game("I", "J", "1-0", "1. c4 e5"),
            game("K", "L", "1-0", "1. e4 e5"),
        ]
        .concat();
        let tree = merged(&pgn).tree;
        // e4 three times, c4 twice, d4 once.
        assert_eq!(moves_at(&tree, ""), ["e4", "c4", "d4"]);
        // After 1. e4: e5 twice before c5 once, though c5 came first.
        assert_eq!(moves_at(&tree, "e2e4"), ["e5", "c5"]);
        // A tie keeps the first game's: after 1. c4 one game ended, one played e5.
        let pgn = [
            game("A", "B", "1-0", "1. d4"),
            game("C", "D", "1-0", "1. e4"),
        ]
        .concat();
        assert_eq!(moves_at(&merged(&pgn).tree, ""), ["d4", "e4"]);
    }

    /// The same position by two move orders stays two branches: the tree is of moves.
    #[test]
    fn a_transposition_stays_on_its_own_branch() {
        let pgn = [
            game("A", "B", "1-0", "1. e4 e5 2. Nf3 Nc6"),
            game("C", "D", "1-0", "1. Nf3 Nc6 2. e4 e5"),
        ]
        .concat();
        let tree = merged(&pgn).tree;
        assert_eq!(moves_at(&tree, ""), ["e4", "Nf3"]);
        let one = tree.node_at("e2e4 e7e5 g1f3 b8c6");
        let other = tree.node_at("g1f3 b8c6 e2e4 e7e5");
        let position = |fen: &str| fen.split(' ').take(4).collect::<Vec<_>>().join(" ");
        assert_eq!(position(&one.fen), position(&other.fen), "one position");
        assert_ne!(one.uci, other.uci, "by two lines");
    }

    #[test]
    fn comments_and_nags_are_kept_and_both_games_notes_on_a_shared_move() {
        let pgn = [
            game(
                "A",
                "B",
                "1-0",
                "1. e4 { Best by test [%clk 0:03:00] } e5 2. Nf3 $1 { [%eval 0.3] }",
            ),
            game(
                "C",
                "D",
                "0-1",
                "1. e4 $1 { A good start } e5 2. Nf3 $1 $14",
            ),
        ]
        .concat();
        let tree = merged(&pgn).tree;
        let e4 = tree.node_at("e2e4");
        assert_eq!(
            e4.comments.as_deref(),
            Some(&["Best by test".to_string(), "A good start".to_string()][..])
        );
        assert_eq!(e4.nags.as_deref(), Some(&[1][..]));
        let nf3 = tree.node_at("e2e4 e7e5 g1f3");
        assert_eq!(nf3.nags.as_deref(), Some(&[1, 14][..]));
        let ends: Vec<&str> = nf3.comments.iter().flatten().map(String::as_str).collect();
        // Its evaluation left out, its own note is none; the two games end there.
        assert_eq!(ends, ["A – B, 1-0, Open 2024", "C – D, 0-1, Open 2024"]);
    }

    #[test]
    fn each_game_s_line_ends_with_a_comment_naming_it() {
        let pgn = [
            "[Event \"Tata Steel\"]\n[Date \"2024.01.20\"]\n[White \"Carlsen, Magnus\"]\n\
             [Black \"Gukesh D\"]\n[Result \"1-0\"]\n\n1. e4 e5 2. Nf3 1-0\n\n"
                .to_string(),
            "[White \"Smith\"]\n[Black \"Jones\"]\n[Result \"*\"]\n\n1. e4 c5 *\n\n".to_string(),
        ]
        .concat();
        let tree = merged(&pgn).tree;
        assert_eq!(
            tree.node_at("e2e4 e7e5 g1f3").comments.as_deref(),
            Some(&["Carlsen, Magnus – Gukesh D, 1-0, Tata Steel 2024".to_string()][..])
        );
        assert_eq!(
            tree.node_at("e2e4 c7c5").comments.as_deref(),
            Some(&["Smith – Jones".to_string()][..])
        );
    }

    #[test]
    fn games_from_another_start_or_variant_are_left_out() {
        let elsewhere = "[FEN \"4k3/8/8/8/8/8/4P3/4K3 w - - 0 1\"]\n[SetUp \"1\"]\n\n1. e4 *\n\n";
        let crazyhouse = "[Variant \"Crazyhouse\"]\n\n1. e4 e5 *\n\n";
        let pgn = [
            game("A", "B", "1-0", "1. e4 e5"),
            elsewhere.to_string(),
            crazyhouse.to_string(),
            game("C", "D", "1-0", "1. d4"),
        ]
        .concat();
        let both = merged(&pgn);
        assert_eq!((both.merged, both.elsewhere), (2, 2));
        assert_eq!(moves_at(&both.tree, ""), ["e4", "d4"]);
        // The first game sets the start: the endgame's games merge among themselves.
        let pgn = [elsewhere.to_string(), game("A", "B", "1-0", "1. e4")].concat();
        let merged = merged(&pgn);
        assert_eq!((merged.merged, merged.elsewhere), (1, 1));
        assert!(merged.tree.to_pgn().unwrap().contains("[FEN \"4k3/"));
    }

    #[test]
    fn a_chapter_holds_at_most_max_games() {
        let pgn = game("A", "B", "1-0", "1. e4").repeat(MAX_GAMES + 3);
        let merged = merged(&pgn);
        assert_eq!((merged.merged, merged.no_room), (MAX_GAMES, 3));
        assert_eq!(
            merged
                .tree
                .node_at("e2e4")
                .comments
                .iter()
                .flatten()
                .count(),
            MAX_GAMES
        );
    }

    #[test]
    fn a_game_past_the_moves_a_chapter_holds_is_left_out() {
        // Each game a knight dance of 601 plies after its own first move: 16 games fit in
        // 10,000 moves, the 17th does not.
        let firsts = [
            "a3", "a4", "b3", "b4", "c3", "c4", "d3", "d4", "e3", "e4", "f3", "f4",
        ];
        let more = ["g3", "g4", "h3", "h4", "Na3", "Nc3", "Nf3", "Nh3"];
        let dance = ["Nf6", "Nc3", "Ng8", "Nb1"].repeat(150).join(" ");
        let pgn: String = firsts
            .iter()
            .chain(&more)
            .map(|first| {
                // After 1. c3, 1. Nc3 or 1. Na3 the queen's knight cannot go to c3: the king's knight dances.
                let dance = match *first {
                    "c3" | "Nc3" | "Na3" => ["Nf6", "Nf3", "Ng8", "Ng1"].repeat(150).join(" "),
                    "Nf3" | "Nh3" => ["Nc6", "Nc3", "Nb8", "Nb1"].repeat(150).join(" "),
                    _ => dance.clone(),
                };
                format!("1. {first} {dance} *\n\n")
            })
            .collect();
        let merged = merged(&pgn);
        assert_eq!(
            (merged.merged, merged.elsewhere, merged.no_room),
            (16, 0, 4)
        );
        let written = merged.tree.to_pgn().unwrap();
        assert!(tree_from_pgn(&written).is_some(), "the chapter opens again");
    }

    #[test]
    fn the_merged_tree_written_as_pgn_reads_back_the_same() {
        let pgn = [
            game(
                "A",
                "B",
                "1-0",
                "{ Opening note } 1. e4 { Best } e5 (1... c5 { Sicilian } 2. Nf3) 2. Nf3 $1",
            ),
            game("C", "D", "1/2-1/2", "1. e4 c5 2. Nc3 $6 { Closed }"),
            game("E", "F", "0-1", "1. e4 c5 2. Nf3 d6"),
            game("G", "H", "1-0", "1. d4 Nf6"),
        ]
        .concat();
        let tree = merged(&pgn).tree;
        // After 1. e4, c5 twice (and once more as A–B's own variation) before e5 once.
        assert_eq!(moves_at(&tree, "e2e4"), ["c5", "e5"]);
        // Nc3 and Nf3 once each: Nf3, A–B's own variation, came first.
        assert_eq!(moves_at(&tree, "e2e4 c7c5"), ["Nf3", "Nc3"]);
        let written = tree.to_pgn().unwrap();
        let mut read = tree_from_pgn(&written).expect("reads back");
        // Read back, the root has the headers PGN writes.
        read.headers = tree.headers.clone();
        assert_eq!(read, tree, "{written}");
        assert_eq!(read.to_pgn().unwrap(), written);
    }

    #[test]
    fn nothing_is_merged_from_no_game() {
        assert!(merge_games(&[]).is_none());
        assert!(merge_games(&parse_pgn("1. e4 e4 *")).is_none());
    }

    #[test]
    fn a_comment_loses_only_one_game_s_moment() {
        assert_eq!(without_moment("[%clk 0:01:00]"), None);
        assert_eq!(
            without_moment("Good [%eval 0.2] [%cal Ge2e4] move").as_deref(),
            Some("Good [%cal Ge2e4] move")
        );
        assert_eq!(
            without_moment(" as  written ").as_deref(),
            Some(" as  written ")
        );
    }
}
