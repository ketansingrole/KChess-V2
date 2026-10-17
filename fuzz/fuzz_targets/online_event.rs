//! Lichess stream events, as the online game validates each NDJSON line.
#![no_main]

use kchess_domain::api;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &str| {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
        let _ = api::call(
            "validateOnlineEvent",
            &serde_json::json!([event]).to_string(),
        );
    }
});
