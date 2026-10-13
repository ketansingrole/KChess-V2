//! Every `domain("...")` / `validate("...")` literal in the core must name a method
//! `kchess_domain::api::call` answers. The bridge is stringly-typed by design
//! (see RUST_MIGRATION.md: the bridge stays generic), so this test is the
//! compile-time-adjacent pin: a typo fails here, not in production.

use std::collections::BTreeSet;

use kchess_core::engine::domain_client::{DOMAIN_METHODS, is_known_method};

/// `domain("name"` / `validate("name"` literals in these sources.
const SOURCES: &[&str] = &[
    include_str!("../src/engine/mod.rs"),
    include_str!("../src/engine/search.rs"),
    include_str!("../src/engine/analysis.rs"),
    include_str!("../src/engine/review.rs"),
    include_str!("../src/engine/bridge.rs"),
    include_str!("../src/engine/domain_client.rs"),
    include_str!("../src/facade.rs"),
    include_str!("../src/facade/games.rs"),
    include_str!("../src/store/library.rs"),
    include_str!("../src/store/games.rs"),
];

fn literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["domain(\"", "validate(\""] {
        let mut rest = source;
        while let Some(at) = rest.find(marker) {
            let tail = &rest[at + marker.len()..];
            if let Some(end) = tail.find('"') {
                let name = &tail[..end];
                // Skip doc placeholders (`domain("...")`) and format strings.
                if !name.is_empty()
                    && name.chars().any(|c| c.is_ascii_alphanumeric())
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                {
                    out.push(name.to_string());
                }
            }
            rest = tail;
        }
    }
    out
}

#[test]
fn every_domain_literal_is_a_known_rules_method() {
    let mut methods: BTreeSet<String> = BTreeSet::new();
    for source in SOURCES {
        methods.extend(literals(source));
    }
    assert!(
        !methods.is_empty(),
        "expected domain literals in the core sources"
    );
    let mut unknown = Vec::new();
    for method in &methods {
        if !is_known_method(method) {
            unknown.push(method.clone());
        }
    }
    assert!(
        unknown.is_empty(),
        "unknown domain methods called by the core: {unknown:?} (all literals: {methods:?})"
    );
    for method in DOMAIN_METHODS {
        assert!(
            is_known_method(method),
            "typed domain client lists unknown method {method}"
        );
    }
}
