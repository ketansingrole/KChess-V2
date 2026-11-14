//! PGN from files, clipboards and Lichess: parsing, the tree it builds and writing it back must
//! never panic or overflow the stack, whatever the text.
#![no_main]

use kchess_domain::{position, tree};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|pgn: &str| {
    let _ = tree::valid_pgn(pgn);
    let _ = position::pgn_games(pgn);
    if let Some(built) = tree::tree_from_pgn(pgn) {
        let _ = built.to_pgn();
    }
});
