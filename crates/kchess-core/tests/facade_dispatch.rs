//! The facade's dispatch covers exactly the methods of `CoreApi` (`kchess_contracts::core_api::methods()`)
//! plus the KChess extras the desktop reads: nothing missing, nothing extra. Three checks pin it:
//! the listed names are the API's names and the extras, every listed name is answered by the
//! facade's `run_facade` match (the source is compared with the list), and calling each name on a
//! live core never reports `Unknown core method`.

use std::collections::BTreeSet;
use std::sync::Arc;

use kchess_contracts::core_api;
use kchess_core::facade::FACADE_METHODS;
use kchess_core::{Config, Core, Host, Level};
use kchess_domain::misc::{Scope, arities};
use serde_json::{Value, json};

/// The KChess methods beyond `CoreApi` that the facade answers (`KChessCore`'s extras).
const EXTRAS: &[&str] = &[
    "settings",
    "desktopSettings",
    "saveDesktopSettings",
    "trustEnginePath",
    "voiceHistoryDocument",
    "suspend",
    "close",
];

/// More arguments than any method takes. A method with a contract rejects this count before it
/// does any work, so probing with it has no effect.
const EXCESS: usize = 64;

/// The message of a name the facade does not answer.
const UNKNOWN: &str = "Unknown core method";

/// Discards the events and host requests; the probe calls need neither.
struct Quiet;

impl Host for Quiet {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, _event: &str, _payload: Value) {}
}

/// Every name the facade answers, from its list.
fn listed() -> Vec<&'static str> {
    FACADE_METHODS.to_vec()
}

#[test]
fn facade_lists_exactly_the_core_api_methods_and_the_extras() {
    let api: Vec<&str> = core_api::methods()
        .iter()
        .map(|method| method.name)
        .collect();
    let api_set: BTreeSet<&str> = api.iter().copied().collect();
    assert_eq!(api_set.len(), api.len(), "CoreApi method names are unique");

    let expected: BTreeSet<&str> = api_set
        .iter()
        .copied()
        .chain(EXTRAS.iter().copied())
        .collect();
    let listed = listed();
    let listed_set: BTreeSet<&str> = listed.iter().copied().collect();
    assert_eq!(
        listed_set.len(),
        listed.len(),
        "FACADE_METHODS lists no name twice"
    );

    let missing: Vec<&&str> = expected.difference(&listed_set).collect();
    let extra: Vec<&&str> = listed_set.difference(&expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "facade lists missing {missing:?} and extra {extra:?}"
    );
}

#[test]
fn facade_arms_answer_exactly_the_listed_methods() {
    // The arms of `run_facade`: the names at the arm indentation, up to its unknown-method arm.
    let source = include_str!("../src/facade.rs");
    let body = source
        .split("async fn run_facade")
        .nth(1)
        .and_then(|rest| rest.split("Unknown core method").next())
        .expect("run_facade has an unknown-method arm");
    let mut arms: BTreeSet<String> = BTreeSet::new();
    for line in body.lines() {
        let Some(pattern) = line.strip_prefix("            \"") else {
            continue;
        };
        let Some((names, _)) = pattern.split_once("=>") else {
            continue;
        };
        for name in names.split('|') {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                arms.insert(name.to_string());
            }
        }
    }
    // `close` is answered before dispatch, in `facade()` itself.
    assert!(
        source.contains("if method == \"close\""),
        "facade() answers close before dispatch"
    );
    let listed: BTreeSet<String> = listed()
        .iter()
        .filter(|name| **name != "close")
        .map(|name| name.to_string())
        .collect();
    let missing: Vec<&String> = listed.difference(&arms).collect();
    let extra: Vec<&String> = arms.difference(&listed).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "run_facade has no arm for {missing:?} and answers unlisted {extra:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_listed_method_is_dispatched() {
    let dir = tempfile::tempdir().expect("profile");
    let core = Core::open(Config::new(dir.path().to_path_buf()), Arc::new(Quiet)).expect("opens");

    // Only names with a core contract, or the extras, are probed: a name with neither would run.
    let contracts: BTreeSet<&str> = arities(Scope::Core)
        .iter()
        .map(|(name, ..)| *name)
        .collect();
    for name in listed() {
        assert!(
            contracts.contains(name) || EXTRAS.contains(&name),
            "{name} has no contract and is not an extra"
        );
    }

    for name in listed() {
        if name == "close" {
            continue;
        }
        let outcome = core.call(name, vec![Value::Null; EXCESS]).await;
        if let Err(cause) = outcome {
            assert!(
                !cause.message.starts_with(UNKNOWN),
                "the facade does not answer {name}: {}",
                cause.message
            );
        }
    }

    let unknown = core.call("noSuchMethod", vec![json!(1)]).await;
    assert!(
        unknown.is_err_and(|cause| cause.message.starts_with(UNKNOWN)),
        "an unlisted name is unknown"
    );
    core.close().await;
}
