//! The core's contract types, as Rust (serde) and as the TypeScript the frontends import.
//!
//! `cargo run -p kchess-contracts --bin export-types` writes `crates/kchess-contracts/ts/generated/`;
//! `-- --check` fails when those files are stale. See `generate.rs` for how modules are rendered.

use std::collections::BTreeMap;

use serde_json::Value;
use ts_rs::Config;

pub mod arity;
pub mod core_api;
pub mod defaults;
pub mod generate;
pub mod library;
pub mod lichess;
pub mod raw;
pub mod settings;
pub mod types;
pub mod variant;

use generate::Module;

/// Wire values of a string-literal union, as JSON strings.
pub fn strs(list: &[&str]) -> Vec<Value> {
    list.iter().map(|s| Value::from(*s)).collect()
}

/// Numeric values, as integers when whole (`1`, not `1.0`).
pub fn numbers(list: &[f64]) -> Vec<Value> {
    list.iter()
        .map(|n| {
            if n.fract() == 0.0 {
                Value::from(*n as i64)
            } else {
                Value::from(*n)
            }
        })
        .collect()
}

/// Every generated module, keyed by file stem (`generated/<stem>.ts`).
pub fn modules() -> Vec<Module> {
    let cfg = Config::new();
    vec![
        Module {
            stem: "types",
            items: types::items(&cfg),
            // `CoreApi` has always been imported from `types`; it is declared in `core`.
            reexports: vec![("variant", vec!["Variant"]), ("core", vec!["CoreApi"])],
        },
        Module {
            stem: "lichessApi",
            items: lichess::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "variant",
            items: variant::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "library",
            items: library::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "core",
            items: core_api::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "settings",
            items: settings::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "defaultSettings",
            items: defaults::items(&cfg),
            reexports: vec![],
        },
        Module {
            stem: "arity",
            items: arity::items(),
            reexports: vec![],
        },
    ]
}

/// The rendered files: `file name → contents`.
pub fn render_all() -> BTreeMap<String, String> {
    generate::render(&modules())
}
