//! FEN from imports, Lichess and pasted positions, in every variant.
#![no_main]

use kchess_domain::rules;
use kchess_domain::tree::{TreeNode, tree_root};
use kchess_domain::variant::Variant;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|fen: &str| {
    let _ = rules::parse_fen(fen);
    for variant in Variant::ALL {
        let _ = rules::setup_start(variant.key(), fen);
    }
    let _ = tree_root(fen);
    let _ = TreeNode::new(Some(fen));
});
