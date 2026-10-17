//! PGN from files, clipboards and Lichess: parsing, the tree it builds and writing it back must
//! never panic or overflow the stack, whatever the text.
#![no_main]

use kchess_domain::{api, tree};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|pgn: &str| {
    let _ = tree::valid_pgn(pgn);
    let args = serde_json::json!([pgn]).to_string();
    let _ = api::call("pgnGames", &args);
    if let Ok(built) = api::call("treeFromPgn", &args)
        && built != "null"
    {
        let _ = api::call("treeToPgn", &format!("[{built}]"));
    }
});
