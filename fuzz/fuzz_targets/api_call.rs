//! The rules dispatcher with raw argument text: whatever a host sends, `call` answers with a
//! result or an error, never a panic or a stack overflow (deep nesting is bounded).
#![no_main]

use kchess_domain::api;
use libfuzzer_sys::fuzz_target;

/// Methods that read the most structure from their arguments.
const METHODS: [&str; 12] = [
    "treeFromPgn",
    "treeToPgn",
    "replay",
    "setupReplay",
    "analyseReview",
    "studyDocumentPgn",
    "pvSan",
    "playMove",
    "startNode",
    "sanHistory",
    "pgnFromMoves",
    "pathOf",
];

fuzz_target!(|input: (u8, &str)| {
    let (pick, args) = input;
    let _ = api::call(METHODS[usize::from(pick) % METHODS.len()], args);
});
