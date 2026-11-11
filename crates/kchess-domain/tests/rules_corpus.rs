//! Every rules method answers as it did when the TypeScript suites last ran against it.
//!
//! `fixtures/rules-corpus.jsonl.gz` holds 32k calls the TypeScript test suites made through the
//! Node module before they were removed (`tests/core`, the renderer's unit tests): up to 200 per
//! method, chosen by the hash of their arguments, with the first 16 hex digits of the SHA-256 of
//! each answer (keys sorted, fractions to 9 significant digits), or `e:` and the error. This pins the behaviour those suites checked, including
//! the Rust-only rules their golden digests covered. Regenerate it only for an intended change:
//! `KCHESS_WRITE_CORPUS=1 cargo test -p kchess-domain --test rules_corpus` rewrites each digest.
//!
//! The calls go through the corpus's own dispatcher (`corpus/`), which reads each method's JSON
//! arguments and calls the typed rules. Methods whose rules were deleted, since nothing but the
//! corpus called them, are `RETIRED`: their recorded cases are kept and skipped.

mod corpus;

/// Methods whose rules are gone: nothing but the corpus called them.
const RETIRED: &[&str] = &[
    "EMPTY_BOARD",
    "EMPTY_FEN",
    "FILES",
    "RANKS",
    "START_SETUP",
    "materialBalance",
    "nearestEngineLevel",
    "pgnFromMoves",
    "playMove",
    "puzzleFromDb",
];

use std::io::{Read, Write};
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// An answer with sorted keys and fractions to 9 significant digits: the last bit of `exp`
/// differs between CPUs (as it did for the recorded suites), and nothing else may.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => {
            let x = n.as_f64().unwrap_or_default();
            if x.fract() == 0.0 {
                json!(x)
            } else {
                json!(format!("{x:.8e}").parse::<f64>().unwrap_or(x))
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), canonical(&map[k])))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

fn digest(answer: &Result<String, String>) -> String {
    match answer {
        Ok(text) => {
            let text = serde_json::from_str::<Value>(text)
                .map_or_else(|_| text.clone(), |value| canonical(&value).to_string());
            let hash = Sha256::digest(text.as_bytes());
            hash.iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        Err(message) => format!("e:{message}"),
    }
}

/// The first 200 bytes of a call's arguments for a report, cut at a character boundary (a cut
/// inside "…" once turned a changed answer into a panic that named none).
fn head(args: &str) -> &str {
    let mut end = args.len().min(200);
    while !args.is_char_boundary(end) {
        end -= 1;
    }
    &args[..end]
}

#[test]
fn a_report_cuts_arguments_between_characters() {
    let args = format!("{}…", "a".repeat(199));
    assert_eq!(head(&args), "a".repeat(199));
}

#[test]
fn every_method_answers_as_recorded() {
    // Trees nest some 2,000 levels: parsing, walking and dropping them recurse.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(check_the_corpus)
        .unwrap()
        .join()
        .unwrap();
}

fn check_the_corpus() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules-corpus.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let write = std::env::var("KCHESS_WRITE_CORPUS").as_deref() == Ok("1");
    let mut rewritten = Vec::new();
    let mut differ = Vec::new();
    let mut cases = 0;
    for line in text.lines() {
        let mut case: Value = serde_json::from_str(line).unwrap();
        let (method, args) = (case["m"].as_str().unwrap(), case["a"].as_str().unwrap());
        if RETIRED.contains(&method) {
            rewritten.push(case.to_string());
            continue;
        }
        let actual = digest(&corpus::call(method, args));
        cases += 1;
        if actual != case["d"] {
            if differ.len() < 20 {
                differ.push(format!("{method} {}: {} ≠ {actual}", head(args), case["d"]));
            }
            case["d"] = json!(actual);
        }
        rewritten.push(case.to_string());
    }
    if write {
        let mut out = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::best(),
        );
        out.write_all(format!("{}\n", rewritten.join("\n")).as_bytes())
            .unwrap();
        out.finish().unwrap();
        return;
    }
    assert!(cases > 20_000, "the corpus checks {cases} cases");
    assert!(differ.is_empty(), "answers changed:\n{}", differ.join("\n"));
}
