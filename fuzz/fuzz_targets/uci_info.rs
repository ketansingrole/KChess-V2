//! UCI `info` lines: an engine is another program, so its output is outside input.
#![no_main]

use kchess_domain::api;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &str| {
    for white_to_move in [true, false] {
        let _ = api::call(
            "parseInfo",
            &serde_json::json!([line, white_to_move]).to_string(),
        );
    }
});
