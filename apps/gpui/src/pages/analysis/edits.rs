//! The analysis board's edit history: Undo and Redo (⌘Z, ⇧⌘Z; Ctrl+Z, Ctrl+Y) over every change
//! to the tree of moves — a move played, a line deleted or promoted, an annotation, a comment.
//!
//! The board records the tree as it settles after each change (`track`), so an edit made anywhere
//! is covered without each one remembering to save itself. Typing one comment is one step. A new
//! document (an import, a study, New analysis) starts a new history.

use crate::game::tree::TreeNode;

/// Steps kept: a long session of edits still undoes far back without holding every tree forever.
const LIMIT: usize = 200;

/// A tree and the move the board showed with it.
#[derive(Clone)]
pub(super) struct Snapshot {
    pub root: TreeNode,
    pub path: String,
}

pub(super) struct Edits {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The tree as last recorded, and where the board is on it.
    current: Snapshot,
    /// The move whose comment is being typed: further typing there joins the same step.
    typing: Option<String>,
}

impl Edits {
    pub fn new(root: &TreeNode, path: &str) -> Edits {
        Edits {
            undo: Vec::new(),
            redo: Vec::new(),
            current: Snapshot {
                root: root.clone(),
                path: path.to_string(),
            },
            typing: None,
        }
    }

    /// Forget every step: another document is on the board.
    pub fn reset(&mut self, root: &TreeNode, path: &str) {
        *self = Edits::new(root, path);
    }

    /// The board settled at `root` and `path`. A changed tree is a new step (or, while one
    /// comment is typed, part of the last one); a move to another position only moves the board.
    /// `typing` names the move whose comment changed.
    pub fn track(&mut self, root: &TreeNode, path: &str, typing: Option<String>) {
        if *root == self.current.root {
            self.current.path = path.to_string();
            if typing.is_none() {
                self.typing = None;
            }
            return;
        }
        let joined = typing.is_some() && typing == self.typing;
        if !joined {
            self.undo.push(self.current.clone());
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.current = Snapshot {
            root: root.clone(),
            path: path.to_string(),
        };
        self.typing = typing;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The tree before the last step, which becomes the current one.
    pub fn undo(&mut self) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo
            .push(std::mem::replace(&mut self.current, previous.clone()));
        self.typing = None;
        Some(previous)
    }

    /// The step last undone, again.
    pub fn redo(&mut self) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo
            .push(std::mem::replace(&mut self.current, next.clone()));
        self.typing = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(moves: &[&str]) -> TreeNode {
        let mut root = TreeNode::new(None);
        let mut path = String::new();
        for uci in moves {
            path = root.add_move(&path, uci).expect("a legal move");
        }
        root
    }

    #[test]
    fn moves_are_undone_and_redone_in_order() {
        let empty = tree(&[]);
        let mut edits = Edits::new(&empty, "");
        assert!(!edits.can_undo());
        let one = tree(&["e2e4"]);
        edits.track(&one, "e2e4", None);
        let two = tree(&["e2e4", "e7e5"]);
        edits.track(&two, "e2e4 e7e5", None);
        // Looking around changes nothing to undo.
        edits.track(&two, "", None);

        let back = edits.undo().expect("a step");
        assert!(back.root == one && back.path == "e2e4");
        let back = edits.undo().expect("a step");
        assert!(back.root == empty && back.path.is_empty());
        assert!(edits.undo().is_none());

        let again = edits.redo().expect("a step");
        assert!(again.root == one);
        // A new edit drops what was undone.
        edits.track(&tree(&["d2d4"]), "d2d4", None);
        assert!(!edits.can_redo());
        assert!(edits.undo().expect("a step").root == one);
    }

    #[test]
    fn typing_one_comment_is_one_step() {
        let start = tree(&["e2e4"]);
        let mut edits = Edits::new(&start, "e2e4");
        for text in ["G", "Go", "Good"] {
            let mut root = start.clone();
            root.node_at_mut("e2e4").comments = Some(vec![text.into()]);
            edits.track(&root, "e2e4", Some("e2e4".into()));
        }
        assert!(edits.undo().expect("a step").root == start);
        assert!(!edits.can_undo());
    }

    #[test]
    fn a_comment_typed_again_later_is_a_step_of_its_own() {
        let start = tree(&["e2e4"]);
        let mut edits = Edits::new(&start, "e2e4");
        let mut first = start.clone();
        first.node_at_mut("e2e4").comments = Some(vec!["Good".into()]);
        edits.track(&first, "e2e4", Some("e2e4".into()));
        edits.track(&first, "", None);
        let mut second = first.clone();
        second.node_at_mut("e2e4").comments = Some(vec!["Good move".into()]);
        edits.track(&second, "e2e4", Some("e2e4".into()));
        assert!(edits.undo().expect("a step").root == first);
        assert!(edits.undo().expect("a step").root == start);
    }

    #[test]
    fn the_history_is_bounded() {
        let mut root = TreeNode::new(None);
        let mut edits = Edits::new(&root, "");
        for step in 0..(LIMIT + 10) {
            root.node_at_mut("").comments = Some(vec![step.to_string()]);
            edits.track(&root, "", None);
        }
        let mut undone = 0;
        while edits.undo().is_some() {
            undone += 1;
        }
        assert_eq!(undone, LIMIT);
    }
}
