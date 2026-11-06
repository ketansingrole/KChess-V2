//! Chess.com archive games, as the sync reads each one from the public API.
#![no_main]

use kchess_domain::chesscom;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = chesscom::game_id(text);
    let _ = chesscom::archive_month(text);
    if let Ok(game) = serde_json::from_str::<serde_json::Value>(text) {
        let _ = chesscom::game_row("hikaru", &game);
    }
});
