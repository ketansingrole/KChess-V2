//! What a study's card shows. The frontends remember results per PGN; the summary itself is
//! worked out here.

use serde::{Deserialize, Serialize};

use crate::tree::summarize_pgn;

/// What a study's card shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudySummary {
    /// The position at the end of the main line.
    pub fen: String,
    /// Half-moves in the main line.
    pub plies: u32,
    /// Moves with a comment, the start's included.
    pub comments: usize,
    pub variations: usize,
    /// "White – Black", '' when neither is known.
    pub players: String,
    pub event: String,
}

/// The card of a study's PGN, or none when the PGN cannot be imported. Read without building
/// the study's tree (`tree::summarize_pgn`); the answer is the tree's.
pub fn summarize_study(pgn: &str) -> Option<StudySummary> {
    let summary = summarize_pgn(pgn)?;
    // A header's value as the card shows it: '' when empty, '?' or missing.
    let known = |name: &str| {
        summary
            .headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty() && *value != "?")
            .unwrap_or("")
    };
    let (white, black) = (known("White"), known("Black"));
    let players = if white.is_empty() && black.is_empty() {
        String::new()
    } else {
        format!(
            "{} \u{2013} {}",
            if white.is_empty() { "?" } else { white },
            if black.is_empty() { "?" } else { black },
        )
    };
    Some(StudySummary {
        fen: summary.fen,
        plies: summary.plies,
        comments: summary.comments,
        variations: summary.variations,
        players,
        event: known("Event").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::summarize_study;
    use crate::tree::{TreeNode, tree_from_pgn};
    use serde_json::{Value, json};

    /// The summary as it was read from the whole tree, which the summary must still answer.
    fn from_tree(pgn: &str) -> Value {
        let Some(root) = tree_from_pgn(pgn) else {
            return Value::Null;
        };
        let known = |name: &str| {
            root.headers
                .as_ref()
                .and_then(|headers| headers.get(name))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && *value != "?")
                .unwrap_or("")
                .to_string()
        };
        let commented = |node: &TreeNode| node.comments.as_ref().is_some_and(|c| !c.is_empty());
        let mut comments = usize::from(commented(&root));
        let mut variations = 0usize;
        let mut stack = vec![&root];
        while let Some(node) = stack.pop() {
            variations += node.children.len().saturating_sub(1);
            for child in &node.children {
                comments += usize::from(commented(child));
                stack.push(child);
            }
        }
        let mut end = &root;
        while let Some(next) = end.children.first() {
            end = next;
        }
        let (white, black) = (known("White"), known("Black"));
        let players = if white.is_empty() && black.is_empty() {
            String::new()
        } else {
            format!(
                "{} \u{2013} {}",
                if white.is_empty() { "?" } else { &white },
                if black.is_empty() { "?" } else { &black },
            )
        };
        json!({
            "fen": end.fen,
            "plies": end.ply.saturating_sub(root.ply),
            "comments": comments,
            "variations": variations,
            "players": players,
            "event": known("Event"),
        })
    }

    /// Read without the tree, a study's summary is the tree's: positions set up and in variants,
    /// comments before and after moves, nested variations, headers left unknown, and documents the
    /// tree refuses (an illegal move deep in a variation, two games, no PGN).
    #[test]
    fn the_summary_is_the_trees() {
        let documents = [
            "",
            "*",
            "1. e4 *",
            "{Before} 1. e4 {After} e5 (1... c5 {Sicilian} 2. Nf3 (2. c3) d6) 2. Nf3 Nc6 3. Bb5 a6 *",
            "[Event \"Club\"]\n[White \"Ann\"]\n[Black \"?\"]\n\n1. d4 d5 2. c4 e6 1-0",
            "[White \"\"]\n[Black \"Bo\"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. O-O *",
            "[FEN \"8/8/8/4k3/8/8/4P3/4K3 w - - 0 40\"]\n[SetUp \"1\"]\n\n40. e4+ Kd6 41. Kd2 *",
            "[FEN \"8/8/8/4k3/8/8/4P3/4K3 b - - 3 40\"]\n[SetUp \"1\"]\n\n*",
            "[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. Nc3 Qa5 4. P@d4 *",
            "[Variant \"Three-check\"]\n\n1. e4 e5 2. Bc4 Nc6 3. Bxf7+ *",
            "1. e4 e5 (1... c5 2. Nf3 (2. Qh5 Qxh5??)) 2. Nf3 *",
            "1. e4 e5 2. Ke3 Ke6 *",
            "1. e4 e5 2. Qxf7 *",
            "1. e4 *\n\n1. d4 *",
            "[Event \"Two chapters\"]\n\n1. e4 $1 {Good} e5 $2 (1... e6 $5) *",
        ];
        let mut read = 0;
        for pgn in documents {
            let tree = from_tree(pgn);
            read += usize::from(!tree.is_null());
            assert_eq!(json!(summarize_study(pgn)), tree, "{pgn:?}");
        }
        assert!(read >= 10, "most of the documents are studies");
    }

    /// A document at the size limit is read, as the tree reads it, and one a unit longer is not.
    #[test]
    fn the_summary_keeps_the_trees_size_limit() {
        // `{…} *`: a comment fills the document to the limit (2,000,000 UTF-16 units).
        let at_limit = format!("{{{}}} *", "a".repeat(2_000_000 - 4));
        assert_eq!(at_limit.len(), 2_000_000);
        let summary = summarize_study(&at_limit).unwrap();
        assert_eq!(summary.comments, 1);
        assert_eq!(json!(summary), from_tree(&at_limit));
        let over = format!("{{{}}} *", "a".repeat(2_000_000 - 3));
        assert_eq!(summarize_study(&over), None);
        assert_eq!(from_tree(&over), Value::Null);
    }
}
