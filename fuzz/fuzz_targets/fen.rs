//! FEN from imports, Lichess and pasted positions, in every variant.
#![no_main]

use kchess_domain::{api, rules};
use libfuzzer_sys::fuzz_target;

const VARIANTS: [&str; 9] = [
    "standard",
    "chess960",
    "crazyhouse",
    "atomic",
    "antichess",
    "kingOfTheHill",
    "threeCheck",
    "horde",
    "racingKings",
];

fuzz_target!(|fen: &str| {
    let _ = rules::parse_fen(fen);
    for variant in VARIANTS {
        let _ = rules::setup_start(variant, fen);
    }
    let _ = api::call("startNode", &serde_json::json!([fen]).to_string());
});
