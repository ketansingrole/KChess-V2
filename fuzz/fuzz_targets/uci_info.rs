//! UCI `info` lines: an engine is another program, so its output is outside input.
#![no_main]

use kchess_domain::training::uci_info;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &str| {
    for white_to_move in [true, false] {
        let _ = uci_info::parse_info(line, white_to_move);
    }
});
