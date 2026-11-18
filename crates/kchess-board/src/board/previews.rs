//! The small boards a page pictures positions with (`BoardState::preview`), one per key, kept
//! while the page shows them. Each page lets go of the ones it no longer lists (`retain`), so a
//! board scrolled out of a virtual list is still there when it comes back.

use std::collections::HashMap;
use std::hash::Hash;

use gpui_kit::{App, AppContext as _, Entity};

use super::{BoardState, ChessBoard};

pub struct Previews<K = String> {
    boards: HashMap<K, Entity<ChessBoard>>,
}

impl<K> Default for Previews<K> {
    fn default() -> Self {
        Previews {
            boards: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash> Previews<K> {
    /// The board of `key`, made from `state` the first time it is asked for.
    pub fn board(
        &mut self,
        key: K,
        state: impl FnOnce() -> BoardState,
        cx: &mut App,
    ) -> Entity<ChessBoard> {
        self.boards
            .entry(key)
            .or_insert_with(|| cx.new(|_| ChessBoard::new(state())))
            .clone()
    }

    /// The board of `key`, showing `state`: made, or moved on to it.
    pub fn show(&mut self, key: K, state: BoardState, cx: &mut App) -> Entity<ChessBoard> {
        match self.boards.get(&key) {
            Some(board) => {
                board.update(cx, |board, cx| board.set_state(state, cx));
                board.clone()
            }
            None => {
                let board = cx.new(|_| ChessBoard::new(state));
                self.boards.insert(key, board.clone());
                board
            }
        }
    }

    /// Keeps only the boards whose key `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.boards.retain(|key, _| keep(key));
    }

    /// Lets go of the board of `key`; the next ask makes it anew.
    pub fn remove(&mut self, key: &K) {
        self.boards.remove(key);
    }

    pub fn clear(&mut self) {
        self.boards.clear();
    }

    /// The boards kept, by key.
    #[cfg(any(test, feature = "test-support"))]
    pub fn kept(&self) -> impl Iterator<Item = (&K, &Entity<ChessBoard>)> {
        self.boards.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Color;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";

    fn state(fen: &str) -> BoardState {
        BoardState::preview(fen, Default::default(), Color::White, None)
    }

    /// A board is made once per key; one let go is made anew, and `show` moves a kept board on.
    #[gpui_kit::test]
    fn boards_are_kept_by_key_until_let_go(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            let mut previews = Previews::default();
            let first = previews.board("a".to_string(), || state(START), cx);
            let again = previews.board("a".to_string(), || state(E4), cx);
            assert_eq!(first.entity_id(), again.entity_id());
            assert_eq!(again.read(cx).state.fen, START, "made once");

            let shown = previews.show("a".to_string(), state(E4), cx);
            assert_eq!(shown.entity_id(), first.entity_id());
            assert_eq!(shown.read(cx).state.fen, E4, "moved on");

            previews.board("b".to_string(), || state(START), cx);
            previews.retain(|key| key == "b");
            assert_eq!(previews.kept().count(), 1);
            let b = previews.board("b".to_string(), || state(E4), cx);
            assert_eq!(b.read(cx).state.fen, START, "kept");

            previews.remove(&"b".to_string());
            let anew = previews.show("b".to_string(), state(E4), cx);
            assert_ne!(anew.entity_id(), b.entity_id());
            previews.clear();
            assert_eq!(previews.kept().count(), 0);
        });
    }
}
